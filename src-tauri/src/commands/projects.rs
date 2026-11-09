use std::{sync::Arc, time::Instant};

use serde::Serialize;
use tauri::State;

use crate::{
    core::{
        AppError,
        artisan::{ArtisanAction, ArtisanActionResult, run_artisan_action as run_artisan},
        history::{
            now_unix_seconds, record_operation, result_from_exit_code, summarize_error,
            summarize_output,
        },
        projects::{
            ProjectRecord, ProjectScanResult, RemoteDirectoryListing,
            list_remote_directories as browse, project_record, scan_remote_projects as scan,
        },
        ssh::ConnectionManager,
    },
    storage::{Database, NewOperationRecord, NewProject, OperationCategory, OperationResult},
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

/// One Artisan action result plus the local history write outcome.
///
/// A history failure never hides the execution result: the command already
/// ran, so the UI still receives its bounded output and exit status.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectArtisanRun {
    pub result: ArtisanActionResult,
    pub history_recorded: bool,
    pub history_error: Option<String>,
}

#[tauri::command]
pub async fn run_project_artisan_action(
    database: State<'_, Database>,
    manager: State<'_, Arc<ConnectionManager>>,
    id: String,
    project_id: String,
    action: ArtisanAction,
    confirmed: bool,
) -> Result<ProjectArtisanRun, AppError> {
    if id.trim().is_empty() {
        return Err(AppError::validation("Server ID is required."));
    }
    if project_id.trim().is_empty() {
        return Err(AppError::validation("Project ID is required."));
    }

    let project = database
        .get_project(&project_id)?
        .ok_or_else(|| AppError::not_found("The saved project no longer exists."))?;
    if project.server_id != id {
        return Err(AppError::validation(
            "The saved project does not belong to the selected server.",
        ));
    }

    let project_id = project.id;
    let target_path = project.remote_path;
    let started_at = now_unix_seconds();
    let started = Instant::now();
    let outcome = run_artisan(
        manager.inner().as_ref(),
        &id,
        &project_id,
        &target_path,
        action,
        confirmed,
    )
    .await;

    let result = match outcome {
        Ok(result) => result,
        Err(error) => {
            let _ = record_operation(
                database.inner(),
                &NewOperationRecord {
                    server_id: Some(id),
                    project_id: Some(project_id),
                    category: OperationCategory::Artisan,
                    operation: action.operation_name().to_string(),
                    safe_summary: format!(
                        "The Artisan {} action for {} did not complete",
                        action.argument(),
                        target_path
                    ),
                    path: Some(target_path),
                    output_summary: summarize_error(&error),
                    result: OperationResult::Failure,
                    exit_code: None,
                    started_at,
                    duration_ms: i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX),
                },
            );
            return Err(error);
        }
    };

    let record = NewOperationRecord {
        server_id: Some(id),
        project_id: Some(project_id),
        category: OperationCategory::Artisan,
        operation: result.action.operation_name().to_string(),
        safe_summary: result.safe_summary(),
        path: Some(result.target_path.clone()),
        output_summary: summarize_output(&result.stdout, &result.stderr),
        result: result_from_exit_code(result.exit_code),
        exit_code: result.exit_code.and_then(|code| i32::try_from(code).ok()),
        started_at: result.started_at,
        duration_ms: i64::try_from(result.duration_ms).unwrap_or(i64::MAX),
    };
    let (history_recorded, history_error) = match record_operation(database.inner(), &record) {
        Ok(_) => (true, None),
        Err(error) => (false, Some(error.to_string())),
    };

    Ok(ProjectArtisanRun {
        result,
        history_recorded,
        history_error,
    })
}

#[tauri::command]
pub fn delete_project(database: State<'_, Database>, id: String) -> Result<(), AppError> {
    if id.trim().is_empty() {
        return Err(AppError::validation("Project ID is required."));
    }

    database.delete_project(&id)?;
    Ok(())
}
