//! Typed explorer-node discovery.
//!
//! React asks for one explorer node at a time. Each request names the node and
//! its optional database/schema scope, and the service dispatches it to the
//! shared adapter contract. A refresh is the same node request: it never
//! reloads sibling nodes because the backend keeps no explorer cache.

use serde::{Deserialize, Serialize};

use crate::core::AppError;

use super::{
    errors::DatabaseError,
    pagination::{Page, PageRequest},
    service::DatabaseConnectionRegistry,
    traits::{
        DatabaseSummary, MetadataScope, ObjectCounts, SchemaSummary, SequenceSummary, TableSummary,
        ViewSummary,
    },
};

/// One lazily loaded explorer node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ExplorerNode {
    Databases,
    Schemas,
    Tables {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        database: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<String>,
    },
    Views {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        database: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<String>,
    },
    Sequences {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        database: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<String>,
    },
    ObjectCounts {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        database: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<String>,
    },
}

impl ExplorerNode {
    /// Validates the node scope with the same identifier rules every adapter
    /// uses. Database and schema names never become SQL text.
    pub fn scope(&self) -> Result<Option<MetadataScope>, DatabaseError> {
        match self {
            Self::Databases | Self::Schemas => Ok(None),
            Self::Tables {
                database, schema, ..
            }
            | Self::Views {
                database, schema, ..
            }
            | Self::Sequences {
                database, schema, ..
            }
            | Self::ObjectCounts {
                database, schema, ..
            } => MetadataScope::new(database.as_deref(), schema.as_deref()).map(Some),
        }
    }

    pub const fn kind(&self) -> ExplorerNodeKind {
        match self {
            Self::Databases => ExplorerNodeKind::Databases,
            Self::Schemas => ExplorerNodeKind::Schemas,
            Self::Tables { .. } => ExplorerNodeKind::Tables,
            Self::Views { .. } => ExplorerNodeKind::Views,
            Self::Sequences { .. } => ExplorerNodeKind::Sequences,
            Self::ObjectCounts { .. } => ExplorerNodeKind::ObjectCounts,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExplorerNodeKind {
    Databases,
    Schemas,
    Tables,
    Views,
    Sequences,
    ObjectCounts,
}

/// Payload returned for one explorer node. The tag matches [`ExplorerNode`]
/// so React can reject a response for the wrong node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "nodeKind", content = "payload", rename_all = "camelCase")]
pub enum ExplorerNodePayload {
    Databases(Page<DatabaseSummary>),
    Schemas(Page<SchemaSummary>),
    Tables(Page<TableSummary>),
    Views(Page<ViewSummary>),
    Sequences(Page<SequenceSummary>),
    ObjectCounts(ObjectCounts),
}

impl ExplorerNodePayload {
    pub const fn kind(&self) -> ExplorerNodeKind {
        match self {
            Self::Databases(_) => ExplorerNodeKind::Databases,
            Self::Schemas(_) => ExplorerNodeKind::Schemas,
            Self::Tables(_) => ExplorerNodeKind::Tables,
            Self::Views(_) => ExplorerNodeKind::Views,
            Self::Sequences(_) => ExplorerNodeKind::Sequences,
            Self::ObjectCounts(_) => ExplorerNodeKind::ObjectCounts,
        }
    }
}

/// Stateless metadata service used by the discovery commands.
pub struct DatabaseDiscoveryService<'a> {
    connections: &'a DatabaseConnectionRegistry,
}

impl<'a> DatabaseDiscoveryService<'a> {
    pub fn new(connections: &'a DatabaseConnectionRegistry) -> Self {
        Self { connections }
    }

    pub async fn databases(
        &self,
        profile_id: &str,
        page: PageRequest,
    ) -> Result<Page<DatabaseSummary>, AppError> {
        let adapter = self.connections.adapter(profile_id).await?;
        let adapter = adapter.lock().await;
        adapter.list_databases(page).await.map_err(AppError::from)
    }

    pub async fn schemas(
        &self,
        profile_id: &str,
        page: PageRequest,
    ) -> Result<Page<SchemaSummary>, AppError> {
        let adapter = self.connections.adapter(profile_id).await?;
        let adapter = adapter.lock().await;
        adapter.list_schemas(page).await.map_err(AppError::from)
    }

    pub async fn tables(
        &self,
        profile_id: &str,
        scope: &MetadataScope,
        page: PageRequest,
    ) -> Result<Page<TableSummary>, AppError> {
        let adapter = self.connections.adapter(profile_id).await?;
        let adapter = adapter.lock().await;
        adapter
            .list_tables(scope, page)
            .await
            .map_err(AppError::from)
    }

    pub async fn views(
        &self,
        profile_id: &str,
        scope: &MetadataScope,
        page: PageRequest,
    ) -> Result<Page<ViewSummary>, AppError> {
        let adapter = self.connections.adapter(profile_id).await?;
        let adapter = adapter.lock().await;
        adapter
            .list_views(scope, page)
            .await
            .map_err(AppError::from)
    }

    pub async fn sequences(
        &self,
        profile_id: &str,
        scope: &MetadataScope,
        page: PageRequest,
    ) -> Result<Page<SequenceSummary>, AppError> {
        let adapter = self.connections.adapter(profile_id).await?;
        let adapter = adapter.lock().await;
        adapter
            .list_sequences(scope, page)
            .await
            .map_err(AppError::from)
    }

    pub async fn object_counts(
        &self,
        profile_id: &str,
        scope: &MetadataScope,
    ) -> Result<ObjectCounts, AppError> {
        let adapter = self.connections.adapter(profile_id).await?;
        let adapter = adapter.lock().await;
        adapter.object_counts(scope).await.map_err(AppError::from)
    }

    /// Loads exactly one explorer node. This is also the single-node refresh
    /// path: no other node is read or cached by the backend.
    pub async fn load_node(
        &self,
        profile_id: &str,
        node: &ExplorerNode,
        page: PageRequest,
    ) -> Result<ExplorerNodePayload, AppError> {
        let scope = node.scope()?;
        match node {
            ExplorerNode::Databases => Ok(ExplorerNodePayload::Databases(
                self.databases(profile_id, page).await?,
            )),
            ExplorerNode::Schemas => Ok(ExplorerNodePayload::Schemas(
                self.schemas(profile_id, page).await?,
            )),
            ExplorerNode::Tables { .. } => {
                let scope = scope.expect("tables always have a validated scope");
                Ok(ExplorerNodePayload::Tables(
                    self.tables(profile_id, &scope, page).await?,
                ))
            }
            ExplorerNode::Views { .. } => {
                let scope = scope.expect("views always have a validated scope");
                Ok(ExplorerNodePayload::Views(
                    self.views(profile_id, &scope, page).await?,
                ))
            }
            ExplorerNode::Sequences { .. } => {
                let scope = scope.expect("sequences always have a validated scope");
                Ok(ExplorerNodePayload::Sequences(
                    self.sequences(profile_id, &scope, page).await?,
                ))
            }
            ExplorerNode::ObjectCounts { .. } => {
                let scope = scope.expect("object counts always have a validated scope");
                Ok(ExplorerNodePayload::ObjectCounts(
                    self.object_counts(profile_id, &scope).await?,
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::{MetadataValue, ObjectMetadata, TableKind, ViewKind};

    fn database(name: &str) -> DatabaseSummary {
        DatabaseSummary {
            name: name.to_string(),
            system: false,
            metadata: ObjectMetadata {
                encoding: MetadataValue::available("utf8mb4".to_string()),
                collation: MetadataValue::available("utf8mb4_unicode_ci".to_string()),
                size_bytes: MetadataValue::available(4_096),
                ..ObjectMetadata::default()
            },
        }
    }

    fn page<T>(items: Vec<T>) -> Page<T> {
        Page::from_items(items, PageRequest::default(), Some(1))
    }

    #[test]
    fn explorer_nodes_validate_scope_and_report_their_kind() {
        assert_eq!(ExplorerNode::Databases.scope().expect("scope"), None);
        assert_eq!(ExplorerNode::Schemas.kind(), ExplorerNodeKind::Schemas);
        assert_eq!(
            ExplorerNode::Tables {
                database: Some("app".to_string()),
                schema: Some("public".to_string()),
            }
            .scope()
            .expect("scope"),
            Some(MetadataScope::new(Some("app"), Some("public")).expect("scope"))
        );

        let error = ExplorerNode::Views {
            database: None,
            schema: Some("bad\nschema".to_string()),
        }
        .scope()
        .expect_err("invalid schema");
        assert_eq!(error.kind, crate::database::DatabaseErrorKind::Validation);
        assert_eq!(
            ExplorerNode::ObjectCounts {
                database: None,
                schema: None,
            }
            .kind(),
            ExplorerNodeKind::ObjectCounts
        );
    }

    #[test]
    fn explorer_node_requests_deserialize_from_the_frontend_tag() {
        let node: ExplorerNode = serde_json::from_value(serde_json::json!({
            "kind": "tables",
            "database": "app",
            "schema": "public"
        }))
        .expect("deserialize node");

        assert_eq!(
            node,
            ExplorerNode::Tables {
                database: Some("app".to_string()),
                schema: Some("public".to_string()),
            }
        );
        assert_eq!(node.kind(), ExplorerNodeKind::Tables);

        let json = serde_json::to_value(ExplorerNode::ObjectCounts {
            database: Some("app".to_string()),
            schema: None,
        })
        .expect("serialize node");
        assert_eq!(json["kind"], "objectCounts");
        assert_eq!(json["database"], "app");
    }

    #[test]
    fn payloads_are_tagged_and_never_contain_connection_details() {
        let payload = ExplorerNodePayload::Databases(page(vec![database("app")]));
        let json = serde_json::to_string(&payload).expect("payload json");

        assert!(json.contains("\"nodeKind\":\"databases\""));
        assert!(json.contains("\"status\":\"available\""));
        assert!(!json.contains("127.0.0.1"));
        assert!(!json.contains("password"));
        assert!(!json.contains("mysql://"));

        assert_eq!(payload.kind(), ExplorerNodeKind::Databases);
        assert_eq!(
            ExplorerNodePayload::Views(page(Vec::<ViewSummary>::new())).kind(),
            ExplorerNodeKind::Views
        );
        assert_eq!(
            ExplorerNodePayload::ObjectCounts(ObjectCounts {
                tables: MetadataValue::available(2),
                views: MetadataValue::Unavailable,
                materialized_views: MetadataValue::Unavailable,
                sequences: MetadataValue::Unavailable,
            })
            .kind(),
            ExplorerNodeKind::ObjectCounts
        );
    }

    #[test]
    fn summary_types_round_trip_with_unavailable_fields() {
        let summary = TableSummary {
            database: Some("app".to_string()),
            schema: Some("public".to_string()),
            name: "users".to_string(),
            kind: TableKind::Table,
            estimated_rows: MetadataValue::available(12),
            metadata: ObjectMetadata {
                owner: MetadataValue::available("app".to_string()),
                engine: MetadataValue::Unavailable,
                ..ObjectMetadata::default()
            },
        };
        let json = serde_json::to_string(&summary).expect("summary json");
        let decoded: TableSummary = serde_json::from_str(&json).expect("summary round trip");

        assert_eq!(decoded, summary);
        assert_eq!(decoded.metadata.engine, MetadataValue::Unavailable);
        assert_eq!(
            ViewKind::MaterializedView,
            ViewKind::MaterializedView,
            "view kinds stay typed"
        );
    }
}
