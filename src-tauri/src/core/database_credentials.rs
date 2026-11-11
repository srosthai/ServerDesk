//! Database passwords live only in the OS credential store.
//!
//! Database profiles persist an opaque `serverdesk:db:<uuid>` reference in
//! SQLite. The password itself is never written to SQLite, logs, command
//! errors, or a frontend payload. This service owns creation, replacement,
//! verification, and deletion so every operation also keeps the credential
//! index current for `--purge-data`.

use uuid::Uuid;

use crate::{
    core::{
        AppError, ErrorCode,
        credential_index::CredentialIndex,
        credentials::{CredentialStore, validate_reference, validate_secret},
    },
    storage::Database,
};

/// Prefix that distinguishes database credentials from SSH credentials.
pub const DATABASE_CREDENTIAL_PREFIX: &str = "serverdesk:db:";

/// Generates an opaque reference for one saved database password.
pub fn new_database_reference() -> String {
    format!("{DATABASE_CREDENTIAL_PREFIX}{}", Uuid::new_v4())
}

/// Returns whether a reference belongs to the database credential namespace.
pub fn is_database_reference(reference: &str) -> bool {
    reference
        .strip_prefix(DATABASE_CREDENTIAL_PREFIX)
        .and_then(|suffix| Uuid::parse_str(suffix).ok())
        .is_some()
}

/// Rejects references from other credential namespaces such as SSH.
pub fn validate_database_reference(reference: &str) -> Result<(), AppError> {
    validate_reference(reference)?;
    if !is_database_reference(reference) {
        return Err(AppError::validation(
            "Invalid database credential reference.",
        ));
    }

    Ok(())
}

/// Coordinates database passwords between the credential store and its index.
pub struct DatabaseCredentialService<'a> {
    credentials: &'a dyn CredentialStore,
    index: &'a CredentialIndex,
}

impl<'a> DatabaseCredentialService<'a> {
    pub fn new(credentials: &'a dyn CredentialStore, index: &'a CredentialIndex) -> Self {
        Self { credentials, index }
    }

    /// Saves a new database password and returns its opaque reference.
    ///
    /// The reference is indexed before the secret is written. If the
    /// credential store rejects the write, the index entry is rolled back so a
    /// failed save cannot leave a phantom reference behind.
    pub fn create(&self, password: &str) -> Result<String, AppError> {
        validate_database_password(password)?;

        let reference = new_database_reference();
        self.index.add(&reference)?;
        if let Err(error) = self.credentials.set_secret(&reference, password) {
            let _ = self.index.remove(&reference);
            return Err(map_database_credential_error(error));
        }

        Ok(reference)
    }

    /// Replaces the password for an existing database reference.
    ///
    /// Replacing also heals a reference whose secret was removed from the
    /// credential store before the profile was deleted.
    pub fn replace(&self, reference: &str, password: &str) -> Result<(), AppError> {
        validate_database_reference(reference)?;
        validate_database_password(password)?;

        let was_indexed = self.index.contains(reference)?;
        self.index.add(reference)?;
        if let Err(error) = self.credentials.set_secret(reference, password) {
            if !was_indexed {
                let _ = self.index.remove(reference);
            }
            return Err(map_database_credential_error(error));
        }

        Ok(())
    }

    /// Checks that the saved database password is readable and valid.
    ///
    /// The password is read and discarded. This method is safe to expose as a
    /// verification command because it never returns secret material.
    pub fn verify(&self, reference: &str) -> Result<(), AppError> {
        self.read(reference).map(|_| ())
    }

    /// Reads a database password for a backend connection attempt.
    ///
    /// This value must never be returned from a Tauri command or written to a
    /// log. Adapters pass it directly to the database driver.
    pub fn read(&self, reference: &str) -> Result<String, AppError> {
        validate_database_reference(reference)?;
        self.credentials
            .get_secret(reference)
            .map_err(map_database_credential_error)
    }

    /// Deletes a saved database password.
    ///
    /// Deletion is idempotent so a profile can still be removed when its
    /// credential entry was already cleaned up separately.
    pub fn delete(&self, reference: &str) -> Result<(), AppError> {
        validate_database_reference(reference)?;
        self.delete_reference(reference)
    }

    /// Deletes a database profile and its credential in the safe order.
    ///
    /// The credential entry is removed before the SQLite row. If the OS
    /// credential store is unavailable or locked, the profile is kept so the
    /// operation can be retried instead of leaving an orphaned secret behind.
    pub fn delete_profile(&self, database: &Database, connection_id: &str) -> Result<(), AppError> {
        let reference = database
            .get_database_connection(connection_id)?
            .and_then(|profile| profile.credential_ref);

        if let Some(reference) = reference.as_deref() {
            self.delete_reference(reference)?;
        }

        database.delete_database_connection(connection_id)?;
        Ok(())
    }

    fn delete_reference(&self, reference: &str) -> Result<(), AppError> {
        validate_reference(reference)?;
        self.credentials
            .delete_secret(reference)
            .map_err(map_database_credential_error)?;
        let _ = self.index.remove(reference);
        Ok(())
    }
}

fn validate_database_password(password: &str) -> Result<(), AppError> {
    if password.is_empty() {
        return Err(AppError::validation(
            "The database password cannot be empty.",
        ));
    }
    validate_secret(password)
}

fn map_database_credential_error(error: AppError) -> AppError {
    match error.code {
        ErrorCode::NotFound => AppError::credential_missing(
            "The saved database password is missing. Save the password again.",
        ),
        _ => error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        core::credentials::MemoryCredentialStore,
        storage::{AuthType, DatabaseEngine, NewDatabaseConnection, NewServer},
    };

    fn index() -> (tempfile::TempDir, CredentialIndex) {
        let directory = tempfile::tempdir().expect("temporary directory");
        let index = CredentialIndex::open(directory.path()).expect("credential index");
        (directory, index)
    }

    fn server(database: &Database) -> String {
        database
            .create_server(&NewServer {
                name: "Production".to_string(),
                host: "198.51.100.24".to_string(),
                port: 22,
                username: "deploy".to_string(),
                auth_type: AuthType::Password,
                credential_ref: Some("serverdesk:ssh:test-server".to_string()),
                group_name: None,
                tags: vec![],
                notes: None,
            })
            .expect("create server")
            .id
    }

    fn connection(server_id: &str, credential_ref: Option<&str>) -> NewDatabaseConnection {
        NewDatabaseConnection {
            server_id: server_id.to_string(),
            name: "Primary".to_string(),
            engine: DatabaseEngine::Postgresql,
            host: "127.0.0.1".to_string(),
            port: 5432,
            socket: None,
            database_name: Some("app".to_string()),
            username: "app".to_string(),
            credential_ref: credential_ref.map(str::to_string),
            read_only: true,
            ssl_mode: Some("require".to_string()),
        }
    }

    struct FailingStore {
        error: AppError,
    }

    impl CredentialStore for FailingStore {
        fn set_secret(&self, _reference: &str, _secret: &str) -> Result<(), AppError> {
            Err(self.error.clone())
        }

        fn get_secret(&self, _reference: &str) -> Result<String, AppError> {
            Err(self.error.clone())
        }

        fn delete_secret(&self, _reference: &str) -> Result<(), AppError> {
            Err(self.error.clone())
        }
    }

    struct DeleteFailingStore {
        inner: MemoryCredentialStore,
        error: AppError,
    }

    impl CredentialStore for DeleteFailingStore {
        fn set_secret(&self, reference: &str, secret: &str) -> Result<(), AppError> {
            self.inner.set_secret(reference, secret)
        }

        fn get_secret(&self, reference: &str) -> Result<String, AppError> {
            self.inner.get_secret(reference)
        }

        fn delete_secret(&self, _reference: &str) -> Result<(), AppError> {
            Err(self.error.clone())
        }
    }

    #[test]
    fn creates_reads_replaces_and_deletes_a_database_credential() {
        let (_directory, index) = index();
        let credentials = MemoryCredentialStore::default();
        let service = DatabaseCredentialService::new(&credentials, &index);

        let reference = service.create("first-password").expect("create credential");
        assert!(reference.starts_with(DATABASE_CREDENTIAL_PREFIX));
        assert!(is_database_reference(&reference));
        assert_eq!(
            credentials.get_secret(&reference).expect("stored secret"),
            "first-password"
        );
        assert_eq!(
            service.read(&reference).expect("read secret"),
            "first-password"
        );
        service.verify(&reference).expect("verify secret");
        assert_eq!(
            index.references().expect("indexed references"),
            vec![reference.clone()]
        );

        service
            .replace(&reference, "second-password")
            .expect("replace credential");
        assert_eq!(
            service.read(&reference).expect("read replaced secret"),
            "second-password"
        );
        assert!(!service.read(&reference).expect("secret").contains("first"));

        service.delete(&reference).expect("delete credential");
        assert!(credentials.get_secret(&reference).is_err());
        assert!(index.references().expect("indexed references").is_empty());
        assert_eq!(
            service.read(&reference).expect_err("missing secret").code,
            ErrorCode::CredentialMissing
        );
        assert_eq!(
            service.verify(&reference).expect_err("missing secret").code,
            ErrorCode::CredentialMissing
        );
    }

    #[test]
    fn create_failure_rolls_back_the_index_and_preserves_typed_errors() {
        for error in [
            AppError::credential_unavailable("credential store unavailable"),
            AppError::credential_locked("credential store locked"),
            AppError::credential_invalid("credential store returned invalid data"),
        ] {
            let (_directory, index) = index();
            let expected = error.code;
            let store = FailingStore { error };
            let service = DatabaseCredentialService::new(&store, &index);

            let error = service.create("database-password").expect_err("save fails");

            assert_eq!(error.code, expected);
            assert!(index.references().expect("indexed references").is_empty());
        }
    }

    #[test]
    fn replace_failure_does_not_leave_a_new_index_entry() {
        let (_directory, index) = index();
        let store = FailingStore {
            error: AppError::credential_locked("credential store locked"),
        };
        let service = DatabaseCredentialService::new(&store, &index);
        let reference = new_database_reference();

        let error = service
            .replace(&reference, "database-password")
            .expect_err("replace fails");

        assert_eq!(error.code, ErrorCode::CredentialLocked);
        assert!(index.references().expect("indexed references").is_empty());
    }

    #[test]
    fn rejects_references_from_other_credential_namespaces() {
        let (_directory, index) = index();
        let credentials = MemoryCredentialStore::default();
        let service = DatabaseCredentialService::new(&credentials, &index);
        let ssh_reference = "serverdesk:ssh:00000000-0000-4000-8000-000000000001";

        assert_eq!(
            service
                .replace(ssh_reference, "database-password")
                .expect_err("replace rejects SSH reference")
                .code,
            ErrorCode::Validation
        );
        assert_eq!(
            service
                .read(ssh_reference)
                .expect_err("read rejects SSH reference")
                .code,
            ErrorCode::Validation
        );
        assert_eq!(
            service
                .delete(ssh_reference)
                .expect_err("delete rejects SSH reference")
                .code,
            ErrorCode::Validation
        );
    }

    #[test]
    fn rejects_empty_database_passwords_without_touching_the_index() {
        let (_directory, index) = index();
        let credentials = MemoryCredentialStore::default();
        let service = DatabaseCredentialService::new(&credentials, &index);

        let error = service.create("").expect_err("empty password");

        assert_eq!(error.code, ErrorCode::Validation);
        assert!(index.references().expect("indexed references").is_empty());
    }

    #[test]
    fn persists_only_the_opaque_reference_on_the_database_profile() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let database_path = directory.path().join("serverdesk.sqlite3");
        let database = Database::open(&database_path).expect("file database");
        let credentials = MemoryCredentialStore::default();
        let index = CredentialIndex::open(directory.path()).expect("credential index");
        let service = DatabaseCredentialService::new(&credentials, &index);
        let server_id = server(&database);
        let password = "database-password-that-must-not-reach-sqlite";
        let reference = service.create(password).expect("create credential");
        let profile = database
            .create_database_connection(&connection(&server_id, Some(&reference)))
            .expect("create database profile");

        assert_eq!(profile.credential_ref.as_deref(), Some(reference.as_str()));
        drop(database);

        let mut stored_bytes = Vec::new();
        for entry in std::fs::read_dir(directory.path()).expect("data directory") {
            let path = entry.expect("data directory entry").path();
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("serverdesk.sqlite3"))
            {
                stored_bytes.extend(std::fs::read(path).expect("database file"));
            }
        }
        let stored_text = String::from_utf8_lossy(&stored_bytes);

        assert!(!stored_text.contains(password));
        assert!(stored_text.contains(&reference));
    }

    #[test]
    fn deleting_a_profile_removes_the_credential_before_the_metadata() {
        let (_directory, index) = index();
        let credentials = MemoryCredentialStore::default();
        let service = DatabaseCredentialService::new(&credentials, &index);
        let database = Database::in_memory().expect("in-memory database");
        let server_id = server(&database);
        let reference = service.create("database-password").expect("credential");
        let profile = database
            .create_database_connection(&connection(&server_id, Some(&reference)))
            .expect("create database profile");

        service
            .delete_profile(&database, &profile.id)
            .expect("delete profile");

        assert!(
            database
                .get_database_connection(&profile.id)
                .expect("read profile")
                .is_none()
        );
        assert!(credentials.get_secret(&reference).is_err());
        assert!(index.references().expect("indexed references").is_empty());
    }

    #[test]
    fn credential_failure_blocks_profile_deletion_and_keeps_metadata() {
        let (_directory, index) = index();
        let credentials = DeleteFailingStore {
            inner: MemoryCredentialStore::default(),
            error: AppError::credential_locked("credential store locked"),
        };
        let service = DatabaseCredentialService::new(&credentials, &index);
        let database = Database::in_memory().expect("in-memory database");
        let server_id = server(&database);
        let reference = service.create("database-password").expect("credential");
        let profile = database
            .create_database_connection(&connection(&server_id, Some(&reference)))
            .expect("create database profile");

        let error = service
            .delete_profile(&database, &profile.id)
            .expect_err("locked store blocks delete");

        assert_eq!(error.code, ErrorCode::CredentialLocked);
        assert!(
            database
                .get_database_connection(&profile.id)
                .expect("read profile")
                .is_some()
        );
        assert!(
            index
                .references()
                .expect("indexed references")
                .contains(&reference)
        );
        assert_eq!(
            credentials
                .get_secret(&reference)
                .expect("credential still readable"),
            "database-password"
        );
    }
}
