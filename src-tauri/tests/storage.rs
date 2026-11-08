use serverdesk_lib::storage::{
    AuthType, Database, NewOperationRecord, NewPortForward, NewProject, NewSavedCommand, NewServer,
};

fn server_input(name: &str) -> NewServer {
    NewServer {
        name: name.to_string(),
        host: "198.51.100.24".to_string(),
        port: 22,
        username: "deploy".to_string(),
        auth_type: AuthType::PrivateKey,
        credential_ref: Some("credential:test-server".to_string()),
        group_name: Some("Production".to_string()),
        tags: vec!["web".to_string(), "laravel".to_string()],
        notes: Some("Fixture server".to_string()),
    }
}

#[test]
fn initializes_schema_at_version_one() {
    let database = Database::in_memory().expect("in-memory database");

    assert_eq!(database.schema_version().expect("schema version"), 1);
    assert!(database.list_servers().expect("server list").is_empty());
}

#[test]
fn server_crud_persists_metadata_without_secrets() {
    let database = Database::in_memory().expect("in-memory database");
    let created = database
        .create_server(&server_input("Production"))
        .expect("create server");

    assert_eq!(created.name, "Production");
    assert_eq!(created.port, 22);
    assert_eq!(created.auth_type, AuthType::PrivateKey);
    assert_eq!(created.tags, vec!["web", "laravel"]);
    assert_eq!(
        created.credential_ref.as_deref(),
        Some("credential:test-server")
    );

    let fetched = database
        .get_server(&created.id)
        .expect("get server")
        .expect("server exists");
    assert_eq!(fetched, created);
    assert_eq!(database.list_servers().expect("list servers").len(), 1);

    let mut update = server_input("Staging");
    update.port = 2222;
    update.tags = vec!["staging".to_string()];
    let updated = database
        .update_server(&created.id, &update)
        .expect("update server");

    assert_eq!(updated.name, "Staging");
    assert_eq!(updated.port, 2222);
    assert_eq!(updated.tags, vec!["staging"]);
    assert!(updated.updated_at >= created.updated_at);

    database.delete_server(&created.id).expect("delete server");
    assert!(
        database
            .get_server(&created.id)
            .expect("get deleted")
            .is_none()
    );
    assert!(
        database
            .list_servers()
            .expect("list after delete")
            .is_empty()
    );
}

#[test]
fn related_records_survive_and_cascade() {
    let database = Database::in_memory().expect("in-memory database");
    let server = database
        .create_server(&server_input("Production"))
        .expect("create server");

    let project = database
        .create_project(&NewProject {
            server_id: server.id.clone(),
            name: "storefront".to_string(),
            remote_path: "/var/www/storefront".to_string(),
            project_type: "laravel".to_string(),
        })
        .expect("create project");
    assert_eq!(
        database
            .get_project(&project.id)
            .expect("get project")
            .expect("project exists")
            .remote_path,
        "/var/www/storefront"
    );

    let known_host = database
        .upsert_known_host(&server.id, "ssh-ed25519", "SHA256:first")
        .expect("insert known host");
    let updated_host = database
        .upsert_known_host(&server.id, "ssh-ed25519", "SHA256:second")
        .expect("update known host");
    assert_eq!(known_host.id, updated_host.id);
    assert_eq!(updated_host.fingerprint, "SHA256:second");
    assert_eq!(
        database
            .list_known_hosts(&server.id)
            .expect("list known hosts")
            .len(),
        1
    );

    let forward = database
        .create_port_forward(&NewPortForward {
            server_id: server.id.clone(),
            bind_host: "127.0.0.1".to_string(),
            local_port: 5432,
            remote_host: "127.0.0.1".to_string(),
            remote_port: 5432,
        })
        .expect("create port forward");
    assert_eq!(
        database
            .list_port_forwards(&server.id)
            .expect("list port forwards")[0]
            .id,
        forward.id
    );

    database.delete_server(&server.id).expect("delete server");
    assert!(
        database
            .list_projects(&server.id)
            .expect("projects")
            .is_empty()
    );
    assert!(
        database
            .list_known_hosts(&server.id)
            .expect("known hosts")
            .is_empty()
    );
    assert!(
        database
            .list_port_forwards(&server.id)
            .expect("port forwards")
            .is_empty()
    );
}

#[test]
fn saved_commands_history_and_preferences_round_trip() {
    let database = Database::in_memory().expect("in-memory database");

    let command = database
        .create_saved_command(&NewSavedCommand {
            name: "Restart nginx".to_string(),
            category: "service".to_string(),
            command_template: "systemctl restart nginx".to_string(),
            requires_confirmation: true,
        })
        .expect("create saved command");
    assert!(command.requires_confirmation);
    assert_eq!(
        database.list_saved_commands().expect("list commands").len(),
        1
    );
    database
        .delete_saved_command(&command.id)
        .expect("delete saved command");
    assert!(
        database
            .list_saved_commands()
            .expect("list commands")
            .is_empty()
    );

    database
        .record_operation(&NewOperationRecord {
            server_id: None,
            project_id: None,
            operation: "hostname".to_string(),
            safe_summary: "Read hostname".to_string(),
            exit_code: Some(0),
            started_at: 1_700_000_000,
            duration_ms: 120,
        })
        .expect("record operation");
    let operations = database.list_operations(10).expect("list operations");
    assert_eq!(operations.len(), 1);
    assert_eq!(operations[0].exit_code, Some(0));
    database
        .clear_operation_history()
        .expect("clear operation history");
    assert!(
        database
            .list_operations(10)
            .expect("list operations")
            .is_empty()
    );

    let first = database
        .set_preference("theme", "dark")
        .expect("set preference");
    let second = database
        .set_preference("theme", "light")
        .expect("update preference");
    assert_eq!(first.key, second.key);
    assert_eq!(
        database
            .get_preference("theme")
            .expect("get preference")
            .expect("preference exists")
            .value,
        "light"
    );
    database
        .delete_preference("theme")
        .expect("delete preference");
    assert!(
        database
            .get_preference("theme")
            .expect("get preference")
            .is_none()
    );
}
