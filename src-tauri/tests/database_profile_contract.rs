//! Contract tests for the database profile command surface.
//!
//! React command names and payload keys are covered by
//! `src/services/databaseProfiles.test.ts`. These tests lock down the Rust
//! DTO shape, validation, typed errors, duplicate handling, and credential
//! lifecycle without exposing secrets.

use serde_json::json;
use serverdesk_lib::{
    core::{
        AppError, ErrorCode,
        credential_index::CredentialIndex,
        credentials::{CredentialStore, MemoryCredentialStore},
        database_credentials::{DatabaseCredentialService, is_database_reference},
        database_profiles::{
            DatabaseConnectionSnapshot, DatabaseConnectionState, DatabaseProfileInput,
            DatabaseProfileService, DatabaseSslMode, MAX_DATABASE_PROFILE_NAME_LENGTH,
        },
    },
    database::{DatabaseError, DatabaseErrorKind},
    storage::{AuthType, Database, DatabaseConnection, DatabaseEngine, NewServer},
};

fn server_input(name: &str) -> NewServer {
    NewServer {
        name: name.to_string(),
        host: "198.51.100.24".to_string(),
        port: 22,
        username: "deploy".to_string(),
        auth_type: AuthType::Password,
        credential_ref: Some("serverdesk:ssh:test-server".to_string()),
        group_name: None,
        tags: vec![],
        notes: None,
    }
}

fn profile_input(server_id: &str, name: &str) -> DatabaseProfileInput {
    DatabaseProfileInput {
        server_id: server_id.to_string(),
        name: name.to_string(),
        engine: DatabaseEngine::Postgresql,
        host: "127.0.0.1".to_string(),
        port: 5432,
        socket: None,
        database_name: Some("app".to_string()),
        username: "app".to_string(),
        read_only: true,
        ssl_mode: Some(DatabaseSslMode::VerifyCa),
        password: Some("fixture-password".to_string()),
    }
}

fn setup() -> (
    tempfile::TempDir,
    Database,
    MemoryCredentialStore,
    CredentialIndex,
    String,
) {
    let directory = tempfile::tempdir().expect("temporary directory");
    let database = Database::in_memory().expect("in-memory database");
    let credentials = MemoryCredentialStore::default();
    let index = CredentialIndex::open(directory.path()).expect("credential index");
    let server = database
        .create_server(&server_input("Production"))
        .expect("create server");
    (directory, database, credentials, index, server.id)
}

#[test]
fn camel_case_payloads_deserialize_and_reject_unknown_engines_and_modes() {
    let input: DatabaseProfileInput = serde_json::from_value(json!({
        "serverId": "server-1",
        "name": "Primary",
        "engine": "postgresql",
        "host": "127.0.0.1",
        "port": 5432,
        "socket": "/var/run/postgresql/.s.PGSQL.5432",
        "databaseName": "app",
        "username": "app",
        "readOnly": true,
        "sslMode": "verify-ca",
        "password": "fixture-password"
    }))
    .expect("camelCase payload");

    let validated = input.validate().expect("valid payload");
    assert!(validated.read_only);
    assert_eq!(validated.ssl_mode, Some(DatabaseSslMode::VerifyCa));
    assert_eq!(validated.server_id, "server-1");

    for invalid in [
        json!({
            "serverId": "server-1", "name": "Primary", "engine": "oracle",
            "host": "127.0.0.1", "port": 1521, "username": "app", "readOnly": false
        }),
        json!({
            "serverId": "server-1", "name": "Primary", "engine": "mysql",
            "host": "127.0.0.1", "port": 3306, "username": "app", "readOnly": false,
            "sslMode": "sometimes"
        }),
        json!({
            "serverId": "server-1", "name": "Primary", "engine": "mysql",
            "host": "127.0.0.1", "port": 3306, "username": "app", "readOnly": "yes"
        }),
    ] {
        assert!(
            serde_json::from_value::<DatabaseProfileInput>(invalid).is_err(),
            "invalid engine, SSL mode, or read-only value must fail closed"
        );
    }
}

#[test]
fn validation_rejects_unsafe_hosts_ports_sockets_names_and_username_values() {
    let cases: Vec<(&str, String)> = vec![
        ("host", String::new()),
        ("host", "db server".to_string()),
        ("host", "db/../../etc".to_string()),
        ("host", "db;rm -rf".to_string()),
        ("host", "x".repeat(254)),
        ("port", "0".to_string()),
        ("socket", "relative.sock".to_string()),
        ("socket", "/tmp/db\n.sock".to_string()),
        ("name", " ".to_string()),
        ("name", "x".repeat(MAX_DATABASE_PROFILE_NAME_LENGTH + 1)),
        ("username", "app user".to_string()),
        ("username", "app\nuser".to_string()),
        ("databaseName", "app\0".to_string()),
    ];

    for (field, value) in cases {
        let mut candidate = profile_input("server-1", "Primary");
        match field {
            "host" => candidate.host = value,
            "port" => candidate.port = 0,
            "socket" => candidate.socket = Some(value),
            "name" => candidate.name = value,
            "username" => candidate.username = value,
            "databaseName" => candidate.database_name = Some(value),
            _ => unreachable!(),
        }

        let error = candidate.validate().expect_err("invalid value");
        assert_eq!(
            error.code,
            ErrorCode::Validation,
            "{field} must be validated"
        );
    }
}

#[test]
fn profile_crud_coordinates_credentials_and_never_serializes_secrets() {
    let (_directory, database, credentials, index, server_id) = setup();
    let service = DatabaseProfileService::new(&database, &credentials, &index);

    let created = service
        .create(profile_input(&server_id, "Primary"))
        .expect("create profile");
    assert_eq!(created.name, "Primary");
    assert!(created.has_credential);
    assert_eq!(created.ssl_mode, Some(DatabaseSslMode::VerifyCa));

    let serialized = serde_json::to_string(&created).expect("profile json");
    assert!(serialized.contains("\"serverId\""));
    assert!(serialized.contains("\"databaseName\""));
    assert!(serialized.contains("\"hasCredential\":true"));
    assert!(!serialized.contains("credentialRef"));
    assert!(!serialized.contains("fixture-password"));

    let stored = database
        .get_database_connection(&created.id)
        .expect("read stored profile")
        .expect("stored profile");
    let reference = stored.credential_ref.clone().expect("credential reference");
    assert!(is_database_reference(&reference));
    assert_eq!(
        credentials
            .get_secret(&reference)
            .expect("stored database password"),
        "fixture-password"
    );
    assert!(
        service
            .list(&server_id)
            .expect("list profiles")
            .iter()
            .all(|profile| !serde_json::to_string(profile)
                .expect("profile json")
                .contains("fixture-password"))
    );
    assert_eq!(
        service
            .get(&created.id)
            .expect("get profile")
            .expect("profile exists")
            .id,
        created.id
    );

    let mut replacement = profile_input(&server_id, "Primary renamed");
    replacement.password = Some("replacement-password".to_string());
    let updated = service
        .update(&created.id, replacement)
        .expect("update profile");
    assert_eq!(updated.name, "Primary renamed");
    assert_eq!(
        credentials
            .get_secret(&reference)
            .expect("replaced database password"),
        "replacement-password"
    );

    let mut unchanged = profile_input(&server_id, "Primary without password change");
    unchanged.password = None;
    let updated = service
        .update(&created.id, unchanged)
        .expect("update without password");
    assert_eq!(updated.name, "Primary without password change");
    assert_eq!(
        credentials.get_secret(&reference).expect("password stays"),
        "replacement-password"
    );

    service.delete(&created.id).expect("delete profile");
    assert!(
        database
            .get_database_connection(&created.id)
            .expect("read deleted profile")
            .is_none()
    );
    assert!(credentials.get_secret(&reference).is_err());
    assert!(
        index
            .references()
            .expect("credential index")
            .iter()
            .all(|indexed| indexed != &reference)
    );
}

#[test]
fn duplicate_names_return_the_conflict_taxonomy_for_create_and_update() {
    let (_directory, database, credentials, index, server_id) = setup();
    let service = DatabaseProfileService::new(&database, &credentials, &index);

    service
        .create(profile_input(&server_id, "Primary"))
        .expect("first profile");
    let error = service
        .create(profile_input(&server_id, " primary "))
        .expect_err("duplicate create");
    assert_eq!(error.code, ErrorCode::Conflict);
    assert!(error.message.contains("name"));

    let second = service
        .create(profile_input(&server_id, "Replica"))
        .expect("second profile");
    let mut rename = profile_input(&server_id, "PRIMARY");
    rename.password = None;
    let error = service
        .update(&second.id, rename)
        .expect_err("duplicate update");
    assert_eq!(error.code, ErrorCode::Conflict);
}

#[test]
fn missing_profiles_and_servers_return_typed_errors() {
    let (_directory, database, credentials, index, _server_id) = setup();
    let service = DatabaseProfileService::new(&database, &credentials, &index);

    let error = service
        .create(profile_input("missing-server", "Primary"))
        .expect_err("missing server");
    assert_eq!(error.code, ErrorCode::NotFound);

    let error = service
        .update(
            "missing-profile",
            profile_input("missing-server", "Primary"),
        )
        .expect_err("missing profile");
    assert_eq!(error.code, ErrorCode::NotFound);

    let error = service
        .delete("missing-profile")
        .expect_err("missing profile");
    assert_eq!(error.code, ErrorCode::NotFound);
}

#[test]
fn connection_snapshots_expose_engine_version_read_only_and_typed_errors() {
    let profile = DatabaseConnection {
        id: "profile-1".to_string(),
        server_id: "server-1".to_string(),
        name: "Primary".to_string(),
        engine: DatabaseEngine::Mysql,
        host: "127.0.0.1".to_string(),
        port: 3306,
        socket: None,
        database_name: Some("app".to_string()),
        username: "app".to_string(),
        credential_ref: Some("serverdesk:db:hidden-reference".to_string()),
        read_only: true,
        ssl_mode: Some("required".to_string()),
        created_at: 1,
        updated_at: 1,
        last_used_at: None,
    };

    let snapshot = DatabaseConnectionSnapshot::failed(
        &profile,
        DatabaseConnectionState::Error,
        DatabaseError::capability("create_check_constraint is not supported.").into(),
    );
    let json = serde_json::to_value(&snapshot).expect("snapshot json");

    assert_eq!(json["profileId"], "profile-1");
    assert_eq!(json["serverId"], "server-1");
    assert_eq!(json["engine"], "mysql");
    assert_eq!(json["state"], "error");
    assert_eq!(json["readOnly"], true);
    assert_eq!(json["databaseName"], "app");
    assert_eq!(json["error"]["code"], "capability");
    assert!(!json.to_string().contains("hidden-reference"));
    assert!(!json.to_string().contains("credentialRef"));
}

#[test]
fn every_database_error_kind_maps_to_a_stable_app_error_code() {
    let expected = [
        (DatabaseErrorKind::Tunnel, "tunnel"),
        (DatabaseErrorKind::DatabaseAuth, "database_auth"),
        (DatabaseErrorKind::Timeout, "timeout"),
        (DatabaseErrorKind::Permission, "permission"),
        (DatabaseErrorKind::Query, "query"),
        (DatabaseErrorKind::Capability, "capability"),
        (DatabaseErrorKind::Cancelled, "cancelled"),
        (DatabaseErrorKind::Validation, "validation"),
        (DatabaseErrorKind::Connection, "connection"),
        (DatabaseErrorKind::Internal, "internal"),
    ];

    for (kind, code) in expected {
        let error = AppError::from(DatabaseError::new(kind, "safe message"));
        let json = serde_json::to_value(error).expect("typed error json");
        assert_eq!(json["code"], code);
        assert!(
            json["message"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
        );
    }
}

#[test]
fn creating_a_metadata_only_profile_keeps_has_credential_false() {
    let (_directory, database, credentials, index, server_id) = setup();
    let service = DatabaseProfileService::new(&database, &credentials, &index);
    let mut input = profile_input(&server_id, "No password yet");
    input.password = None;

    let profile = service.create(input).expect("metadata-only profile");
    assert!(!profile.has_credential);
    assert!(
        database
            .get_database_connection(&profile.id)
            .expect("stored profile")
            .expect("profile exists")
            .credential_ref
            .is_none()
    );
    service
        .delete(&profile.id)
        .expect("delete metadata-only profile");
}

#[test]
fn profile_service_does_not_accept_a_credential_reference_from_the_input() {
    let (_directory, database, credentials, index, server_id) = setup();
    let service = DatabaseProfileService::new(&database, &credentials, &index);
    let input: DatabaseProfileInput = serde_json::from_value(json!({
        "serverId": server_id,
        "name": "Injected reference",
        "engine": "postgresql",
        "host": "127.0.0.1",
        "port": 5432,
        "username": "app",
        "readOnly": false,
        "credentialRef": "serverdesk:db:attacker-controlled",
        "password": "fixture-password"
    }))
    .expect("unknown credentialRef is ignored by serde's camelCase input DTO");

    let profile = service.create(input).expect("create profile");
    let stored = database
        .get_database_connection(&profile.id)
        .expect("stored profile")
        .expect("profile exists");
    assert_ne!(
        stored.credential_ref.as_deref(),
        Some("serverdesk:db:attacker-controlled")
    );

    let database_credentials = DatabaseCredentialService::new(&credentials, &index);
    database_credentials
        .read(
            stored
                .credential_ref
                .as_deref()
                .expect("generated reference"),
        )
        .expect("profile uses the generated credential reference");
}
