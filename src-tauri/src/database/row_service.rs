//! Row-browsing and row-writing service.
//!
//! Commands depend on this service, which resolves one live adapter from the
//! connection registry and calls the shared [`DatabaseAdapter`] contract. No
//! engine SQL, driver handle, or connection detail reaches the command layer.

use crate::core::{AppError, ErrorCode};

use super::{
    row_writes::{RowDeleteRequest, RowInsertRequest, RowUpdateRequest, RowWriteResult},
    rows::{RowQueryControl, RowReadRequest, RowReadResult},
    service::DatabaseConnectionRegistry,
};

/// Stateless service for bounded table row reads and verified row writes.
pub struct DatabaseRowService<'a> {
    connections: &'a DatabaseConnectionRegistry,
}

impl<'a> DatabaseRowService<'a> {
    pub fn new(connections: &'a DatabaseConnectionRegistry) -> Self {
        Self { connections }
    }

    pub async fn read(
        &self,
        profile_id: &str,
        request: &RowReadRequest,
        control: &RowQueryControl,
    ) -> Result<RowReadResult, AppError> {
        let adapter = self.connections.adapter(profile_id).await?;
        let adapter = adapter.lock().await;
        adapter
            .read_rows(request, control)
            .await
            .map_err(AppError::from)
    }

    pub async fn insert(
        &self,
        profile_id: &str,
        request: &RowInsertRequest,
        control: &RowQueryControl,
    ) -> Result<RowWriteResult, AppError> {
        request.validate().map_err(AppError::from)?;
        let adapter = self.connections.adapter(profile_id).await?;
        let mut adapter = adapter.lock().await;
        adapter
            .insert_row(request, control)
            .await
            .map_err(AppError::from)
    }

    pub async fn update(
        &self,
        profile_id: &str,
        request: &RowUpdateRequest,
        control: &RowQueryControl,
    ) -> Result<RowWriteResult, AppError> {
        request.validate().map_err(AppError::from)?;
        let adapter = self.connections.adapter(profile_id).await?;
        let mut adapter = adapter.lock().await;
        adapter
            .update_row(request, control)
            .await
            .map_err(AppError::from)
    }

    /// Deletes one or more explicit row identities. The command layer must pass
    /// `confirmed = true` after the user reviewed the exact target count. An
    /// unconfirmed request is rejected before any adapter or transaction exists.
    pub async fn delete(
        &self,
        profile_id: &str,
        request: &RowDeleteRequest,
        confirmed: bool,
        control: &RowQueryControl,
    ) -> Result<RowWriteResult, AppError> {
        request.validate().map_err(AppError::from)?;
        if !confirmed {
            return Err(delete_confirmation_error());
        }
        let adapter = self.connections.adapter(profile_id).await?;
        let mut adapter = adapter.lock().await;
        adapter
            .delete_rows(request, control)
            .await
            .map_err(AppError::from)
    }
}

/// Rejects an unconfirmed delete before any adapter or transaction exists.
pub fn delete_confirmation_error() -> AppError {
    AppError::new(
        ErrorCode::Validation,
        "Deleting rows requires explicit confirmation of the selected row identities.",
    )
    .with_details("No rows were deleted. Review the selected rows and confirm the delete again.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unconfirmed_delete_is_rejected_before_any_adapter_is_resolved() {
        let error = delete_confirmation_error();
        assert_eq!(error.code, ErrorCode::Validation);
        assert!(error.message.contains("requires explicit confirmation"));
        assert!(
            error
                .details
                .as_deref()
                .is_some_and(|details| details.contains("No rows were deleted"))
        );
    }
}
