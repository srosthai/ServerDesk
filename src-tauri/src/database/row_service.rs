//! Row-browsing service.
//!
//! Commands depend on this service, which resolves one live adapter from the
//! connection registry and calls the shared [`DatabaseAdapter`] contract. No
//! engine SQL, driver handle, or connection detail reaches the command layer.

use crate::core::AppError;

use super::{
    rows::{RowQueryControl, RowReadRequest, RowReadResult},
    service::DatabaseConnectionRegistry,
};

/// Stateless service for bounded table row reads.
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
}
