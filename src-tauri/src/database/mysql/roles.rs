//! MySQL user, role, and grant DDL planning.
//!
//! Account names use MySQL's `'user'@'host'` literal form. The validated
//! request restricts both parts to characters that cannot terminate the
//! literal, so no user-controlled SQL can escape the account or target.
//! Passwords are never accepted, generated, selected, or rendered.

use crate::database::{
    dialect::IdentifierQuoter,
    errors::DatabaseError,
    roles::{PrivilegeTarget, PrivilegeTargetKind, RoleDdlPlan, RoleKind, ValidatedRoleAction},
};

use super::MYSQL_DIALECT;

/// Builds a preview plan for one validated MySQL role action.
pub fn preview(action: &ValidatedRoleAction) -> Result<RoleDdlPlan, DatabaseError> {
    match action {
        ValidatedRoleAction::CreateRole(options) => {
            let account = account_literal(&options.role.name, options.role.host.as_deref())?;
            let mut warnings = Vec::new();
            let statement = match options.kind {
                RoleKind::User => {
                    warnings.push(
                        "ServerDesk does not set passwords. The account is created locked; \
                         configure authentication outside the app, then unlock it here."
                            .to_string(),
                    );
                    format!("CREATE USER {account} ACCOUNT LOCK")
                }
                RoleKind::Role => {
                    warnings.push(
                        "MySQL roles cannot authenticate. Grant the role to a user and activate it \
                         for the session that needs its privileges."
                            .to_string(),
                    );
                    format!("CREATE ROLE {account}")
                }
            };
            Ok(RoleDdlPlan {
                statements: vec![statement],
                warnings,
                destructive: false,
            })
        }
        ValidatedRoleAction::AlterRole { role, changes } => {
            let account = account_literal(&role.name, role.host.as_deref())?;
            let Some(can_login) = changes.can_login else {
                return Err(DatabaseError::validation(
                    "MySQL role changes currently support only the login lock state.",
                ));
            };
            Ok(RoleDdlPlan {
                statements: vec![format!(
                    "ALTER USER {account} {}",
                    if can_login {
                        "ACCOUNT UNLOCK"
                    } else {
                        "ACCOUNT LOCK"
                    }
                )],
                warnings: vec![
                    "Changing the account lock state affects new sessions; existing sessions may \
                     continue until they disconnect."
                        .to_string(),
                ],
                destructive: false,
            })
        }
        ValidatedRoleAction::GrantPrivileges(change) => {
            let account = account_literal(&change.grantee.name, change.grantee.host.as_deref())?;
            let target = target_sql(&change.target)?;
            let privileges = privilege_list(action);
            let mut warnings = vec![
                "MySQL roles and privileges take effect for a session only after the role is \
                 activated or listed as a default role."
                    .to_string(),
            ];
            if change.grant_option {
                warnings.push(
                    "WITH GRANT OPTION lets the grantee grant the same privileges to other \
                     accounts."
                        .to_string(),
                );
            }
            Ok(RoleDdlPlan {
                statements: vec![format!(
                    "GRANT {privileges} ON {target} TO {account}{}",
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
            let account = account_literal(&change.grantee.name, change.grantee.host.as_deref())?;
            let target = target_sql(&change.target)?;
            let privileges = privilege_list(action);
            let prefix = if change.grant_option {
                "GRANT OPTION FOR "
            } else {
                ""
            };
            Ok(RoleDdlPlan {
                statements: vec![format!(
                    "REVOKE {prefix}{privileges} ON {target} FROM {account}"
                )],
                warnings: vec![
                    "Revoking a privilege can immediately break application queries or migrations \
                     that rely on it."
                        .to_string(),
                ],
                destructive: true,
            })
        }
        ValidatedRoleAction::GrantRole { role, member } => Ok(RoleDdlPlan {
            statements: vec![format!(
                "GRANT {} TO {}",
                account_literal(&role.name, role.host.as_deref())?,
                account_literal(&member.name, member.host.as_deref())?
            )],
            warnings: vec![
                "The member must activate the role with SET ROLE or list it as a default role \
                 before its privileges apply."
                    .to_string(),
            ],
            destructive: false,
        }),
        ValidatedRoleAction::RevokeRole { role, member } => Ok(RoleDdlPlan {
            statements: vec![format!(
                "REVOKE {} FROM {}",
                account_literal(&role.name, role.host.as_deref())?,
                account_literal(&member.name, member.host.as_deref())?
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
        PrivilegeTargetKind::Global => Ok("*.*".to_string()),
        PrivilegeTargetKind::Database => {
            let database = target.database.as_ref().ok_or_else(|| {
                DatabaseError::validation("A MySQL database grant requires a database name.")
            })?;
            Ok(format!("{}.*", MYSQL_DIALECT.quote_identifier(database)))
        }
        PrivilegeTargetKind::Table => {
            let database = target.database.as_ref().ok_or_else(|| {
                DatabaseError::validation("A MySQL table grant requires a database name.")
            })?;
            let object = target.object.as_ref().ok_or_else(|| {
                DatabaseError::validation("A MySQL table grant requires a table name.")
            })?;
            Ok(format!(
                "{}.{}",
                MYSQL_DIALECT.quote_identifier(database),
                MYSQL_DIALECT.quote_identifier(object)
            ))
        }
        PrivilegeTargetKind::Schema | PrivilegeTargetKind::Sequence => {
            Err(DatabaseError::capability(format!(
                "MySQL does not support {} privilege targets.",
                target.kind.as_str()
            )))
        }
    }
}

fn account_literal(name: &str, host: Option<&str>) -> Result<String, DatabaseError> {
    if name.contains('\'') || host.is_some_and(|host| host.contains('\'')) {
        return Err(DatabaseError::validation(
            "An account name or host cannot contain a single quote.",
        ));
    }
    let host = host.unwrap_or("%");
    Ok(format!("'{name}'@'{host}'"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::{
        dialect::Identifier,
        roles::{
            Privilege, PrivilegeTargetRequest, RoleActionRequest, RoleRefRequest,
            ValidatedPrivilegeChange,
        },
    };

    fn grant() -> ValidatedRoleAction {
        RoleActionRequest::GrantPrivileges {
            grantee: RoleRefRequest {
                name: "reporting".to_string(),
                host: Some("%".to_string()),
            },
            privileges: vec![Privilege::Select, Privilege::Update],
            target: PrivilegeTargetRequest {
                kind: PrivilegeTargetKind::Table,
                database: Some("app".to_string()),
                schema: None,
                object: Some("orders".to_string()),
            },
            with_grant_option: true,
        }
        .validate(crate::storage::DatabaseEngine::Mysql)
        .expect("valid grant")
    }

    #[test]
    fn grant_preview_quotes_the_exact_account_and_target() {
        let plan = preview(&grant()).expect("grant preview");
        assert_eq!(
            plan.statements,
            vec!["GRANT SELECT, UPDATE ON `app`.`orders` TO 'reporting'@'%' WITH GRANT OPTION"]
        );
        assert!(
            plan.warnings
                .iter()
                .any(|warning| warning.contains("GRANT OPTION"))
        );
        assert!(!plan.destructive);
    }

    #[test]
    fn revoke_preview_is_destructive_and_exact() {
        let action = RoleActionRequest::RevokePrivileges {
            grantee: RoleRefRequest {
                name: "reporting".to_string(),
                host: Some("10.0.0.%".to_string()),
            },
            privileges: vec![Privilege::Select],
            target: PrivilegeTargetRequest {
                kind: PrivilegeTargetKind::Database,
                database: Some("app".to_string()),
                schema: None,
                object: None,
            },
            revoke_grant_option: false,
        }
        .validate(crate::storage::DatabaseEngine::Mysql)
        .expect("valid revoke");
        let plan = preview(&action).expect("revoke preview");
        assert_eq!(
            plan.statements,
            vec!["REVOKE SELECT ON `app`.* FROM 'reporting'@'10.0.0.%'"]
        );
        assert!(plan.destructive);
    }

    #[test]
    fn create_user_is_locked_or_warned_about_missing_authentication() {
        let action = RoleActionRequest::CreateRole {
            name: "deploy".to_string(),
            host: Some("localhost".to_string()),
            role_kind: RoleKind::User,
            can_login: Some(false),
            superuser: None,
            create_database: None,
            create_role: None,
            inherit: None,
        }
        .validate(crate::storage::DatabaseEngine::Mysql)
        .expect("valid create");
        let plan = preview(&action).expect("create preview");
        assert_eq!(
            plan.statements,
            vec!["CREATE USER 'deploy'@'localhost' ACCOUNT LOCK"]
        );
        assert!(
            plan.warnings
                .iter()
                .any(|warning| warning.contains("does not set passwords"))
        );
    }

    #[test]
    fn role_membership_preview_uses_account_literals() {
        let action = ValidatedRoleAction::GrantRole {
            role: crate::database::RoleRef {
                name: "reporting_reader".to_string(),
                host: Some("%".to_string()),
            },
            member: crate::database::RoleRef {
                name: "reporting".to_string(),
                host: Some("%".to_string()),
            },
        };
        let plan = preview(&action).expect("role grant");
        assert_eq!(
            plan.statements,
            vec!["GRANT 'reporting_reader'@'%' TO 'reporting'@'%'"]
        );
    }

    #[test]
    fn grant_plan_rejects_a_schema_target_before_sql_is_built() {
        let change = ValidatedPrivilegeChange {
            grantee: crate::database::RoleRef {
                name: "reporting".to_string(),
                host: Some("%".to_string()),
            },
            privileges: vec![Privilege::Select],
            target: PrivilegeTarget {
                kind: PrivilegeTargetKind::Schema,
                database: None,
                schema: Some(Identifier::new("app").expect("schema")),
                object: None,
            },
            grant_option: false,
        };
        assert!(preview(&ValidatedRoleAction::GrantPrivileges(change)).is_err());
    }
}
