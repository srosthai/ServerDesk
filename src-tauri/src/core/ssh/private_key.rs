use std::{borrow::Cow, path::PathBuf};

use russh::keys::{PrivateKey, decode_secret_key};

use super::TransportError;

pub(super) fn load_private_key(
    key: &str,
    passphrase: Option<&str>,
) -> Result<PrivateKey, TransportError> {
    let key = key.trim();
    if key.is_empty() {
        return Err(TransportError::Credential(
            "No private key is saved for this server.".to_string(),
        ));
    }

    let contents = if looks_like_private_key(key) {
        Cow::Borrowed(key)
    } else {
        let path = expand_home(key);
        Cow::Owned(std::fs::read_to_string(path).map_err(|_| {
            TransportError::Credential(
                "The saved private key could not be read. Save the key contents or a valid key path."
                    .to_string(),
            )
        })?)
    };

    let passphrase = passphrase.filter(|passphrase| !passphrase.is_empty());
    decode_secret_key(contents.as_ref(), passphrase).map_err(|error| match error {
        russh::keys::Error::KeyIsEncrypted => TransportError::Credential(
            "The private key is encrypted. Add its passphrase and try again.".to_string(),
        ),
        _ if passphrase.is_some() => TransportError::Credential(
            "The private key could not be unlocked. Check the passphrase and key.".to_string(),
        ),
        error if is_unsupported_key(&error) => TransportError::Credential(
            "The private key uses an unsupported format. Use an OpenSSH or PEM key.".to_string(),
        ),
        _ => TransportError::Credential(
            "The saved private key could not be decoded. Use an OpenSSH or PEM private key."
                .to_string(),
        ),
    })
}

pub fn validate_private_key(key: &str, passphrase: Option<&str>) -> Result<(), TransportError> {
    load_private_key(key, passphrase).map(|_| ())
}

fn is_unsupported_key(error: &russh::keys::Error) -> bool {
    matches!(
        error,
        russh::keys::Error::UnsupportedKeyType { .. } | russh::keys::Error::UnknownAlgorithm(_)
    )
}

fn looks_like_private_key(secret: &str) -> bool {
    secret.contains("BEGIN OPENSSH PRIVATE KEY")
        || secret.contains("BEGIN RSA PRIVATE KEY")
        || secret.contains("BEGIN EC PRIVATE KEY")
        || secret.contains("BEGIN PRIVATE KEY")
        || secret.contains("BEGIN ENCRYPTED PRIVATE KEY")
        || secret.starts_with("PuTTY-User-Key-File-")
}

fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))
    {
        return PathBuf::from(home).join(rest);
    }

    PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use russh::keys::{Algorithm, EcdsaCurve, encode_pkcs8_pem, ssh_key::LineEnding};

    use super::*;

    const PASSPHRASE: &str = "fixture-passphrase";

    fn generated_key(algorithm: Algorithm) -> PrivateKey {
        let mut rng = rand::rng();
        PrivateKey::random(&mut rng, algorithm).expect("generate key")
    }

    fn openssh_key(key: &PrivateKey) -> String {
        key.to_openssh(LineEnding::LF)
            .expect("encode key")
            .to_string()
    }

    fn encrypted_openssh_key(key: &PrivateKey) -> String {
        let mut rng = rand::rng();
        key.encrypt(&mut rng, PASSPHRASE)
            .expect("encrypt key")
            .to_openssh(LineEnding::LF)
            .expect("encode encrypted key")
            .to_string()
    }

    fn ed25519_key() -> PrivateKey {
        generated_key(Algorithm::Ed25519)
    }

    #[test]
    fn loads_unencrypted_openssh_key() {
        let key = load_private_key(&openssh_key(&ed25519_key()), None).expect("load key");

        assert_eq!(key.algorithm().as_str(), "ssh-ed25519");
    }

    #[test]
    fn loads_passphrase_protected_openssh_key() {
        let encrypted = encrypted_openssh_key(&ed25519_key());
        let key = load_private_key(&encrypted, Some(PASSPHRASE)).expect("load encrypted key");

        assert_eq!(key.algorithm().as_str(), "ssh-ed25519");
    }

    #[test]
    fn loads_supported_pem_and_openssh_formats() {
        let rsa = generated_key(Algorithm::Rsa { hash: None });
        let ecdsa = generated_key(Algorithm::Ecdsa {
            curve: EcdsaCurve::NistP256,
        });
        let pkcs8 = ed25519_key();
        let mut pkcs8_der = Vec::new();
        encode_pkcs8_pem(&pkcs8, &mut pkcs8_der).expect("encode PKCS#8 key");
        let pkcs8_pem = String::from_utf8(pkcs8_der).expect("PKCS#8 key is UTF-8");

        let loaded_rsa = load_private_key(&openssh_key(&rsa), None).expect("load RSA key");
        let loaded_ecdsa = load_private_key(&openssh_key(&ecdsa), None).expect("load ECDSA key");
        let loaded_pkcs8 = load_private_key(&pkcs8_pem, None).expect("load PKCS#8 key");

        assert_eq!(loaded_rsa.algorithm().as_str(), "ssh-rsa");
        assert_eq!(loaded_ecdsa.algorithm().as_str(), "ecdsa-sha2-nistp256");
        assert_eq!(loaded_pkcs8.algorithm().as_str(), "ssh-ed25519");
    }

    #[test]
    fn encrypted_key_requires_a_passphrase() {
        let encrypted = encrypted_openssh_key(&ed25519_key());
        let error =
            load_private_key(&encrypted, None).expect_err("encrypted key must need a passphrase");

        assert!(matches!(error, TransportError::Credential(_)));
        assert!(!format!("{error:?}").contains(PASSPHRASE));
    }

    #[test]
    fn wrong_passphrase_returns_a_credential_error() {
        let encrypted = encrypted_openssh_key(&ed25519_key());
        let error =
            load_private_key(&encrypted, Some("wrong")).expect_err("wrong passphrase must fail");

        assert!(matches!(error, TransportError::Credential(_)));
        assert!(!format!("{error:?}").contains("wrong"));
    }

    #[test]
    fn malformed_key_returns_a_credential_error() {
        let error = load_private_key("not a private key", None).expect_err("invalid key");

        assert!(matches!(error, TransportError::Credential(_)));
    }

    #[test]
    fn rejects_empty_key_material() {
        assert!(matches!(
            load_private_key("   ", None),
            Err(TransportError::Credential(_))
        ));
    }

    #[test]
    fn expands_home_paths() {
        if std::env::var_os("HOME").is_some() {
            assert!(expand_home("~/key").is_absolute());
        }
    }

    #[test]
    fn detects_supported_key_formats() {
        assert!(looks_like_private_key(
            "-----BEGIN OPENSSH PRIVATE KEY-----"
        ));
        assert!(looks_like_private_key(
            "-----BEGIN ENCRYPTED PRIVATE KEY-----"
        ));
        assert!(looks_like_private_key("PuTTY-User-Key-File-3: ssh-ed25519"));
        assert!(!looks_like_private_key("~/.ssh/id_ed25519"));
    }
}
