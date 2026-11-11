use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use crate::{
    core::{AppError, credential_index::read_index, credentials::CredentialStore},
    storage::Database,
};

const DATABASE_FILE_NAME: &str = "serverdesk.sqlite3";
const CREDENTIAL_INDEX_FILE_NAME: &str = "credential-references.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PurgeReport {
    pub credential_count: usize,
    pub removed_data_directory: bool,
}

/// Removes every known credential and all local application data.
///
/// The operation is fail-safe: if any credential cannot be deleted, the local
/// database and credential index are kept so the cleanup can be retried. This
/// prevents a half-finished uninstall from destroying the only record of which
/// keyring entries still need removal.
pub fn purge_local_data(
    data_directory: &Path,
    credentials: &dyn CredentialStore,
) -> Result<PurgeReport, AppError> {
    if !data_directory.exists() {
        return Ok(PurgeReport {
            credential_count: 0,
            removed_data_directory: false,
        });
    }

    let database_path = data_directory.join(DATABASE_FILE_NAME);
    let index_path = data_directory.join(CREDENTIAL_INDEX_FILE_NAME);

    let mut references = BTreeSet::new();
    if database_path.exists() {
        let database = Database::open(&database_path)?;
        for reference in database
            .list_servers()?
            .into_iter()
            .filter_map(|server| server.credential_ref)
        {
            references.insert(reference);
        }
        references.extend(database.list_database_connection_credential_refs()?);
    }
    references.extend(read_index(&index_path)?);

    let mut failures = 0;
    for reference in &references {
        if credentials.delete_secret(reference).is_err() {
            failures += 1;
        }
    }

    if failures > 0 {
        return Err(AppError::credential(format!(
            "Could not remove {failures} stored credential(s). Local data was kept so cleanup can be retried."
        )));
    }

    std::fs::remove_dir_all(data_directory)
        .map_err(|_| AppError::internal("Could not remove the local ServerDesk data directory."))?;

    Ok(PurgeReport {
        credential_count: references.len(),
        removed_data_directory: true,
    })
}

/// Resolves the default application data directory without a running Tauri app.
///
/// This mirrors `tauri::path::PathResolver::app_data_dir`, which appends the
/// bundle identifier to the platform data directory.
pub fn default_data_directory() -> Result<PathBuf, AppError> {
    dirs::data_dir()
        .map(|directory| directory.join("com.serverdesk.app"))
        .ok_or_else(|| {
            AppError::internal("Could not resolve the local application data directory.")
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        core::{AppError, credentials::MemoryCredentialStore},
        storage::{AuthType, DatabaseEngine, NewDatabaseConnection, NewServer},
    };

    fn server(reference: Option<&str>) -> NewServer {
        NewServer {
            name: "Production".to_string(),
            host: "203.0.113.10".to_string(),
            port: 22,
            username: "deploy".to_string(),
            auth_type: AuthType::Password,
            credential_ref: reference.map(str::to_string),
            group_name: None,
            tags: vec![],
            notes: None,
        }
    }

    #[test]
    fn removes_database_and_indexed_credentials() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let database = Database::open(directory.path().join(DATABASE_FILE_NAME)).expect("database");
        database
            .create_server(&server(Some("serverdesk:ssh:database")))
            .expect("saved server");
        drop(database);

        let index = crate::core::credential_index::CredentialIndex::open(directory.path())
            .expect("credential index");
        index
            .add("serverdesk:ssh:orphan")
            .expect("orphan reference");

        let credentials = MemoryCredentialStore::default();
        credentials
            .set_secret("serverdesk:ssh:database", "database-secret")
            .expect("database secret");
        credentials
            .set_secret("serverdesk:ssh:orphan", "orphan-secret")
            .expect("orphan secret");

        let report = purge_local_data(directory.path(), &credentials).expect("purge");

        assert_eq!(report.credential_count, 2);
        assert!(report.removed_data_directory);
        assert!(!directory.path().exists());
        assert!(credentials.get_secret("serverdesk:ssh:database").is_err());
        assert!(credentials.get_secret("serverdesk:ssh:orphan").is_err());
    }

    #[test]
    fn removes_database_connection_credentials_even_without_an_index_entry() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let database = Database::open(directory.path().join(DATABASE_FILE_NAME)).expect("database");
        let server = database
            .create_server(&server(Some("serverdesk:ssh:database")))
            .expect("saved server");
        database
            .create_database_connection(&NewDatabaseConnection {
                server_id: server.id,
                name: "Primary".to_string(),
                engine: DatabaseEngine::Postgresql,
                host: "127.0.0.1".to_string(),
                port: 5432,
                socket: None,
                database_name: Some("app".to_string()),
                username: "app".to_string(),
                credential_ref: Some(
                    "serverdesk:db:00000000-0000-4000-8000-000000000001".to_string(),
                ),
                read_only: true,
                ssl_mode: Some("require".to_string()),
            })
            .expect("saved database connection");
        drop(database);

        let credentials = MemoryCredentialStore::default();
        credentials
            .set_secret("serverdesk:ssh:database", "ssh-secret")
            .expect("ssh secret");
        credentials
            .set_secret(
                "serverdesk:db:00000000-0000-4000-8000-000000000001",
                "database-secret",
            )
            .expect("database secret");

        let report = purge_local_data(directory.path(), &credentials).expect("purge");

        assert_eq!(report.credential_count, 2);
        assert!(credentials.get_secret("serverdesk:ssh:database").is_err());
        assert!(
            credentials
                .get_secret("serverdesk:db:00000000-0000-4000-8000-000000000001")
                .is_err()
        );
    }

    #[test]
    fn keeps_data_when_a_credential_cannot_be_deleted() {
        struct FailingStore;

        impl CredentialStore for FailingStore {
            fn set_secret(&self, _reference: &str, _secret: &str) -> Result<(), AppError> {
                Ok(())
            }

            fn get_secret(&self, _reference: &str) -> Result<String, AppError> {
                Err(AppError::not_found("missing"))
            }

            fn delete_secret(&self, _reference: &str) -> Result<(), AppError> {
                Err(AppError::credential("keyring unavailable"))
            }
        }

        let directory = tempfile::tempdir().expect("temporary directory");
        let database = Database::open(directory.path().join(DATABASE_FILE_NAME)).expect("database");
        database
            .create_server(&server(Some("serverdesk:ssh:database")))
            .expect("saved server");
        drop(database);

        let error = purge_local_data(directory.path(), &FailingStore).expect_err("purge fails");

        assert_eq!(error.code, crate::core::ErrorCode::Credential);
        assert!(directory.path().exists());
        assert!(directory.path().join(DATABASE_FILE_NAME).exists());
    }

    #[test]
    fn missing_data_directory_is_a_successful_noop() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let missing = directory.path().join("not-created");
        let credentials = MemoryCredentialStore::default();

        let report = purge_local_data(&missing, &credentials).expect("purge noop");

        assert_eq!(report.credential_count, 0);
        assert!(!report.removed_data_directory);
    }
}
