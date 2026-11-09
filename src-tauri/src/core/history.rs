//! Sanitized operation-history recording.
//!
//! History stores only bounded metadata and a short, sanitized output summary.
//! Raw command output, terminal keystrokes, environment files, passwords,
//! private keys, passphrases, and tokens are never persisted. The sanitizer is
//! intentionally defense in depth: callers must also avoid passing raw output.

use crate::{
    core::AppError,
    storage::{Database, NewOperationRecord, OperationRecord, OperationResult},
};

/// Maximum characters kept in the short output summary.
pub const MAX_OUTPUT_SUMMARY_CHARS: usize = 360;
/// Maximum characters kept in the human-readable operation summary.
pub const MAX_SAFE_SUMMARY_CHARS: usize = 500;
/// Maximum number of output lines considered for one summary.
const MAX_SUMMARY_LINES: usize = 3;
const MAX_OPERATION_CHARS: usize = 120;
const MAX_PATH_CHARS: usize = 4_096;

/// Current Unix time in seconds, matching the history schema.
pub fn now_unix_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or_default()
}

/// Records one sanitized operation row.
///
/// Every text field is sanitized again here, even when the caller already
/// built a safe summary. Storage also bounds the output summary length so a
/// large command result can never be stored wholesale.
pub fn record_operation(
    database: &Database,
    input: &NewOperationRecord,
) -> Result<OperationRecord, AppError> {
    let mut input = input.clone();
    input.operation = bounded_single_line(&input.operation, MAX_OPERATION_CHARS);
    input.safe_summary = sanitize_text_bounded(&input.safe_summary, MAX_SAFE_SUMMARY_CHARS);
    input.path = input
        .path
        .as_deref()
        .map(|path| sanitize_text_bounded(path, MAX_PATH_CHARS))
        .filter(|path| !path.trim().is_empty());
    input.output_summary = input
        .output_summary
        .as_deref()
        .map(|summary| sanitize_text_bounded(summary, MAX_OUTPUT_SUMMARY_CHARS))
        .filter(|summary| !summary.trim().is_empty());

    database.record_operation(&input).map_err(AppError::from)
}

/// Maps a command exit status to a history result.
pub fn result_from_exit_code(exit_code: Option<u32>) -> OperationResult {
    match exit_code {
        Some(0) => OperationResult::Success,
        Some(_) => OperationResult::Failure,
        None => OperationResult::Unknown,
    }
}

/// Builds a short, sanitized summary from stdout and stderr.
///
/// The summary is limited to a few non-empty lines and a hard character bound.
/// It is never the complete command output.
pub fn summarize_output(stdout: &str, stderr: &str) -> Option<String> {
    let combined = if stderr.trim().is_empty() {
        stdout.to_string()
    } else if stdout.trim().is_empty() {
        stderr.to_string()
    } else {
        format!("{stderr}\n{stdout}")
    };
    let sanitized = sanitize_text_bounded(&combined, MAX_OUTPUT_SUMMARY_CHARS);
    let lines = sanitized
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(MAX_SUMMARY_LINES)
        .collect::<Vec<_>>();

    if lines.is_empty() {
        None
    } else {
        Some(bounded_single_line(
            &lines.join(" | "),
            MAX_OUTPUT_SUMMARY_CHARS,
        ))
    }
}

/// Builds a short, sanitized failure summary from an application error.
pub fn summarize_error(error: &AppError) -> Option<String> {
    let detail = error
        .details
        .as_deref()
        .and_then(|details| details.lines().map(str::trim).find(|line| !line.is_empty()));
    let text = match detail {
        Some(detail) => format!("{} {detail}", error.message),
        None => error.message.clone(),
    };
    let summary = sanitize_text_bounded(&text, MAX_OUTPUT_SUMMARY_CHARS);
    if summary.trim().is_empty() {
        None
    } else {
        Some(summary)
    }
}

/// Sanitizes text line by line and bounds the total number of characters.
pub fn sanitize_text_bounded(input: &str, max_chars: usize) -> String {
    let stripped = strip_ansi(input);
    let mut output = String::with_capacity(stripped.len().min(max_chars.saturating_mul(4)));
    let mut in_private_key = false;

    for line in stripped.lines() {
        if in_private_key {
            if is_private_key_end(line) {
                in_private_key = false;
            }
            continue;
        }

        let sanitized = if is_private_key_begin(line) {
            in_private_key = !is_private_key_end(line);
            "[redacted private key]".to_string()
        } else if looks_like_dotenv_line(line) {
            "[redacted environment line]".to_string()
        } else if let Some(redacted) = redact_bearer_token(line) {
            redacted
        } else if let Some(redacted) = redact_url_password(line) {
            redacted
        } else if let Some(redacted) = redact_secret_assignment(line) {
            redacted
        } else if let Some(redacted) = redact_token_shape(line) {
            redacted
        } else {
            strip_line_controls(line)
        };

        if output
            .chars()
            .count()
            .saturating_add(sanitized.chars().count())
            > max_chars
        {
            let remaining = max_chars.saturating_sub(output.chars().count());
            if remaining > 0 {
                output.push_str(&truncate_chars(&sanitized, remaining.saturating_sub(1)));
                output.push('…');
            }
            break;
        }

        output.push_str(&sanitized);
        output.push('\n');
    }

    if output.is_empty() && !stripped.is_empty() {
        output = truncate_chars(&strip_line_controls(&stripped), max_chars);
    }

    output.trim_end().to_string()
}

fn bounded_single_line(input: &str, max_chars: usize) -> String {
    let single_line = input.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_chars(&sanitize_text_bounded(&single_line, max_chars), max_chars)
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
/// Raw environment files must never be stored in history.
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
    let (index, separator) = line
        .char_indices()
        .find(|(_, character)| matches!(character, '=' | ':'))?;
    let key = line[..index]
        .trim()
        .trim_matches(|character| matches!(character, '"' | '\'' | '{' | '[' | ','));
    let value = &line[index + separator.len_utf8()..];
    if key.is_empty() || value.trim().is_empty() || !is_secret_key(key) {
        return None;
    }

    Some(format!(
        "{}<redacted>",
        &line[..index + separator.len_utf8()]
    ))
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

    #[test]
    fn redacts_assignments_bearer_tokens_and_credentials_in_urls() {
        let sanitized = sanitize_text_bounded(
            "DB_PASSWORD=hunter2\n\
             APP_KEY=base64:super-secret\n\
             passphrase: correct horse battery staple\n\
             Authorization: Bearer abc123token\n\
             DATABASE_URL=mysql://deploy:db-secret@127.0.0.1/app\n\
             api_key=sk-live-1234567890abcdef\n",
            MAX_OUTPUT_SUMMARY_CHARS,
        );

        for secret in [
            "hunter2",
            "base64:super-secret",
            "correct horse battery staple",
            "abc123token",
            "db-secret",
            "sk-live-1234567890abcdef",
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
            MAX_OUTPUT_SUMMARY_CHARS,
        );

        assert!(!sanitized.contains("super-secret-key-body"));
        assert!(!sanitized.contains("BEGIN OPENSSH"));
        assert!(!sanitized.contains('\u{1b}'));
        assert!(sanitized.contains("[redacted private key]"));
        assert!(sanitized.contains("key follows"));
        assert!(sanitized.contains("done"));
    }

    #[test]
    fn output_summary_is_short_and_never_full_output() {
        let stdout = (0..100)
            .map(|index| format!("line {index} {}\n", "x".repeat(100)))
            .collect::<String>();
        let summary = summarize_output(&stdout, "").expect("summary");

        assert!(summary.chars().count() <= MAX_OUTPUT_SUMMARY_CHARS);
        assert!(!summary.contains("line 99"));
        assert!(summary.contains("line 0"));
    }

    #[test]
    fn error_summary_is_sanitized_and_bounded() {
        let error = AppError::remote("Remote command failed")
            .with_details("DB_PASSWORD=hunter2\nAuthorization: Bearer abc123\nmore detail");
        let summary = summarize_error(&error).expect("summary");

        assert!(!summary.contains("hunter2"));
        assert!(!summary.contains("abc123"));
        assert!(summary.contains("Remote command failed"));
        assert!(summary.contains("DB_PASSWORD=<redacted>"));
    }
}
