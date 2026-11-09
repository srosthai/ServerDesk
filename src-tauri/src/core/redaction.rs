//! Central redaction for diagnostics, command output, and bounded log tails.
//!
//! Redaction is intentionally defense in depth: callers must still avoid
//! storing raw command output, terminal bytes, environment files, or
//! credentials. The patterns cover dotenv lines, secret assignments, bearer
//! tokens, credentials embedded in URLs, common token prefixes, and private-key
//! blocks.

/// Text after redaction plus whether at least one value was replaced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RedactedText {
    pub text: String,
    pub redacted: bool,
}

/// Redacts secret patterns and drops private-key block bodies.
///
/// Lines are joined with `\n` and the original trailing-newline state is
/// preserved. Callers that display line-oriented content can use
/// [`redact_text_preserving_lines`] instead.
pub fn redact_text(input: &str) -> RedactedText {
    redact(input, false)
}

/// Redacts secret patterns while keeping one output line per input line.
///
/// Private-key block lines are replaced rather than dropped, so a log tail
/// keeps its line count and metadata.
pub fn redact_text_preserving_lines(input: &str) -> RedactedText {
    redact(input, true)
}

/// Redacts text and bounds it to `max_chars` characters.
pub fn sanitize_text_bounded(input: &str, max_chars: usize) -> String {
    let redacted = redact_text(input);
    truncate_chars(redacted.text.trim_end(), max_chars)
}

fn redact(input: &str, preserve_private_key_lines: bool) -> RedactedText {
    let stripped = strip_ansi(input);
    let ends_with_newline = stripped.ends_with('\n');
    let mut output = String::with_capacity(stripped.len());
    let mut redacted = false;
    let mut in_private_key = false;

    for line in stripped.lines() {
        if in_private_key {
            redacted = true;
            if is_private_key_end(line) {
                in_private_key = false;
            }
            if preserve_private_key_lines {
                output.push_str("[redacted private key line]");
                output.push('\n');
            }
            continue;
        }

        if is_private_key_begin(line) {
            redacted = true;
            in_private_key = !is_private_key_end(line);
            output.push_str("[redacted private key]");
            output.push('\n');
            continue;
        }

        if looks_like_dotenv_line(line) {
            redacted = true;
            output.push_str("[redacted environment line]");
        } else if let Some(value) = redact_bearer_token(line) {
            redacted = true;
            output.push_str(&value);
        } else if let Some(value) = redact_url_password(line) {
            redacted = true;
            output.push_str(&value);
        } else if let Some(value) = redact_secret_assignment(line) {
            redacted = true;
            output.push_str(&value);
        } else if let Some(value) = redact_token_shape(line) {
            redacted = true;
            output.push_str(&value);
        } else {
            output.push_str(&strip_line_controls(line));
        }
        output.push('\n');
    }

    if !ends_with_newline && output.ends_with('\n') {
        output.pop();
    }

    RedactedText {
        text: output,
        redacted,
    }
}

fn truncate_chars(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    if max_chars == 0 {
        return String::new();
    }

    let mut truncated = input
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    truncated.push('…');
    truncated
}

fn strip_ansi(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut characters = input.chars().peekable();

    while let Some(character) = characters.next() {
        if character != '\x1b' {
            output.push(character);
            continue;
        }

        match characters.peek() {
            Some('[') => {
                characters.next();
                for character in characters.by_ref() {
                    if ('@'..='~').contains(&character) {
                        break;
                    }
                }
            }
            Some(']') => {
                characters.next();
                while let Some(character) = characters.next() {
                    if character == '\x07' {
                        break;
                    }
                    if character == '\x1b' && characters.peek() == Some(&'\\') {
                        characters.next();
                        break;
                    }
                }
            }
            Some(_) => {
                characters.next();
            }
            None => {}
        }
    }

    output
}

fn strip_line_controls(line: &str) -> String {
    line.chars()
        .filter(|character| !character.is_control() || *character == '\t')
        .collect()
}

fn is_private_key_begin(line: &str) -> bool {
    let line = line.trim();
    line.starts_with("-----BEGIN") && line.contains("PRIVATE KEY")
}

fn is_private_key_end(line: &str) -> bool {
    let line = line.trim();
    line.starts_with("-----END") && line.contains("PRIVATE KEY")
}

/// Replaces every dotenv-style assignment, not just entries that look secret.
/// Raw environment files must never reach diagnostics or history.
fn looks_like_dotenv_line(line: &str) -> bool {
    let trimmed = line.trim();
    let candidate = trimmed.strip_prefix("export ").unwrap_or(trimmed);
    let Some((key, _)) = candidate.split_once('=') else {
        return false;
    };
    let key = key.trim();
    !key.is_empty()
        && key
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_uppercase())
        && key.chars().all(|character| {
            character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
        })
}

fn redact_secret_assignment(line: &str) -> Option<String> {
    for (index, separator) in line
        .char_indices()
        .filter(|(_, character)| matches!(character, '=' | ':'))
    {
        // Use the token immediately before the separator so a message prefix
        // such as "Remote command failed: DB_PASSWORD=..." does not hide the
        // actual key. A line may contain several candidates.
        let key = line[..index]
            .split_whitespace()
            .next_back()
            .unwrap_or_default()
            .trim_matches(|character| matches!(character, '"' | '\'' | '{' | '[' | ','));
        // A sudoers NOPASSWD rule is policy text, not a credential. Do not
        // replace the scoped rule ServerDesk shows when sudo denies an action.
        if key.eq_ignore_ascii_case("NOPASSWD") {
            continue;
        }
        let value = &line[index + separator.len_utf8()..];
        if value.trim().is_empty() || !is_secret_key(key) {
            continue;
        }

        return Some(format!(
            "{}<redacted>",
            &line[..index + separator.len_utf8()]
        ));
    }

    None
}

fn is_secret_key(key: &str) -> bool {
    let key = key.trim().to_ascii_uppercase();
    [
        "PASSWORD",
        "PASSWD",
        "PASSPHRASE",
        "SECRET",
        "TOKEN",
        "PRIVATE_KEY",
        "PRIVATE KEY",
        "API_KEY",
        "APIKEY",
        "APP_KEY",
        "ACCESS_KEY",
        "CREDENTIAL",
        "AUTH_KEY",
        "SIGNING_KEY",
        "ENCRYPTION_KEY",
        "AUTHORIZATION",
        "COOKIE",
        "SESSION",
    ]
    .iter()
    .any(|needle| key.contains(needle))
        || key == "KEY"
        || key.ends_with("_KEY")
        || key.starts_with("KEY_")
}

fn redact_bearer_token(line: &str) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    let index = lower.find("bearer ")?;
    let value_start = index + "bearer ".len();
    if line[value_start..].trim().is_empty() {
        return None;
    }

    Some(format!("{}<redacted>", &line[..value_start]))
}

fn redact_url_password(line: &str) -> Option<String> {
    let scheme = line.find("://")?;
    let authority_start = scheme + 3;
    let authority_len = line[authority_start..]
        .find(['/', '?', '#'])
        .unwrap_or(line.len() - authority_start);
    let authority_end = authority_start + authority_len;
    let at = authority_start + line[authority_start..authority_end].rfind('@')?;
    let colon = authority_start + line[authority_start..at].find(':')?;
    if colon + 1 >= at {
        return None;
    }

    Some(format!("{}:<redacted>{}", &line[..colon], &line[at..]))
}

/// Redacts common token shapes even when they are not in an assignment.
fn redact_token_shape(line: &str) -> Option<String> {
    const TOKEN_PREFIXES: [&str; 8] = [
        "ghp_",
        "gho_",
        "ghu_",
        "ghs_",
        "github_pat_",
        "xoxb-",
        "xoxp-",
        "sk-",
    ];

    for prefix in TOKEN_PREFIXES {
        let Some(index) = line.find(prefix) else {
            continue;
        };
        let starts_cleanly = index == 0
            || !line[..index]
                .chars()
                .next_back()
                .is_some_and(|character| character.is_ascii_alphanumeric() || character == '_');
        if !starts_cleanly {
            continue;
        }
        let token = line[index..]
            .chars()
            .take_while(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
            })
            .count();
        if token <= prefix.len() + 4 {
            continue;
        }
        return Some(format!("{}[redacted token]", &line[..index]));
    }

    let upper = line.to_ascii_uppercase();
    for prefix in ["AKIA", "ASIA"] {
        let Some(index) = upper.find(prefix) else {
            continue;
        };
        let token = line[index..]
            .chars()
            .take_while(|character| character.is_ascii_alphanumeric())
            .count();
        if token >= 16 {
            return Some(format!("{}[redacted token]", &line[..index]));
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAX: usize = 4_096;

    #[test]
    fn redacts_assignments_bearer_tokens_and_credentials_in_urls() {
        let sanitized = sanitize_text_bounded(
            "DB_PASSWORD=hunter2\n\
             APP_KEY=base64:super-secret\n\
             passphrase: correct horse battery staple\n\
             Authorization: Bearer abc123token\n\
             DATABASE_URL=mysql://deploy:db-secret@127.0.0.1/app\n\
             api_key=sk-live-1234567890abcdef\n\
             GITHUB_TOKEN=ghp_1234567890abcdef\n",
            MAX,
        );

        for secret in [
            "hunter2",
            "base64:super-secret",
            "correct horse battery staple",
            "abc123token",
            "db-secret",
            "sk-live-1234567890abcdef",
            "ghp_1234567890abcdef",
            "DB_PASSWORD=",
            "APP_KEY=",
        ] {
            assert!(
                !sanitized.contains(secret),
                "sanitized text still contains {secret:?}: {sanitized}"
            );
        }
        assert!(sanitized.contains("[redacted environment line]"));
        assert!(sanitized.contains("passphrase:<redacted>"));
        assert!(sanitized.contains("Bearer <redacted>"));
        assert!(sanitized.contains("api_key=<redacted>"));
    }

    #[test]
    fn redacts_private_key_blocks_and_strips_ansi() {
        let sanitized = sanitize_text_bounded(
            "\u{1b}[32mkey follows\u{1b}[0m\n\
             -----BEGIN OPENSSH PRIVATE KEY-----\n\
             super-secret-key-body\n\
             -----END OPENSSH PRIVATE KEY-----\n\
             done\n",
            MAX,
        );

        assert!(!sanitized.contains("super-secret-key-body"));
        assert!(!sanitized.contains("BEGIN OPENSSH"));
        assert!(!sanitized.contains('\u{1b}'));
        assert!(sanitized.contains("[redacted private key]"));
        assert!(sanitized.contains("key follows"));
        assert!(sanitized.contains("done"));
    }

    #[test]
    fn preserving_redaction_keeps_line_count() {
        let output = redact_text_preserving_lines(
            "before\n-----BEGIN OPENSSH PRIVATE KEY-----\nsecret body\n-----END OPENSSH PRIVATE KEY-----\nafter\n",
        );

        assert!(output.redacted);
        assert!(!output.text.contains("secret body"));
        assert_eq!(output.text.lines().count(), 5);
        assert!(output.text.starts_with("before\n"));
        assert!(output.text.ends_with("after\n"));
    }

    #[test]
    fn redacts_common_token_shapes_without_assignments() {
        for token in [
            "ghp_abcdefghijklmnop",
            "github_pat_abcdefghijklmnop",
            "xoxb-1234567890-abcdef",
            "sk-proj-abcdefghijklmnop",
            "AKIAIOSFODNN7EXAMPLE",
        ] {
            let output = redact_text(&format!("prefix {token} suffix"));
            assert!(output.redacted, "{token} must be redacted");
            assert!(
                !output.text.contains(token),
                "redacted output still contains {token:?}: {}",
                output.text
            );
        }
    }

    #[test]
    fn ordinary_sudo_guidance_is_not_mistaken_for_a_secret() {
        let text = "deploy ALL=(root) NOPASSWD: /usr/bin/systemctl restart -- nginx.service";
        let output = redact_text(text);
        assert!(!output.redacted);
        assert_eq!(output.text, text);
    }

    #[test]
    fn preserves_a_missing_trailing_newline() {
        assert_eq!(redact_text("plain").text, "plain");
        assert_eq!(redact_text("plain\n").text, "plain\n");
    }
}
