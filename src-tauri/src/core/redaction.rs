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

/// Redacts generic secrets and SQL password literals.
///
/// SQL history, snippets, and live query text need the generic secret
/// redaction plus literals that are only secret because of SQL syntax, such as
/// `CREATE USER ... IDENTIFIED BY 'secret'` and
/// `ALTER ROLE ... PASSWORD 'secret'`. Those literals are not assignments and
/// would otherwise survive the generic line-oriented redactor.
pub fn redact_sql(input: &str) -> RedactedText {
    let redacted = redact_text(input);
    let sql_redacted = redact_sql_secret_literals(&redacted.text);
    RedactedText {
        redacted: redacted.redacted || sql_redacted != redacted.text,
        text: sql_redacted,
    }
}

/// Redacts text and SQL password literals, then bounds it to `max_chars`.
pub fn sanitize_sql_bounded(input: &str, max_chars: usize) -> String {
    truncate_chars(redact_sql(input).text.trim_end(), max_chars)
}

fn redact_sql_secret_literals(input: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0;

    while cursor < input.len() {
        let Some((keyword_start, keyword_len)) = find_next_sql_secret_keyword(&lower, cursor)
        else {
            output.push_str(&input[cursor..]);
            break;
        };

        let after_keyword = keyword_start + keyword_len;
        output.push_str(&input[cursor..after_keyword]);

        let keyword = &lower[keyword_start..after_keyword];
        let Some((literal_start, literal_end, quote)) =
            sql_secret_literal_after(input, after_keyword, keyword)
        else {
            cursor = after_keyword;
            continue;
        };

        output.push_str(&input[after_keyword..literal_start]);
        output.push(quote);
        output.push_str("<redacted>");
        output.push(quote);
        cursor = literal_end + quote.len_utf8();
    }

    output
}

fn find_next_sql_secret_keyword(lower: &str, from: usize) -> Option<(usize, usize)> {
    let mut best: Option<(usize, usize)> = None;
    for keyword in ["password", "identified"] {
        if let Some(index) = find_ascii_word(lower, keyword, from)
            && best.is_none_or(|(best_index, _)| index < best_index)
        {
            best = Some((index, keyword.len()));
        }
    }
    best
}

fn find_ascii_word(haystack: &str, needle: &str, from: usize) -> Option<usize> {
    let mut search_from = from;
    while search_from < haystack.len() {
        let relative = haystack[search_from..].find(needle)?;
        let index = search_from + relative;
        let before_is_word = haystack[..index]
            .chars()
            .next_back()
            .is_some_and(|character| character.is_ascii_alphanumeric() || character == '_');
        let after_index = index + needle.len();
        let after_is_word = haystack[after_index..]
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphanumeric() || character == '_');
        if !before_is_word && !after_is_word {
            return Some(index);
        }
        search_from = after_index;
    }
    None
}

/// Returns the opening quote index, closing quote index, and quote character
/// for a password literal that follows `password` or `identified ... by`.
fn sql_secret_literal_after(
    input: &str,
    start: usize,
    keyword: &str,
) -> Option<(usize, usize, char)> {
    let mut index = skip_ascii_whitespace(input, start);

    if keyword == "password" {
        if input
            .as_bytes()
            .get(index)
            .is_some_and(|byte| *byte == b'=' || *byte == b':')
        {
            index = skip_ascii_whitespace(input, index + 1);
        } else if let Some(after_for) = consume_ascii_word(input, index, "for") {
            // MySQL: SET PASSWORD FOR 'user'@'host' = 'password'. The account
            // expression is not secret; skip it before looking for the literal.
            index = skip_sql_account_expression(input, after_for);
            index = skip_ascii_whitespace(input, index);
            if input
                .as_bytes()
                .get(index)
                .is_some_and(|byte| *byte == b'=')
            {
                index = skip_ascii_whitespace(input, index + 1);
            } else {
                let after_to = consume_ascii_word(input, index, "to")?;
                index = skip_ascii_whitespace(input, after_to);
            }
        } else if input
            .as_bytes()
            .get(index)
            .is_some_and(|byte| *byte == b'(')
        {
            // MySQL: PASSWORD('password') function call.
            index = skip_ascii_whitespace(input, index + 1);
        }
    } else {
        // MySQL: IDENTIFIED [WITH auth_plugin] BY 'password' or
        // IDENTIFIED WITH auth_plugin AS 'auth_string'.
        if let Some(after_with) = consume_ascii_word(input, index, "with") {
            index = skip_ascii_whitespace(input, after_with);
            index = skip_auth_plugin(input, index);
            index = skip_ascii_whitespace(input, index);
        }
        if let Some(after_by) = consume_ascii_word(input, index, "by") {
            index = skip_ascii_whitespace(input, after_by);
            // Older MySQL syntax: IDENTIFIED BY PASSWORD 'hash'.
            if let Some(after_password) = consume_ascii_word(input, index, "password") {
                index = skip_ascii_whitespace(input, after_password);
            }
        } else {
            let after_as = consume_ascii_word(input, index, "as")?;
            index = skip_ascii_whitespace(input, after_as);
        }
    }

    quoted_literal_at(input, index)
}

fn quoted_literal_at(input: &str, index: usize) -> Option<(usize, usize, char)> {
    let quote = match input[index..].chars().next()? {
        '\'' => '\'',
        '"' => '"',
        _ => return None,
    };
    let literal_end = closing_quote_index(input, index, quote)?;
    Some((index, literal_end, quote))
}

/// Skips a MySQL account expression such as `'app'@'%'`, `app@localhost`, or
/// `CURRENT_USER` without treating either part as the password literal.
fn skip_sql_account_expression(input: &str, start: usize) -> usize {
    let mut index = skip_ascii_whitespace(input, start);
    index = skip_sql_account_part(input, index);
    index = skip_ascii_whitespace(input, index);
    if input
        .as_bytes()
        .get(index)
        .is_some_and(|byte| *byte == b'@')
    {
        index = skip_ascii_whitespace(input, index + 1);
        index = skip_sql_account_part(input, index);
    }
    index
}

fn skip_sql_account_part(input: &str, start: usize) -> usize {
    let Some(first) = input[start..].chars().next() else {
        return start;
    };
    if matches!(first, '\'' | '"') {
        return closing_quote_index(input, start, first)
            .map(|index| index + first.len_utf8())
            .unwrap_or(start);
    }

    let mut index = start;
    for character in input[start..].chars() {
        if character.is_whitespace() || matches!(character, '@' | '=' | ';') {
            break;
        }
        index += character.len_utf8();
    }
    index
}

fn skip_ascii_whitespace(input: &str, start: usize) -> usize {
    let mut index = start;
    for character in input[start..].chars() {
        if !character.is_ascii_whitespace() {
            break;
        }
        index += character.len_utf8();
    }
    index
}

fn consume_ascii_word(input: &str, start: usize, word: &str) -> Option<usize> {
    let end = start + word.len();
    if end > input.len() {
        return None;
    }
    let candidate = &input[start..end];
    if !candidate.eq_ignore_ascii_case(word) {
        return None;
    }
    if input[end..]
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return None;
    }
    Some(end)
}

fn skip_auth_plugin(input: &str, start: usize) -> usize {
    let Some(first) = input[start..].chars().next() else {
        return start;
    };
    if matches!(first, '\'' | '"') {
        return closing_quote_index(input, start, first)
            .map(|index| index + first.len_utf8())
            .unwrap_or(start);
    }

    let mut index = start;
    for character in input[start..].chars() {
        if !(character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')) {
            break;
        }
        index += character.len_utf8();
    }
    index
}

fn closing_quote_index(input: &str, start: usize, quote: char) -> Option<usize> {
    let mut characters = input[start + quote.len_utf8()..].char_indices();
    while let Some((offset, character)) = characters.next() {
        if character == '\\' {
            characters.next();
            continue;
        }
        if character == quote {
            if characters
                .clone()
                .next()
                .is_some_and(|(_, next)| next == quote)
            {
                characters.next();
                continue;
            }
            return Some(start + quote.len_utf8() + offset);
        }
    }
    None
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
        } else {
            let mut line = strip_line_controls(line);
            // A single line can contain several independent secrets, for
            // example `DB_PASSWORD=... Authorization: Bearer ...`. Apply every
            // pattern instead of stopping at the first match.
            for _ in 0..4 {
                let previous = line.clone();
                if let Some(value) = redact_bearer_token(&line) {
                    redacted = true;
                    line = value;
                }
                if let Some(value) = redact_url_password(&line) {
                    redacted = true;
                    line = value;
                }
                if let Some(value) = redact_secret_assignment(&line) {
                    redacted = true;
                    line = value;
                }
                if let Some(value) = redact_token_shape(&line) {
                    redacted = true;
                    line = value;
                }
                if line == previous {
                    break;
                }
            }
            output.push_str(&line);
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
        let trimmed_value = value.trim();
        if trimmed_value.is_empty()
            || trimmed_value == "<redacted>"
            || trimmed_value.eq_ignore_ascii_case("Bearer <redacted>")
            || !is_secret_key(key)
        {
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
    fn redacts_multiple_secret_patterns_on_one_line() {
        let output = redact_text(
            "authentication failed for DB_PASSWORD=fixture-password with \
             Authorization: Bearer fixture-token",
        );

        assert!(output.redacted);
        assert!(!output.text.contains("fixture-password"));
        assert!(!output.text.contains("fixture-token"));
        assert!(output.text.contains("DB_PASSWORD=<redacted>"));
    }

    #[test]
    fn redacts_sql_password_literals_without_touching_columns() {
        let sanitized = sanitize_sql_bounded(
            "CREATE USER 'app'@'%' IDENTIFIED BY 'fixture-password';\n\
             ALTER ROLE app WITH PASSWORD 'fixture-role-password';\n\
             CREATE USER 'plugin'@'%' IDENTIFIED WITH caching_sha2_password BY 'fixture-plugin-password';\n\
             CREATE USER 'hash'@'%' IDENTIFIED WITH caching_sha2_password AS 'fixture-auth-hash';\n\
             SET PASSWORD = 'fixture-set-password';\n\
             SET PASSWORD FOR 'app'@'%' = 'fixture-set-for-password';\n\
             SELECT PASSWORD('fixture-password-function');\n\
             SELECT password, token FROM users WHERE active = true;\n",
            MAX,
        );

        for secret in [
            "fixture-password",
            "fixture-role-password",
            "fixture-plugin-password",
            "fixture-auth-hash",
            "fixture-set-password",
            "fixture-set-for-password",
            "fixture-password-function",
        ] {
            assert!(
                !sanitized.contains(secret),
                "sanitized SQL still contains {secret:?}: {sanitized}"
            );
        }
        assert!(sanitized.contains("IDENTIFIED BY '<redacted>'"));
        assert!(sanitized.contains("PASSWORD '<redacted>'"));
        assert!(sanitized.contains("SET PASSWORD ="));
        // A column named `password` is ordinary SQL and stays readable.
        assert!(sanitized.contains("SELECT password, token FROM users"));
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
