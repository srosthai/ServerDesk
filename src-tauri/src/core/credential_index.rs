use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::Mutex,
};

use serde::{Deserialize, Serialize};

use crate::core::AppError;

const INDEX_FILE_NAME: &str = "credential-references.json";
const INDEX_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct CredentialIndexFile {
    version: u32,
    references: BTreeSet<String>,
}

/// Tracks credential references created by this installation.
///
/// SQLite stores references for saved servers, but a secret can be created and
/// left behind if the application exits before the server profile is saved.
/// The index closes that gap so the uninstall purge can remove every keyring
/// entry created by this installation. It contains references only, never
/// secret values.
pub struct CredentialIndex {
    path: PathBuf,
    references: Mutex<BTreeSet<String>>,
}

impl CredentialIndex {
    pub fn open(data_directory: &Path) -> Result<Self, AppError> {
        let path = data_directory.join(INDEX_FILE_NAME);
        let references = read_index(&path)?;

        Ok(Self {
            path,
            references: Mutex::new(references),
        })
    }

    pub fn add(&self, reference: &str) -> Result<(), AppError> {
        let mut references = self.lock()?;
        if !references.insert(reference.to_string()) {
            return Ok(());
        }

        if let Err(error) = write_index(&self.path, &references) {
            references.remove(reference);
            return Err(error);
        }

        Ok(())
    }

    pub fn remove(&self, reference: &str) -> Result<(), AppError> {
        let mut references = self.lock()?;
        if !references.remove(reference) {
            return Ok(());
        }

        write_index(&self.path, &references)
    }

    pub fn references(&self) -> Result<Vec<String>, AppError> {
        Ok(self.lock()?.iter().cloned().collect())
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, BTreeSet<String>>, AppError> {
        self.references
            .lock()
            .map_err(|_| AppError::internal("The local credential index is unavailable."))
    }
}

pub fn read_index(path: &Path) -> Result<BTreeSet<String>, AppError> {
    if !path.exists() {
        return Ok(BTreeSet::new());
    }

    let contents = std::fs::read_to_string(path)
        .map_err(|_| AppError::internal("Could not read the local credential index."))?;
    let index: CredentialIndexFile = serde_json::from_str(&contents)
        .map_err(|_| AppError::internal("The local credential index is malformed."))?;

    if index.version != INDEX_VERSION {
        return Err(AppError::internal(
            "The local credential index has an unsupported version.",
        ));
    }

    Ok(index.references)
}

fn write_index(path: &Path, references: &BTreeSet<String>) -> Result<(), AppError> {
    let contents = serde_json::to_vec(&CredentialIndexFile {
        version: INDEX_VERSION,
        references: references.clone(),
    })
    .map_err(|_| AppError::internal("Could not encode the local credential index."))?;

    std::fs::write(path, contents)
        .map_err(|_| AppError::internal("Could not save the local credential index."))?;
    restrict_index_permissions(path);

    Ok(())
}

fn restrict_index_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        if let Ok(metadata) = std::fs::metadata(path) {
            let mut permissions = metadata.permissions();
            permissions.set_mode(0o600);
            let _ = std::fs::set_permissions(path, permissions);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_references_and_removes_duplicates() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let index = CredentialIndex::open(directory.path()).expect("credential index");

        index.add("serverdesk:ssh:one").expect("add reference");
        index.add("serverdesk:ssh:one").expect("add duplicate");
        index.add("serverdesk:ssh:two").expect("add reference");
        index
            .remove("serverdesk:ssh:one")
            .expect("remove reference");

        assert_eq!(
            index.references().expect("references"),
            vec!["serverdesk:ssh:two".to_string()]
        );

        let reopened = CredentialIndex::open(directory.path()).expect("reopened index");
        assert_eq!(
            reopened.references().expect("references"),
            vec!["serverdesk:ssh:two".to_string()]
        );
    }

    #[test]
    fn rejects_malformed_index_files() {
        let directory = tempfile::tempdir().expect("temporary directory");
        std::fs::write(directory.path().join(INDEX_FILE_NAME), b"not valid json")
            .expect("write malformed index");

        assert!(CredentialIndex::open(directory.path()).is_err());
    }
}
