use std::sync::Arc;

use tauri::State;

use crate::{
    core::{
        AppError,
        projects::{
            ProjectRecord, ProjectScanResult, RemoteDirectoryListing,
            list_remote_directories as browse, project_record, scan_remote_projects as scan,
        },
        ssh::ConnectionManager,
    },
    storage::{Database, NewProject},
};

#[tauri::command]
pub fn list_projects(
    database: State<'_, Database>,
    id: String,
) -> Result<Vec<ProjectRecord>, AppError> {
    if id.trim().is_empty() {
        return Err(AppError::validation("Server ID is required."));
    }

    load_project_records(database.inner(), &id)
}

fn load_project_records(
    database: &Database,
    server_id: &str,
) -> Result<Vec<ProjectRecord>, AppError> {
    let mut records = Vec::new();
    for project in database.list_projects(server_id)? {
        let operations = database.list_project_operations(
            &project.id,
            crate::core::projects::RECENT_PROJECT_OPERATIONS,
        )?;
        records.push(project_record(project, operations));
    }
    Ok(records)
}

#[tauri::command]
pub async fn browse_remote_directories(
    manager: State<'_, Arc<ConnectionManager>>,
    id: String,
    path: String,
) -> Result<RemoteDirectoryListing, AppError> {
    browse(manager.inner().as_ref(), &id, &path).await
}

#[tauri::command]
pub async fn scan_remote_projects(
    database: State<'_, Database>,
    manager: State<'_, Arc<ConnectionManager>>,
    id: String,
    path: String,
    include_children: bool,
) -> Result<ProjectScanResult, AppError> {
    let server_id = id.clone();
    let mut scan = scan(manager.inner().as_ref(), &id, &path, include_children).await?;
    let detected = std::mem::take(&mut scan.projects);
    let mut records = Vec::with_capacity(detected.len());

    for project in detected {
        let stored = database.upsert_project(&NewProject {
            server_id: server_id.clone(),
            name: project.name,
            remote_path: project.remote_path,
            project_type: project.project_type,
            laravel_version: project.laravel_version,
            php_version: project.php_version,
            queue_hint: project.queue_hint,
            scheduler_hint: project.scheduler_hint,
        })?;
        let operations = database.list_project_operations(
            &stored.id,
            crate::core::projects::RECENT_PROJECT_OPERATIONS,
        )?;
        records.push(project_record(stored, operations));
    }

    Ok(ProjectScanResult::from_scan(scan, records))
}

#[tauri::command]
pub fn delete_project(database: State<'_, Database>, id: String) -> Result<(), AppError> {
    if id.trim().is_empty() {
        return Err(AppError::validation("Project ID is required."));
    }

    database.delete_project(&id)?;
    Ok(())
}
