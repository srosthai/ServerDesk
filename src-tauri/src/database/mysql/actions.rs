//! MySQL database lifecycle DDL.
//!
//! MySQL can create and drop databases. It has no owner/template concept and
//! no atomic, safe rename operation, so rename is rejected as a capability
//! error with an explanation before any statement is generated.

use crate::database::{
    actions::{DatabaseDdlPreview, ValidatedDatabaseAction},
    dialect::{Identifier, IdentifierQuoter, SqlDialect},
    errors::DatabaseError,
};

use super::MYSQL_DIALECT;

/// Builds a preview for one validated MySQL database action.
pub fn preview(action: &ValidatedDatabaseAction) -> Result<DatabaseDdlPreview, DatabaseError> {
    match action {
        ValidatedDatabaseAction::Create(options) => {
            if options.owner.is_some() {
                return Err(DatabaseError::capability(
                    "MySQL does not support a database owner option. The configured account owns \
                     the new schema.",
                ));
            }
            if options.template.is_some() {
                return Err(DatabaseError::capability(
                    "MySQL does not support database templates.",
                ));
            }

            let mut statement = format!(
                "CREATE DATABASE {}",
                MYSQL_DIALECT.quote_identifier(&options.name)
            );
            if let Some(encoding) = options.encoding.as_deref() {
                statement.push_str(&format!(
                    " CHARACTER SET {}",
                    MYSQL_DIALECT.quote_identifier(&option_identifier(encoding)?)
                ));
            }
            if let Some(collation) = options.collation.as_deref() {
                statement.push_str(&format!(
                    " COLLATE {}",
                    MYSQL_DIALECT.quote_identifier(&option_identifier(collation)?)
                ));
            }

            let mut warnings = vec![
                "MySQL applies the server default character set and collation when none is \
                 selected."
                    .to_string(),
            ];
            if options.collation.is_some() && options.encoding.is_none() {
                warnings.push(
                    "The collation selects its character set; setting both keeps the intent \
                     explicit."
                        .to_string(),
                );
            }

            Ok(DatabaseDdlPreview::new(
                MYSQL_DIALECT.engine(),
                action,
                vec![statement],
                warnings,
            ))
        }
        ValidatedDatabaseAction::Rename { .. } => Err(DatabaseError::capability(
            "MySQL does not support renaming a database safely. Create a new database and migrate \
             its data instead.",
        )),
        ValidatedDatabaseAction::Drop { name } => Ok(DatabaseDdlPreview::new(
            MYSQL_DIALECT.engine(),
            action,
            vec![format!(
                "DROP DATABASE {}",
                MYSQL_DIALECT.quote_identifier(name)
            )],
            vec![
                "Dropping a database permanently removes every table, view, and row inside it."
                    .to_string(),
                "MySQL cannot drop the database used by the current connection; switch the active \
                 database first."
                    .to_string(),
            ],
        )),
        ValidatedDatabaseAction::SetActive { name } => Ok(DatabaseDdlPreview::new(
            MYSQL_DIALECT.engine(),
            action,
            Vec::new(),
            vec![format!(
                "ServerDesk reopens the MySQL session against {} to change the active schema. \
                 The active schema controls unqualified table names and SQL context.",
                name
            )],
        )),
    }
}

fn option_identifier(value: &str) -> Result<Identifier, DatabaseError> {
    Identifier::new(value).map_err(|error| {
        DatabaseError::validation("The MySQL character set or collation is not valid.")
            .with_details(error.message)
    })
}

#[cfg(test)]
mod tests {
    use crate::database::actions::{DatabaseActionRequest, ValidatedCreateDatabase};

    use super::*;

    fn options() -> ValidatedCreateDatabase {
        let ValidatedDatabaseAction::Create(options) = DatabaseActionRequest::Create {
            name: "app_next".to_string(),
            encoding: Some("utf8mb4".to_string()),
            collation: Some("utf8mb4_unicode_ci".to_string()),
            owner: None,
            template: None,
        }
        .validate()
        .expect("valid create") else {
            unreachable!("create request");
        };
        options
    }

    #[test]
    fn create_preview_quotes_identifiers_and_reports_mysql_options() {
        let preview = preview(&ValidatedDatabaseAction::Create(options())).expect("create preview");

        assert_eq!(preview.action.as_str(), "create");
        assert_eq!(preview.database, "app_next");
        assert_eq!(
            preview.statements,
            ["CREATE DATABASE `app_next` CHARACTER SET `utf8mb4` COLLATE `utf8mb4_unicode_ci`"]
        );
        assert!(!preview.destructive);
        assert_eq!(preview.engine, crate::storage::DatabaseEngine::Mysql);
    }

    #[test]
    fn owner_and_template_are_typed_capability_errors_on_mysql() {
        let mut owner_options = options();
        owner_options.owner = Some(Identifier::new("deploy").expect("owner"));
        let error = preview(&ValidatedDatabaseAction::Create(owner_options))
            .expect_err("owner is unsupported");
        assert_eq!(error.kind, crate::database::DatabaseErrorKind::Capability);
        assert!(error.message.contains("owner"));

        let mut template_options = options();
        template_options.template = Some(Identifier::new("template0").expect("template"));
        let error = preview(&ValidatedDatabaseAction::Create(template_options))
            .expect_err("template is unsupported");
        assert_eq!(error.kind, crate::database::DatabaseErrorKind::Capability);
        assert!(error.message.contains("template"));
    }

    #[test]
    fn rename_is_rejected_with_an_explanation() {
        let action = ValidatedDatabaseAction::Rename {
            name: Identifier::new("app").expect("name"),
            new_name: Identifier::new("app_next").expect("new name"),
        };
        let error = preview(&action).expect_err("MySQL rename is unsupported");
        assert_eq!(error.kind, crate::database::DatabaseErrorKind::Capability);
        assert!(error.message.contains("does not support renaming"));
    }

    #[test]
    fn drop_and_set_active_generate_expected_previews() {
        let drop = preview(&ValidatedDatabaseAction::Drop {
            name: Identifier::new("odd`name").expect("name"),
        })
        .expect("drop preview");
        assert_eq!(drop.statements, ["DROP DATABASE `odd``name`"]);
        assert!(drop.destructive);
        assert!(drop.requires_confirmation);

        let active = preview(&ValidatedDatabaseAction::SetActive {
            name: Identifier::new("app").expect("name"),
        })
        .expect("set-active preview");
        assert!(active.statements.is_empty());
        assert!(active.warnings[0].contains("reopens the MySQL session"));
    }
}
