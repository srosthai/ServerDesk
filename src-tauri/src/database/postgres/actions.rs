//! PostgreSQL database lifecycle DDL.
//!
//! PostgreSQL supports creating a database with an owner, template, encoding,
//! and collation, renaming an inactive catalog, and dropping an inactive
//! catalog. The current database cannot be renamed or dropped by one session.

use crate::database::{
    actions::{DatabaseDdlPreview, ValidatedDatabaseAction},
    dialect::{IdentifierQuoter, SqlDialect},
    errors::DatabaseError,
};

use super::POSTGRES_DIALECT;

/// Builds a preview for one validated PostgreSQL database action.
pub fn preview(action: &ValidatedDatabaseAction) -> Result<DatabaseDdlPreview, DatabaseError> {
    match action {
        ValidatedDatabaseAction::Create(options) => {
            let mut statement = format!(
                "CREATE DATABASE {}",
                POSTGRES_DIALECT.quote_identifier(&options.name)
            );
            let mut clauses = Vec::new();

            if let Some(owner) = options.owner.as_ref() {
                clauses.push(format!(
                    "OWNER = {}",
                    POSTGRES_DIALECT.quote_identifier(owner)
                ));
            }
            if let Some(template) = options.template.as_ref() {
                clauses.push(format!(
                    "TEMPLATE = {}",
                    POSTGRES_DIALECT.quote_identifier(template)
                ));
            }
            if let Some(encoding) = options.encoding.as_deref() {
                clauses.push(format!(
                    "ENCODING = {}",
                    string_literal("encoding", encoding)?
                ));
            }
            if let Some(collation) = options.collation.as_deref() {
                let collation = string_literal("collation", collation)?;
                clauses.push(format!("LC_COLLATE = {collation}"));
                clauses.push(format!("LC_CTYPE = {collation}"));
            }

            if !clauses.is_empty() {
                statement.push_str(" WITH ");
                statement.push_str(&clauses.join(" "));
            }

            let mut warnings = vec![
                "PostgreSQL records encoding and locale at creation time; they cannot be changed \
                 later."
                    .to_string(),
            ];
            if (options.encoding.is_some() || options.collation.is_some())
                && options.template.is_none()
            {
                warnings.push(
                    "A custom encoding or locale usually requires TEMPLATE = template0. PostgreSQL \
                     rejects combinations that do not match the selected template."
                        .to_string(),
                );
            }

            Ok(DatabaseDdlPreview::new(
                POSTGRES_DIALECT.engine(),
                action,
                vec![statement],
                warnings,
            ))
        }
        ValidatedDatabaseAction::Rename { name, new_name } => Ok(DatabaseDdlPreview::new(
            POSTGRES_DIALECT.engine(),
            action,
            vec![format!(
                "ALTER DATABASE {} RENAME TO {}",
                POSTGRES_DIALECT.quote_identifier(name),
                POSTGRES_DIALECT.quote_identifier(new_name)
            )],
            vec![
                "PostgreSQL cannot rename the database used by the current connection.".to_string(),
                "Sessions using the old catalog name must reconnect after the rename.".to_string(),
            ],
        )),
        ValidatedDatabaseAction::Drop { name } => Ok(DatabaseDdlPreview::new(
            POSTGRES_DIALECT.engine(),
            action,
            vec![format!(
                "DROP DATABASE {}",
                POSTGRES_DIALECT.quote_identifier(name)
            )],
            vec![
                "Dropping a database permanently removes every table, view, and row inside it."
                    .to_string(),
                "PostgreSQL cannot drop the database used by the current connection; switch the \
                 active database first."
                    .to_string(),
            ],
        )),
        ValidatedDatabaseAction::SetActive { name } => Ok(DatabaseDdlPreview::new(
            POSTGRES_DIALECT.engine(),
            action,
            Vec::new(),
            vec![format!(
                "PostgreSQL databases are separate catalogs. ServerDesk reopens the session \
                 against {} to change the browsing and SQL context.",
                name
            )],
        )),
    }
}

fn string_literal(label: &str, value: &str) -> Result<String, DatabaseError> {
    // Validation in `database::actions` already restricted this value to a
    // small ASCII allowlist. The extra check keeps a future caller from
    // interpolating a quote or control character.
    if value.is_empty()
        || value
            .chars()
            .any(|character| character == '\'' || character.is_control())
    {
        return Err(DatabaseError::validation(format!(
            "The PostgreSQL database {label} is not valid."
        )));
    }
    Ok(format!("'{}'", value.replace('\'', "''")))
}

#[cfg(test)]
mod tests {
    use crate::database::actions::{DatabaseActionRequest, ValidatedCreateDatabase};

    use super::*;

    fn create_request() -> ValidatedCreateDatabase {
        let ValidatedDatabaseAction::Create(options) = DatabaseActionRequest::Create {
            name: "app_next".to_string(),
            encoding: Some("UTF8".to_string()),
            collation: Some("C.UTF-8".to_string()),
            owner: Some("deploy".to_string()),
            template: Some("template0".to_string()),
        }
        .validate()
        .expect("valid create") else {
            unreachable!("create request");
        };
        options
    }

    #[test]
    fn create_preview_uses_engine_specific_owner_template_and_locale_options() {
        let preview =
            preview(&ValidatedDatabaseAction::Create(create_request())).expect("create preview");

        assert_eq!(
            preview.statements,
            [
                "CREATE DATABASE \"app_next\" WITH OWNER = \"deploy\" TEMPLATE = \"template0\" \
                 ENCODING = 'UTF8' LC_COLLATE = 'C.UTF-8' LC_CTYPE = 'C.UTF-8'"
            ]
        );
        assert_eq!(preview.engine, crate::storage::DatabaseEngine::Postgresql);
        assert!(
            preview
                .warnings
                .iter()
                .any(|warning| warning.contains("cannot be changed"))
        );
    }

    #[test]
    fn create_warns_when_a_custom_locale_has_no_template() {
        let ValidatedDatabaseAction::Create(mut options) = DatabaseActionRequest::Create {
            name: "app_next".to_string(),
            encoding: Some("UTF8".to_string()),
            collation: None,
            owner: None,
            template: None,
        }
        .validate()
        .expect("valid create") else {
            unreachable!("create request");
        };
        options.template = None;

        let preview = preview(&ValidatedDatabaseAction::Create(options)).expect("preview");
        assert!(
            preview
                .warnings
                .iter()
                .any(|warning| warning.contains("template0"))
        );
    }

    #[test]
    fn rename_and_drop_quote_catalog_names() {
        let rename = preview(&ValidatedDatabaseAction::Rename {
            name: crate::database::Identifier::new("app").expect("name"),
            new_name: crate::database::Identifier::new("odd\"name").expect("new name"),
        })
        .expect("rename preview");
        assert_eq!(
            rename.statements,
            ["ALTER DATABASE \"app\" RENAME TO \"odd\"\"name\""]
        );

        let drop = preview(&ValidatedDatabaseAction::Drop {
            name: crate::database::Identifier::new("app").expect("name"),
        })
        .expect("drop preview");
        assert_eq!(drop.statements, ["DROP DATABASE \"app\""]);
        assert!(drop.destructive);
    }

    #[test]
    fn set_active_preview_has_no_ddl_and_explains_the_reconnect() {
        let preview = preview(&ValidatedDatabaseAction::SetActive {
            name: crate::database::Identifier::new("app_next").expect("name"),
        })
        .expect("set active preview");

        assert!(preview.statements.is_empty());
        assert!(preview.ddl.is_empty());
        assert!(preview.warnings[0].contains("reopens the session"));
    }
}
