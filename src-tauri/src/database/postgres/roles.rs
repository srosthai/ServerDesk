//! PostgreSQL role and privilege DDL planning.
//!
//! PostgreSQL has one role concept: a role with LOGIN is shown as a user.
//! Role names are quoted identifiers. No password or `pg_authid` column is
//! accepted, selected, or rendered.

use crate::database::{
    dialect::{Identifier, IdentifierQuoter},
    errors::DatabaseError,
    roles::{PrivilegeTarget, PrivilegeTargetKind, RoleDdlPlan, RoleKind, ValidatedRoleAction},
};

use super::POSTGRES_DIALECT;

/// Builds a preview plan for one validated PostgreSQL role action.
pub fn preview(action: &ValidatedRoleAction) -> Result<RoleDdlPlan, DatabaseError> {
    match action {
        ValidatedRoleAction::CreateRole(options) => {
            let role = POSTGRES_DIALECT.quote_identifier(&identifier(&options.role.name)?);
            let can_login = options
                .changes
                .can_login
                .unwrap_or(options.kind == RoleKind::User);
            let mut options_sql = vec![if can_login { "LOGIN" } else { "NOLOGIN" }.to_string()];
            push_bool_option(&mut options_sql, "SUPERUSER", options.changes.superuser);
            push_bool_option(
                &mut options_sql,
                "CREATEDB",
                options.changes.create_database,
            );
            push_bool_option(&mut options_sql, "CREATEROLE", options.changes.create_role);
            push_bool_option(&mut options_sql, "INHERIT", options.changes.inherit);

            let mut warnings = Vec::new();
            if can_login {
                warnings.push(
                    "ServerDesk does not set passwords. Configure authentication for this role \
                     outside the app before relying on password login."
                        .to_string(),
                );
            }
            if options.changes.superuser == Some(true) {
                warnings.push(
                    "SUPERUSER bypasses every database privilege check and can read or change \
                     every object."
                        .to_string(),
                );
            }
            if options.changes.create_role == Some(true) {
                warnings.push(
                    "CREATEROLE can create, alter, and grant other roles, which can broaden access \
                     beyond the objects this role owns."
                        .to_string(),
                );
            }
            if options.changes.create_database == Some(true) {
                warnings.push("CREATEDB lets this role create new databases.".to_string());
            }

            Ok(RoleDdlPlan {
                statements: vec![format!("CREATE ROLE {role} {}", options_sql.join(" "))],
                warnings,
                destructive: false,
            })
        }
        ValidatedRoleAction::AlterRole { role, changes } => {
            let role = POSTGRES_DIALECT.quote_identifier(&identifier(&role.name)?);
            let mut options_sql = Vec::new();
            push_bool_option(&mut options_sql, "LOGIN", changes.can_login);
            push_bool_option(&mut options_sql, "SUPERUSER", changes.superuser);
            push_bool_option(&mut options_sql, "CREATEDB", changes.create_database);
            push_bool_option(&mut options_sql, "CREATEROLE", changes.create_role);
            push_bool_option(&mut options_sql, "INHERIT", changes.inherit);
            if options_sql.is_empty() {
                return Err(DatabaseError::validation(
                    "A role change must set at least one attribute.",
                ));
            }
            let mut warnings = vec![
                "Role attribute changes apply to future sessions and privilege checks; existing \
                 sessions may retain cached privileges until they reconnect."
                    .to_string(),
            ];
            if changes.superuser == Some(true) {
                warnings.push(
                    "SUPERUSER bypasses every database privilege check and can read or change \
                     every object."
                        .to_string(),
                );
            }
            if changes.create_role == Some(true) {
                warnings.push(
                    "CREATEROLE can create, alter, and grant other roles, which can broaden access \
                     beyond the objects this role owns."
                        .to_string(),
                );
            }
            Ok(RoleDdlPlan {
                statements: vec![format!("ALTER ROLE {role} {}", options_sql.join(" "))],
                warnings,
                destructive: false,
            })
        }
        ValidatedRoleAction::GrantPrivileges(change) => {
            let role = POSTGRES_DIALECT.quote_identifier(&identifier(&change.grantee.name)?);
            let target = target_sql(&change.target)?;
            let privileges = privilege_list(action);
            let mut warnings = vec![
                "PostgreSQL applies granted privileges to role members according to each role's \
                 INHERIT attribute."
                    .to_string(),
            ];
            if change.grant_option {
                warnings.push(
                    "WITH GRANT OPTION lets the grantee grant the same privileges to other roles."
                        .to_string(),
                );
            }
            Ok(RoleDdlPlan {
                statements: vec![format!(
                    "GRANT {privileges} ON {target} TO {role}{}",
                    if change.grant_option {
                        " WITH GRANT OPTION"
                    } else {
                        ""
                    }
                )],
                warnings,
                destructive: false,
            })
        }
        ValidatedRoleAction::RevokePrivileges(change) => {
            let role = POSTGRES_DIALECT.quote_identifier(&identifier(&change.grantee.name)?);
            let target = target_sql(&change.target)?;
            let privileges = privilege_list(action);
            let prefix = if change.grant_option {
                "GRANT OPTION FOR "
            } else {
                ""
            };
            Ok(RoleDdlPlan {
                statements: vec![format!(
                    "REVOKE {prefix}{privileges} ON {target} FROM {role}"
                )],
                warnings: vec![
                    "Revoking a privilege can immediately break queries, views, or migrations that \
                     rely on it."
                        .to_string(),
                ],
                destructive: true,
            })
        }
        ValidatedRoleAction::GrantRole { role, member } => Ok(RoleDdlPlan {
            statements: vec![format!(
                "GRANT {} TO {}",
                POSTGRES_DIALECT.quote_identifier(&identifier(&role.name)?),
                POSTGRES_DIALECT.quote_identifier(&identifier(&member.name)?)
            )],
            warnings: vec![
                "The member receives the role's privileges according to its INHERIT attribute. \
                 NOINHERIT members must SET ROLE explicitly."
                    .to_string(),
            ],
            destructive: false,
        }),
        ValidatedRoleAction::RevokeRole { role, member } => Ok(RoleDdlPlan {
            statements: vec![format!(
                "REVOKE {} FROM {}",
                POSTGRES_DIALECT.quote_identifier(&identifier(&role.name)?),
                POSTGRES_DIALECT.quote_identifier(&identifier(&member.name)?)
            )],
            warnings: vec![
                "Removing the role can immediately remove privileges from the member's active \
                 sessions."
                    .to_string(),
            ],
            destructive: true,
        }),
    }
}

fn push_bool_option(options: &mut Vec<String>, name: &str, value: Option<bool>) {
    if let Some(value) = value {
        options.push(format!("{}{}", if value { "" } else { "NO" }, name));
    }
}

fn privilege_list(action: &ValidatedRoleAction) -> String {
    match action {
        ValidatedRoleAction::GrantPrivileges(change)
        | ValidatedRoleAction::RevokePrivileges(change) => change
            .privileges
            .iter()
            .map(|privilege| privilege.as_sql())
            .collect::<Vec<_>>()
            .join(", "),
        _ => String::new(),
    }
}

fn target_sql(target: &PrivilegeTarget) -> Result<String, DatabaseError> {
    match target.kind {
        PrivilegeTargetKind::Database => {
            let database = target.database.as_ref().ok_or_else(|| {
                DatabaseError::validation("A PostgreSQL database grant requires a database name.")
            })?;
            Ok(format!(
                "DATABASE {}",
                POSTGRES_DIALECT.quote_identifier(database)
            ))
        }
        PrivilegeTargetKind::Schema => {
            let schema = target.schema.as_ref().ok_or_else(|| {
                DatabaseError::validation("A PostgreSQL schema grant requires a schema name.")
            })?;
            Ok(format!(
                "SCHEMA {}",
                POSTGRES_DIALECT.quote_identifier(schema)
            ))
        }
        PrivilegeTargetKind::Table => {
            let schema = target.schema.as_ref().ok_or_else(|| {
                DatabaseError::validation("A PostgreSQL table grant requires a schema name.")
            })?;
            let object = target.object.as_ref().ok_or_else(|| {
                DatabaseError::validation("A PostgreSQL table grant requires a table name.")
            })?;
            Ok(format!(
                "TABLE {}.{}",
                POSTGRES_DIALECT.quote_identifier(schema),
                POSTGRES_DIALECT.quote_identifier(object)
            ))
        }
        PrivilegeTargetKind::Sequence => {
            let schema = target.schema.as_ref().ok_or_else(|| {
                DatabaseError::validation("A PostgreSQL sequence grant requires a schema name.")
            })?;
            let object = target.object.as_ref().ok_or_else(|| {
                DatabaseError::validation("A PostgreSQL sequence grant requires a sequence name.")
            })?;
            Ok(format!(
                "SEQUENCE {}.{}",
                POSTGRES_DIALECT.quote_identifier(schema),
                POSTGRES_DIALECT.quote_identifier(object)
            ))
        }
        PrivilegeTargetKind::Global => Err(DatabaseError::capability(
            "PostgreSQL has no global object target.",
        )),
    }
}

fn identifier(value: &str) -> Result<Identifier, DatabaseError> {
    Identifier::new(value).map_err(|error| {
        DatabaseError::validation("The role name is not valid.").with_details(error.message)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::{
        RoleRef,
        roles::{Privilege, PrivilegeTargetRequest, RoleActionRequest, RoleRefRequest},
    };

    #[test]
    fn grant_preview_quotes_role_and_table_target() {
        let action = RoleActionRequest::GrantPrivileges {
            grantee: RoleRefRequest {
                name: "reporting".to_string(),
                host: None,
            },
            privileges: vec![Privilege::Select, Privilege::Update],
            target: PrivilegeTargetRequest {
                kind: PrivilegeTargetKind::Table,
                database: None,
                schema: Some("public".to_string()),
                object: Some("orders".to_string()),
            },
            with_grant_option: true,
        }
        .validate(crate::storage::DatabaseEngine::Postgresql)
        .expect("valid grant");
        let plan = preview(&action).expect("grant preview");
        assert_eq!(
            plan.statements,
            vec![
                "GRANT SELECT, UPDATE ON TABLE \"public\".\"orders\" TO \"reporting\" WITH GRANT OPTION"
            ]
        );
        assert!(!plan.destructive);
    }

    #[test]
    fn create_user_emits_login_and_explicit_attributes() {
        let action = RoleActionRequest::CreateRole {
            name: "deploy".to_string(),
            host: None,
            role_kind: RoleKind::User,
            can_login: Some(true),
            superuser: Some(false),
            create_database: Some(true),
            create_role: None,
            inherit: Some(false),
        }
        .validate(crate::storage::DatabaseEngine::Postgresql)
        .expect("valid create");
        let plan = preview(&action).expect("create preview");
        assert_eq!(
            plan.statements,
            vec!["CREATE ROLE \"deploy\" LOGIN NOSUPERUSER CREATEDB NOINHERIT"]
        );
        assert!(
            plan.warnings
                .iter()
                .any(|warning| warning.contains("does not set passwords"))
        );
        assert!(
            plan.warnings
                .iter()
                .any(|warning| warning.contains("CREATEDB"))
        );
    }

    #[test]
    fn alter_role_rejects_an_empty_change_set() {
        let action = RoleActionRequest::AlterRole {
            name: "deploy".to_string(),
            host: None,
            can_login: None,
            superuser: None,
            create_database: None,
            create_role: None,
            inherit: None,
        };
        assert!(
            action
                .validate(crate::storage::DatabaseEngine::Postgresql)
                .is_err()
        );
    }

    #[test]
    fn role_membership_preview_quotes_both_roles() {
        let action = ValidatedRoleAction::GrantRole {
            role: RoleRef {
                name: "reporting_reader".to_string(),
                host: None,
            },
            member: RoleRef {
                name: "reporting".to_string(),
                host: None,
            },
        };
        let plan = preview(&action).expect("role grant");
        assert_eq!(
            plan.statements,
            vec!["GRANT \"reporting_reader\" TO \"reporting\""]
        );
    }
}
