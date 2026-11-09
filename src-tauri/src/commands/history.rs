use serde::Serialize;
use tauri::State;

use crate::{
    core::{AppError, history::MAX_OUTPUT_SUMMARY_CHARS},
    storage::{Database, MAX_OPERATION_HISTORY_RECORDS, OperationHistoryFilter, OperationRecord},
};

/// Local retention policy shown on the Command History page.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationHistoryPolicy {
    /// Newest rows kept before the oldest are pruned automatically.
    pub max_entries: u32,
    /// Hard character bound for the stored output summary.
    pub output_summary_max_chars: u32,
    /// Short explanation suitable for direct display.
    pub retention: String,
}

#[tauri::command]
pub fn list_operation_history(
    database: State<'_, Database>,
    filter: Option<OperationHistoryFilter>,
) -> Result<Vec<OperationRecord>, AppError> {
    let filter = filter.unwrap_or_default();
    database
        .list_operations_filtered(&filter)
        .map_err(AppError::from)
}

#[tauri::command]
pub fn get_operation_history_policy() -> OperationHistoryPolicy {
    OperationHistoryPolicy {
        max_entries: MAX_OPERATION_HISTORY_RECORDS,
        output_summary_max_chars: u32::try_from(MAX_OUTPUT_SUMMARY_CHARS).unwrap_or(u32::MAX),
        retention: format!(
            "ServerDesk keeps the newest {MAX_OPERATION_HISTORY_RECORDS} operations and \
             automatically removes older entries. Output is limited to a short, sanitized \
             summary."
        ),
    }
}

/// Clears local operation history after the UI confirms the action.
#[tauri::command]
pub fn clear_operation_history(
    database: State<'_, Database>,
    confirmed: bool,
) -> Result<(), AppError> {
    if !confirmed {
        return Err(AppError::validation(
            "Clearing operation history requires explicit confirmation.",
        ));
    }

    database.clear_operation_history().map_err(AppError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_serializes_with_camel_case_fields() {
        let policy = OperationHistoryPolicy {
            max_entries: MAX_OPERATION_HISTORY_RECORDS,
            output_summary_max_chars: u32::try_from(MAX_OUTPUT_SUMMARY_CHARS).unwrap(),
            retention: "bounded".to_string(),
        };
        let value = serde_json::to_value(policy).expect("serialized policy");

        assert_eq!(value["maxEntries"], 1_000);
        assert_eq!(value["outputSummaryMaxChars"], 360);
        assert_eq!(value["retention"], "bounded");
    }
}
