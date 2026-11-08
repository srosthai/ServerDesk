use serde::{Deserialize, Serialize};

use crate::{
    core::AppError,
    storage::{AuthType, NewServer, Server},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerProfile {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_type: AuthType,
    pub credential_ref: Option<String>,
    pub group_name: Option<String>,
    pub tags: Vec<String>,
    pub notes: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl From<Server> for ServerProfile {
    fn from(server: Server) -> Self {
        Self {
            id: server.id,
            name: server.name,
            host: server.host,
            port: server.port,
            username: server.username,
            auth_type: server.auth_type,
            credential_ref: server.credential_ref,
            group_name: server.group_name,
            tags: server.tags,
            notes: server.notes,
            created_at: server.created_at,
            updated_at: server.updated_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerProfileInput {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_type: AuthType,
    pub credential_ref: Option<String>,
    pub group_name: Option<String>,
    pub tags: Vec<String>,
    pub notes: Option<String>,
}

impl ServerProfileInput {
    pub fn into_new_server(self) -> Result<NewServer, AppError> {
        let name = self.name.trim().to_string();
        let host = self.host.trim().to_string();
        let username = self.username.trim().to_string();

        if name.is_empty() {
            return Err(AppError::validation("Server name is required."));
        }
        if name.len() > 80 {
            return Err(AppError::validation(
                "Server name must be 80 characters or fewer.",
            ));
        }
        if host.is_empty() {
            return Err(AppError::validation("Host is required."));
        }
        if host.len() > 253 || host.chars().any(char::is_whitespace) {
            return Err(AppError::validation(
                "Enter a valid hostname or IP address.",
            ));
        }
        if self.port == 0 {
            return Err(AppError::validation(
                "SSH port must be between 1 and 65535.",
            ));
        }
        if username.is_empty() || username.len() > 64 || username.chars().any(char::is_whitespace) {
            return Err(AppError::validation("Enter a valid SSH username."));
        }

        let tags = normalize_tags(self.tags)?;
        let group_name = normalize_optional(self.group_name, 120, "Group")?;
        let notes = normalize_optional(self.notes, 2000, "Notes")?;
        let credential_ref = normalize_optional(self.credential_ref, 256, "Credential reference")?;

        Ok(NewServer {
            name,
            host,
            port: self.port,
            username,
            auth_type: self.auth_type,
            credential_ref,
            group_name,
            tags,
            notes,
        })
    }
}

fn normalize_optional(
    value: Option<String>,
    max_length: usize,
    label: &str,
) -> Result<Option<String>, AppError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim().to_string();

    if value.is_empty() {
        return Ok(None);
    }
    if value.len() > max_length {
        return Err(AppError::validation(format!(
            "{label} must be {max_length} characters or fewer."
        )));
    }

    Ok(Some(value))
}

fn normalize_tags(tags: Vec<String>) -> Result<Vec<String>, AppError> {
    let mut normalized = Vec::new();

    for tag in tags {
        let tag = tag.trim().to_string();
        if tag.is_empty() {
            continue;
        }
        if tag.len() > 32 {
            return Err(AppError::validation(
                "Each tag must be 32 characters or fewer.",
            ));
        }
        if !normalized.iter().any(|existing: &String| existing == &tag) {
            normalized.push(tag);
        }
    }

    if normalized.len() > 12 {
        return Err(AppError::validation("Use 12 tags or fewer."));
    }

    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_input() -> ServerProfileInput {
        ServerProfileInput {
            name: " Production ".to_string(),
            host: " 198.51.100.24 ".to_string(),
            port: 22,
            username: " deploy ".to_string(),
            auth_type: AuthType::PrivateKey,
            credential_ref: Some(" credential:prod ".to_string()),
            group_name: Some(" Production ".to_string()),
            tags: vec![
                " web ".to_string(),
                "web".to_string(),
                "laravel".to_string(),
            ],
            notes: Some(" Primary host ".to_string()),
        }
    }

    #[test]
    fn normalizes_valid_input() {
        let server = valid_input().into_new_server().expect("valid server");

        assert_eq!(server.name, "Production");
        assert_eq!(server.host, "198.51.100.24");
        assert_eq!(server.username, "deploy");
        assert_eq!(server.credential_ref.as_deref(), Some("credential:prod"));
        assert_eq!(server.group_name.as_deref(), Some("Production"));
        assert_eq!(server.tags, vec!["web", "laravel"]);
        assert_eq!(server.notes.as_deref(), Some("Primary host"));
    }

    #[test]
    fn rejects_empty_name_host_and_username() {
        for input in [
            ServerProfileInput {
                name: "  ".to_string(),
                ..valid_input()
            },
            ServerProfileInput {
                host: "  ".to_string(),
                ..valid_input()
            },
            ServerProfileInput {
                username: " ".to_string(),
                ..valid_input()
            },
        ] {
            assert!(input.into_new_server().is_err());
        }
    }

    #[test]
    fn rejects_invalid_port_and_host_whitespace() {
        let bad_port = ServerProfileInput {
            port: 0,
            ..valid_input()
        };
        assert!(bad_port.into_new_server().is_err());

        let bad_host = ServerProfileInput {
            host: "bad host".to_string(),
            ..valid_input()
        };
        assert!(bad_host.into_new_server().is_err());
    }

    #[test]
    fn rejects_too_many_tags_and_long_tag() {
        let too_many = ServerProfileInput {
            tags: (0..13).map(|index| format!("tag-{index}")).collect(),
            ..valid_input()
        };
        assert!(too_many.into_new_server().is_err());

        let too_long = ServerProfileInput {
            tags: vec!["x".repeat(33)],
            ..valid_input()
        };
        assert!(too_long.into_new_server().is_err());
    }

    #[test]
    fn treats_blank_optional_fields_as_none() {
        let server = ServerProfileInput {
            credential_ref: Some("   ".to_string()),
            group_name: Some(" ".to_string()),
            notes: Some("".to_string()),
            tags: vec!["".to_string()],
            ..valid_input()
        }
        .into_new_server()
        .expect("valid server");

        assert!(server.credential_ref.is_none());
        assert!(server.group_name.is_none());
        assert!(server.notes.is_none());
        assert!(server.tags.is_empty());
    }
}
