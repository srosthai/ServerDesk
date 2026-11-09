use serverdesk_lib::storage::{
    AuthType, Database, NewOperationRecord, NewPortForward, NewProject, NewSavedCommand, NewServer,
    OperationCategory, OperationResult,
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
fn initializes_schema_at_current_version() {
    let database = Database::in_memory().expect("in-memory database");

    assert_eq!(database.schema_version().expect("schema version"), 4);
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
            laravel_version: Some("v11.30.0".to_string()),
            php_version: Some("8.3.14".to_string()),
            queue_hint: Some("Queue connection: redis".to_string()),
            scheduler_hint: None,
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
        .insert_known_host(&server.id, "ssh-ed25519", "SHA256:first")
        .expect("insert known host");
    let duplicate = database
        .insert_known_host(&server.id, "ssh-ed25519", "SHA256:first")
        .expect("idempotent known host");
    assert_eq!(known_host.id, duplicate.id);
    assert!(
        database
            .insert_known_host(&server.id, "ssh-ed25519", "SHA256:second")
            .is_err(),
        "a different fingerprint must never overwrite a trusted key"
    );
    assert_eq!(
        database
            .get_known_host(&server.id, "ssh-ed25519")
            .expect("get known host")
            .expect("known host exists")
            .fingerprint,
        "SHA256:first"
    );
    assert_eq!(
        database
            .list_known_hosts(&server.id)
            .expect("list known hosts")
            .len(),
        1
    );
    assert!(
        database
            .delete_known_host(&server.id, "ssh-ed25519")
            .expect("delete known host")
    );
    assert!(
        !database
            .delete_known_host(&server.id, "ssh-ed25519")
            .expect("delete missing known host")
    );
    database
        .insert_known_host(&server.id, "ssh-ed25519", "SHA256:first")
        .expect("reinsert known host");

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
            category: OperationCategory::Command,
            operation: "hostname".to_string(),
            safe_summary: "Read hostname".to_string(),
            path: None,
            output_summary: Some("fixture-host".to_string()),
            result: OperationResult::Success,
            exit_code: Some(0),
            started_at: 1_700_000_000,
            duration_ms: 120,
        })
        .expect("record operation");
    let operations = database.list_operations(10).expect("list operations");
    assert_eq!(operations.len(), 1);
    assert_eq!(operations[0].exit_code, Some(0));
    assert_eq!(operations[0].category, OperationCategory::Command);
    assert_eq!(operations[0].result, OperationResult::Success);
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

#[test]
fn duplicate_server_names_are_rejected_case_insensitively() {
    let database = Database::in_memory().expect("in-memory database");
    let created = database
        .create_server(&server_input("Production"))
        .expect("create server");

    assert!(database.create_server(&server_input("production")).is_err());
    assert!(
        database
            .server_name_exists("PRODUCTION", None)
            .expect("name lookup")
    );
    assert!(
        !database
            .server_name_exists("PRODUCTION", Some(&created.id))
            .expect("name lookup excluding self")
    );
}

#[test]
fn trusted_known_hosts_survive_a_database_restart() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("serverdesk.sqlite3");
    let server_id;

    {
        let database = Database::open(&path).expect("open database");
        let server = database
            .create_server(&server_input("Production"))
            .expect("create server");
        server_id = server.id.clone();
        database
            .insert_known_host(&server.id, "ssh-ed25519", "SHA256:trusted")
            .expect("insert known host");
    }

    {
        let database = Database::open(&path).expect("reopen database");
        let hosts = database
            .list_known_hosts(&server_id)
            .expect("list known hosts after restart");
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].key_type, "ssh-ed25519");
        assert_eq!(hosts[0].fingerprint, "SHA256:trusted");
    }
}

#[test]
fn duplicate_project_paths_update_one_record() {
    let database = Database::in_memory().expect("in-memory database");
    let server = database
        .create_server(&server_input("Production"))
        .expect("create server");

    let first = database
        .upsert_project(&NewProject {
            server_id: server.id.clone(),
            name: "storefront".to_string(),
            remote_path: "/var/www/storefront".to_string(),
            project_type: "laravel".to_string(),
            laravel_version: None,
            php_version: None,
            queue_hint: None,
            scheduler_hint: None,
        })
        .expect("first detection");

    let second = database
        .upsert_project(&NewProject {
            server_id: server.id.clone(),
            name: "storefront".to_string(),
            remote_path: "/var/www/storefront".to_string(),
            project_type: "laravel".to_string(),
            laravel_version: Some("v11.30.0".to_string()),
            php_version: Some("8.3.14".to_string()),
            queue_hint: Some("Queue connection: redis".to_string()),
            scheduler_hint: Some("A cron entry runs schedule:run for this path".to_string()),
        })
        .expect("second detection");

    assert_eq!(
        first.id, second.id,
        "rescanning must keep the existing record"
    );
    assert_eq!(
        database
            .list_projects(&server.id)
            .expect("list projects")
            .len(),
        1,
        "the same server and path must never create a duplicate"
    );
    assert_eq!(second.laravel_version.as_deref(), Some("v11.30.0"));
    assert_eq!(second.php_version.as_deref(), Some("8.3.14"));
    assert!(second.detected_at >= first.detected_at);
    assert_eq!(
        database
            .get_project_by_path(&server.id, "/var/www/storefront")
            .expect("get by path")
            .expect("project exists")
            .id,
        first.id
    );
}

#[test]
fn projects_survive_a_database_restart() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("serverdesk.sqlite3");
    let server_id;
    let project_id;

    {
        let database = Database::open(&path).expect("open database");
        let server = database
            .create_server(&server_input("Production"))
            .expect("create server");
        server_id = server.id.clone();
        project_id = database
            .upsert_project(&NewProject {
                server_id: server.id.clone(),
                name: "storefront".to_string(),
                remote_path: "/var/www/storefront".to_string(),
                project_type: "laravel".to_string(),
                laravel_version: Some("v11.30.0".to_string()),
                php_version: Some("8.3.14".to_string()),
                queue_hint: Some("Queue connection: redis".to_string()),
                scheduler_hint: None,
            })
            .expect("create project")
            .id;
    }

    {
        let database = Database::open(&path).expect("reopen database");
        let projects = database
            .list_projects(&server_id)
            .expect("list projects after restart");
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].id, project_id);
        assert_eq!(projects[0].remote_path, "/var/www/storefront");
        assert_eq!(projects[0].laravel_version.as_deref(), Some("v11.30.0"));
        assert_eq!(projects[0].php_version.as_deref(), Some("8.3.14"));
        assert_eq!(
            projects[0].queue_hint.as_deref(),
            Some("Queue connection: redis")
        );
    }
}
