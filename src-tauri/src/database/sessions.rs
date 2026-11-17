//! Database session and running-query monitoring.
//!
//! Listing is a bounded metadata read. Query text is redacted before it is
//! serialized, because `pg_stat_activity` and `information_schema.PROCESSLIST`
//! can expose statements that contain passwords or other secrets.
//!
//! Cancelling a query or terminating a session is destructive: it interrupts
//! another account's work and may roll back a transaction. Every action
//! therefore requires an exact typed confirmation phrase, is rejected on a
//! read-only profile, and is reported as successful only after the session or
//! query is re-read and its absence is verified.

use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::{core::AppError, core::redaction::redact_sql, storage::DatabaseEngine};

use super::{
    capabilities::{DatabaseAction as CapabilityAction, DatabaseCapabilities},
    errors::DatabaseError,
    pagination::{Page, PageRequest},
    service::DatabaseConnectionRegistry,
    traits::{DatabaseAdapter, MetadataValue},
};

/// Longest query preview kept for one session. Longer statements are
/// truncated and the truncation is disclosed to the UI.
pub const MAX_SESSION_QUERY_CHARACTERS: usize = 4_096;
/// Longest accepted account or database name in a confirmation request.
pub const MAX_SESSION_IDENTITY_CHARACTERS: usize = 255;
/// Longest accepted typed confirmation phrase.
pub const MAX_SESSION_CONFIRMATION_CHARACTERS: usize = 128;

/// A session or running-query action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionActionKind {
    CancelQuery,
    TerminateSession,
}

impl SessionActionKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CancelQuery => "cancel_query",
            Self::TerminateSession => "terminate_session",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::CancelQuery => "Cancel query",
            Self::TerminateSession => "Terminate session",
        }
    }

    pub const fn capability(self) -> CapabilityAction {
        match self {
            Self::CancelQuery => CapabilityAction::CancelQuery,
            Self::TerminateSession => CapabilityAction::KillSession,
        }
    }

    /// The exact phrase the operator must type before the action is accepted.
    /// The phrase names both the action and the numeric session id, so a
    /// confirmation cannot be reused for a different target.
    pub fn confirmation_phrase(self, session_id: i64) -> String {
        match self {
            Self::CancelQuery => format!("cancel query {session_id}"),
            Self::TerminateSession => format!("terminate session {session_id}"),
        }
    }
}

/// Redacted query preview attached to a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionQuery {
    pub text: String,
    /// True when a secret pattern or SQL password literal was replaced.
    pub redacted: bool,
    /// True when the query exceeded the preview bound.
    pub truncated: bool,
    /// Server-reported query start time where the engine exposes one.
    pub started_at: MetadataValue<String>,
    /// Measured query runtime where the engine exposes one.
    pub elapsed_ms: MetadataValue<u64>,
}

impl SessionQuery {
    /// Redacts and bounds one engine-provided query string. The raw text never
    /// crosses this constructor.
    pub fn from_raw(
        text: &str,
        started_at: MetadataValue<String>,
        elapsed_ms: MetadataValue<u64>,
    ) -> Self {
        let trimmed = text.trim();
        let redacted = redact_sql(trimmed);
        let truncated = redacted.text.chars().count() > MAX_SESSION_QUERY_CHARACTERS;
        let text =
            crate::core::redaction::sanitize_sql_bounded(trimmed, MAX_SESSION_QUERY_CHARACTERS);
        Self {
            text,
            redacted: redacted.redacted,
            truncated,
            started_at,
            elapsed_ms,
        }
    }
}

/// One active database session. Every optional detail is explicit about
/// whether the selected engine exposes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    /// Engine process/connection id.
    pub id: i64,
    pub user: String,
    pub host: MetadataValue<String>,
    pub database: MetadataValue<String>,
    pub application_name: MetadataValue<String>,
    pub command: MetadataValue<String>,
    pub state: MetadataValue<String>,
    pub wait_event: MetadataValue<String>,
    pub backend_type: MetadataValue<String>,
    pub connected_at: MetadataValue<String>,
    /// True for ServerDesk's own connection. The UI disables actions and the
    /// service refuses to target it.
    pub is_current: bool,
    /// `Unavailable` means the session is idle, the engine does not expose
    /// query text to this account, or the query text is empty.
    pub query: MetadataValue<SessionQuery>,
}

impl SessionSummary {
    pub fn is_running(&self) -> bool {
        self.query.is_available()
    }

    pub fn database_label(&self) -> Option<&str> {
        self.database.value().map(String::as_str)
    }

    pub fn query_elapsed_ms(&self) -> Option<u64> {
        self.query
            .value()
            .and_then(|query| query.elapsed_ms.value().copied())
    }
}

/// Untrusted request from React. The confirmation phrase is checked before the
/// target is read, and the expected account is checked again after the target
/// is re-read so a reused numeric id cannot redirect the action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionActionRequest {
    pub session_id: i64,
    pub action: SessionActionKind,
    pub expected_user: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_database: Option<String>,
    pub confirmation: String,
}

/// A locally validated action. Target identity is still checked against live
/// session metadata before execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedSessionAction {
    pub session_id: i64,
    pub action: SessionActionKind,
    pub expected_user: String,
    pub expected_database: Option<String>,
}

impl SessionActionRequest {
    /// Validates local fields and the exact confirmation phrase.
    pub fn validate(&self) -> Result<ValidatedSessionAction, DatabaseError> {
        if self.session_id <= 0 {
            return Err(DatabaseError::validation(
                "A session id must be a positive number.",
            ));
        }
        let expected_user = validate_identity("account name", &self.expected_user)?;
        let expected_database = self
            .expected_database
            .as_deref()
            .map(|database| validate_identity("database name", database))
            .transpose()?;

        if self.confirmation.chars().count() > MAX_SESSION_CONFIRMATION_CHARACTERS {
            return Err(DatabaseError::validation(format!(
                "The session confirmation cannot exceed {MAX_SESSION_CONFIRMATION_CHARACTERS} \
                 characters."
            )));
        }

        let expected_phrase = self.action.confirmation_phrase(self.session_id);
        if self.confirmation.trim() != expected_phrase {
            return Err(DatabaseError::cancelled(format!(
                "The {} for session {} was cancelled because the confirmation phrase did not \
                 match.",
                self.action.as_str(),
                self.session_id
            ))
            .with_details(format!(
                "Type \"{expected_phrase}\" exactly before the action can run. No request was \
                 sent to the database."
            )));
        }

        Ok(ValidatedSessionAction {
            session_id: self.session_id,
            action: self.action,
            expected_user,
            expected_database,
        })
    }
}

fn validate_identity(label: &str, value: &str) -> Result<String, DatabaseError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(DatabaseError::validation(format!(
            "A session action requires a non-empty {label}."
        )));
    }
    if trimmed.chars().count() > MAX_SESSION_IDENTITY_CHARACTERS {
        return Err(DatabaseError::validation(format!(
            "The session {label} cannot exceed {MAX_SESSION_IDENTITY_CHARACTERS} characters."
        )));
    }
    if trimmed.chars().any(char::is_control) {
        return Err(DatabaseError::validation(format!(
            "The session {label} cannot contain control characters."
        )));
    }
    Ok(trimmed.to_string())
}

/// Verified outcome of one session action. Query text is deliberately absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionActionResult {
    pub action: SessionActionKind,
    pub engine: DatabaseEngine,
    pub session_id: i64,
    pub user: String,
    pub database: MetadataValue<String>,
    /// True when the server accepted the cancellation or termination signal.
    pub request_accepted: bool,
    pub verified: bool,
    pub verification: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_error: Option<String>,
    pub duration_ms: u64,
}

/// Stateless session service used by the database session commands.
pub struct DatabaseSessionService<'a> {
    connections: &'a DatabaseConnectionRegistry,
}

impl<'a> DatabaseSessionService<'a> {
    pub fn new(connections: &'a DatabaseConnectionRegistry) -> Self {
        Self { connections }
    }

    pub async fn list(
        &self,
        profile_id: &str,
        page: PageRequest,
        running_only: bool,
    ) -> Result<Page<SessionSummary>, AppError> {
        let adapter = self.connections.adapter(profile_id).await?;
        let adapter = adapter.lock().await;
        adapter
            .require_capability(CapabilityAction::BrowseSessions)
            .map_err(AppError::from)?;
        adapter
            .list_sessions(page, running_only)
            .await
            .map_err(AppError::from)
    }

    pub async fn execute(
        &self,
        profile_id: &str,
        request: SessionActionRequest,
    ) -> Result<SessionActionResult, AppError> {
        let adapter = self.connections.adapter(profile_id).await?;
        let mut adapter = adapter.lock().await;
        execute_session_action_with_adapter(adapter.as_mut(), request)
            .await
            .map_err(AppError::from)
    }
}

/// Executes one validated session action and verifies the outcome.
pub async fn execute_session_action_with_adapter(
    adapter: &mut dyn DatabaseAdapter,
    request: SessionActionRequest,
) -> Result<SessionActionResult, DatabaseError> {
    adapter.require_connection()?;
    let action = request.validate()?;
    adapter.require_capability(action.action.capability())?;

    if adapter.is_read_only() {
        return Err(DatabaseError::permission(
            "The database connection profile is read-only; cancelling queries and terminating \
             sessions require a writable administrative connection.",
        )
        .with_scope("session", action.session_id.to_string()));
    }

    let target = adapter
        .session_summary(action.session_id)
        .await?
        .ok_or_else(|| {
            DatabaseError::validation(
                "The selected session is no longer active. Refresh the session list before \
                 trying again.",
            )
        })?;

    if target.user != action.expected_user {
        return Err(DatabaseError::validation(
            "The selected session now belongs to a different account. Refresh the session list \
             before trying again.",
        )
        .with_scope("session", action.session_id.to_string()));
    }
    if let Some(expected_database) = action.expected_database.as_deref()
        && target
            .database_label()
            .is_some_and(|database| database != expected_database)
    {
        return Err(DatabaseError::validation(
            "The selected session is now connected to a different database. Refresh the session \
             list before trying again.",
        )
        .with_scope("session", action.session_id.to_string()));
    }
    if target.is_current {
        return Err(DatabaseError::validation(
            "ServerDesk will not cancel or terminate its own database connection.",
        )
        .with_scope("session", action.session_id.to_string()));
    }
    if action.action == SessionActionKind::CancelQuery && !target.is_running() {
        return Err(DatabaseError::validation(
            "The selected session is not running a query. Refresh the session list before trying \
             again.",
        )
        .with_scope("session", action.session_id.to_string()));
    }

    let started = Instant::now();
    let request_accepted = adapter.execute_session_action(&action).await?;
    let verification = verify_session_action(adapter, &action, &target, request_accepted).await;
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

    Ok(SessionActionResult {
        action: action.action,
        engine: adapter.engine(),
        session_id: action.session_id,
        user: target.user,
        database: target.database,
        request_accepted,
        verified: verification.verified,
        verification: verification.summary,
        verification_error: verification.error,
        duration_ms,
    })
}

struct SessionVerification {
    verified: bool,
    summary: String,
    error: Option<String>,
}

async fn verify_session_action(
    adapter: &dyn DatabaseAdapter,
    action: &ValidatedSessionAction,
    target: &SessionSummary,
    request_accepted: bool,
) -> SessionVerification {
    match adapter.session_summary(action.session_id).await {
        Ok(None) => SessionVerification {
            verified: true,
            summary: match action.action {
                SessionActionKind::CancelQuery => format!(
                    "Session {} is no longer present, so its query is no longer running. \
                     Cancellation verified.",
                    action.session_id
                ),
                SessionActionKind::TerminateSession => format!(
                    "Session {} is no longer present. Termination verified.",
                    action.session_id
                ),
            },
            error: None,
        },
        Ok(Some(current)) => match action.action {
            SessionActionKind::CancelQuery => {
                let target_text = target.query.value().map(|query| query.text.as_str());
                match current.query.value() {
                    None => SessionVerification {
                        verified: true,
                        summary: format!(
                            "Session {} remains open but no query is running. Cancellation \
                             verified.",
                            action.session_id
                        ),
                        error: None,
                    },
                    Some(query) if Some(query.text.as_str()) != target_text => {
                        SessionVerification {
                            verified: true,
                            summary: format!(
                                "Session {} now reports a different query, so the selected query \
                                 is no longer running. Cancellation verified.",
                                action.session_id
                            ),
                            error: None,
                        }
                    }
                    Some(_) => SessionVerification {
                        verified: false,
                        summary: format!(
                            "The cancellation request finished, but session {} still reports the \
                             same running query.",
                            action.session_id
                        ),
                        error: Some(unaccepted_note(
                            request_accepted,
                            "The query was still running when the session list was re-read.",
                        )),
                    },
                }
            }
            SessionActionKind::TerminateSession => SessionVerification {
                verified: false,
                summary: format!(
                    "The termination request finished, but session {} is still present.",
                    action.session_id
                ),
                error: Some(unaccepted_note(
                    request_accepted,
                    "The session was still present when the session list was re-read.",
                )),
            },
        },
        Err(error) => SessionVerification {
            verified: false,
            summary: format!(
                "The {} request finished, but session {} could not be re-read to verify the \
                 outcome.",
                action.action.as_str(),
                action.session_id
            ),
            error: Some(error.message),
        },
    }
}

fn unaccepted_note(request_accepted: bool, fallback: &str) -> String {
    if request_accepted {
        fallback.to_string()
    } else {
        format!("The server did not accept the request. {fallback}")
    }
}

/// Builds the capability decision used by the session UI. The backend repeats
/// the same check before every action.
pub fn session_capabilities(capabilities: &DatabaseCapabilities) -> (bool, bool, bool) {
    (
        capabilities.supports(CapabilityAction::BrowseSessions),
        capabilities.supports(CapabilityAction::CancelQuery),
        capabilities.supports(CapabilityAction::KillSession),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::FakeDatabaseAdapter;
    use crate::database::errors::DatabaseErrorKind;
    use crate::database::traits::MetadataValue;
    use crate::storage::DatabaseEngine;

    fn request(action: SessionActionKind, id: i64) -> SessionActionRequest {
        SessionActionRequest {
            session_id: id,
            action,
            expected_user: "app".to_string(),
            expected_database: Some("app".to_string()),
            confirmation: action.confirmation_phrase(id),
        }
    }

    fn connected(engine: DatabaseEngine) -> FakeDatabaseAdapter {
        let mut adapter = FakeDatabaseAdapter::new(engine, Some("16.2.0"));
        adapter.connect_for_test();
        adapter
    }

    #[test]
    fn confirmation_phrase_names_the_action_and_session() {
        assert_eq!(
            SessionActionKind::CancelQuery.confirmation_phrase(42),
            "cancel query 42"
        );
        assert_eq!(
            SessionActionKind::TerminateSession.confirmation_phrase(42),
            "terminate session 42"
        );
    }

    #[test]
    fn a_mismatched_confirmation_is_cancelled_before_execution() {
        let mut request = request(SessionActionKind::TerminateSession, 501);
        request.confirmation = "terminate session 502".to_string();

        let error = request.validate().expect_err("confirmation must match");
        assert_eq!(error.kind, DatabaseErrorKind::Cancelled);
        assert!(error.message.contains("confirmation phrase"));
        assert!(
            error
                .details
                .as_deref()
                .is_some_and(|details| details.contains("terminate session 501"))
        );
    }

    #[test]
    fn query_text_is_redacted_before_it_can_be_serialized() {
        let query = SessionQuery::from_raw(
            "ALTER ROLE reporting WITH PASSWORD 'fixture-role-secret'",
            MetadataValue::unavailable(),
            MetadataValue::available(1_200),
        );
        let json = serde_json::to_string(&query).expect("query json");

        assert!(query.redacted);
        assert!(!json.contains("fixture-role-secret"));
        assert!(json.contains("<redacted>"));
        assert!(!query.truncated);
    }

    #[test]
    fn long_query_text_is_truncated_and_disclosed() {
        let raw = "SELECT ".to_string() + &"x".repeat(MAX_SESSION_QUERY_CHARACTERS + 20);
        let query = SessionQuery::from_raw(
            &raw,
            MetadataValue::unavailable(),
            MetadataValue::unavailable(),
        );

        assert!(query.truncated);
        assert!(query.text.chars().count() <= MAX_SESSION_QUERY_CHARACTERS);
    }

    #[tokio::test]
    async fn read_only_profiles_deny_session_actions() {
        let mut adapter = connected(DatabaseEngine::Postgresql);
        adapter.set_read_only_for_test(true);

        let error = execute_session_action_with_adapter(
            &mut adapter,
            request(SessionActionKind::TerminateSession, 501),
        )
        .await
        .expect_err("read-only profile");

        assert_eq!(error.kind, DatabaseErrorKind::Permission);
        assert_eq!(adapter.session_execution_count(), 0);
    }

    #[tokio::test]
    async fn unsupported_session_capability_is_rejected_before_execution() {
        let mut adapter = FakeDatabaseAdapter::new(DatabaseEngine::Postgresql, None);
        adapter.connect_for_test();

        let error = execute_session_action_with_adapter(
            &mut adapter,
            request(SessionActionKind::TerminateSession, 501),
        )
        .await
        .expect_err("session capability required");

        assert_eq!(error.kind, DatabaseErrorKind::Capability);
        assert_eq!(adapter.session_execution_count(), 0);
    }

    #[tokio::test]
    async fn termination_is_verified_only_after_the_session_is_gone() {
        let mut adapter = connected(DatabaseEngine::Postgresql);
        let result = execute_session_action_with_adapter(
            &mut adapter,
            request(SessionActionKind::TerminateSession, 501),
        )
        .await
        .expect("termination");

        assert!(result.verified);
        assert!(result.request_accepted);
        assert!(result.verification.contains("Termination verified"));
        assert_eq!(adapter.session_execution_count(), 1);
    }

    #[tokio::test]
    async fn a_skipped_termination_is_reported_as_unverified() {
        let mut adapter = connected(DatabaseEngine::Postgresql);
        adapter = adapter.with_skipped_session_execution();

        let result = execute_session_action_with_adapter(
            &mut adapter,
            request(SessionActionKind::TerminateSession, 501),
        )
        .await
        .expect("termination attempt");

        assert!(!result.verified);
        assert!(result.request_accepted);
        assert!(result.verification.contains("still present"));
        assert!(result.verification_error.is_some());
    }

    #[tokio::test]
    async fn cancel_is_verified_when_the_query_is_gone() {
        let mut adapter = connected(DatabaseEngine::Postgresql);
        let result = execute_session_action_with_adapter(
            &mut adapter,
            request(SessionActionKind::CancelQuery, 501),
        )
        .await
        .expect("cancel");

        assert!(result.verified);
        assert!(result.verification.contains("Cancellation verified"));
    }

    #[tokio::test]
    async fn a_session_belonging_to_another_account_is_not_targeted() {
        let mut adapter = connected(DatabaseEngine::Postgresql);
        let mut request = request(SessionActionKind::TerminateSession, 501);
        request.expected_user = "someone_else".to_string();

        let error = execute_session_action_with_adapter(&mut adapter, request)
            .await
            .expect_err("account mismatch");

        assert_eq!(error.kind, DatabaseErrorKind::Validation);
        assert_eq!(adapter.session_execution_count(), 0);
    }

    #[tokio::test]
    async fn the_current_connection_cannot_target_itself() {
        let mut adapter = connected(DatabaseEngine::Postgresql);
        let error = execute_session_action_with_adapter(
            &mut adapter,
            request(SessionActionKind::TerminateSession, 503),
        )
        .await
        .expect_err("current session");

        assert_eq!(error.kind, DatabaseErrorKind::Validation);
        assert!(error.message.contains("own database connection"));
        assert_eq!(adapter.session_execution_count(), 0);
    }

    #[tokio::test]
    async fn permission_denial_from_the_server_is_typed_and_unverified() {
        let mut adapter = connected(DatabaseEngine::Postgresql);
        adapter = adapter.with_session_execution_error(DatabaseError::permission(
            "The database account does not have permission for this operation.",
        ));

        let error = execute_session_action_with_adapter(
            &mut adapter,
            request(SessionActionKind::TerminateSession, 501),
        )
        .await
        .expect_err("permission denied");

        assert_eq!(error.kind, DatabaseErrorKind::Permission);
        assert_eq!(adapter.session_execution_count(), 1);
    }

    #[tokio::test]
    async fn listing_sessions_marks_unsupported_fields_unavailable() {
        let adapter = connected(DatabaseEngine::Mysql);
        let page = adapter
            .list_sessions(PageRequest::clamped(Some(100), Some(0)), false)
            .await
            .expect("sessions");

        let session = page
            .items
            .iter()
            .find(|session| session.id == 101)
            .expect("fixture session");
        assert!(!session.connected_at.is_available());
        assert!(!session.application_name.is_available());
        assert!(!session.wait_event.is_available());
        assert!(!session.backend_type.is_available());
        assert!(session.host.is_available());
        assert!(session.query.is_available());
    }

    #[tokio::test]
    async fn running_only_filters_idle_sessions() {
        let adapter = connected(DatabaseEngine::Postgresql);
        let page = adapter
            .list_sessions(PageRequest::clamped(Some(100), Some(0)), true)
            .await
            .expect("sessions");

        assert!(!page.items.is_empty());
        assert!(page.items.iter().all(SessionSummary::is_running));
        assert!(!page.items.iter().any(|session| session.id == 502));
    }
}
