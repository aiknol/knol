//! Core domain types for the memory platform.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── Memory Types ──

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum MemoryType {
    Episodic,
    Semantic,
    Procedural,
    Working,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    Preference,
    Fact,
    Task,
    Event,
    Relationship,
    Summary,
    Procedure,
}

impl MemoryKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Preference => "preference",
            Self::Fact => "fact",
            Self::Task => "task",
            Self::Event => "event",
            Self::Relationship => "relationship",
            Self::Summary => "summary",
            Self::Procedure => "procedure",
        }
    }
}

impl std::fmt::Display for MemoryKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryScope {
    User,
    Team,
    Project,
    Agent,
    Org,
}

impl MemoryScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Team => "team",
            Self::Project => "project",
            Self::Agent => "agent",
            Self::Org => "org",
        }
    }
}

impl std::fmt::Display for MemoryScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryStatus {
    Active,
    Superseded,
    Archived,
    Deleted,
}

impl MemoryStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Superseded => "superseded",
            Self::Archived => "archived",
            Self::Deleted => "deleted",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
    System,
    Tool,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::System => "system",
            Self::Tool => "tool",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CreatedBy {
    System,
    User,
    Admin,
    Connector,
}

impl CreatedBy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Admin => "admin",
            Self::Connector => "connector",
        }
    }
}

// ── Domain Structs ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryItem {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub user_id: Option<Uuid>,
    pub scope: String,
    pub kind: String,
    pub content: String,
    pub content_json: Option<serde_json::Value>,
    pub confidence: f32,
    pub importance: f32,
    pub status: String,
    pub valid_from: DateTime<Utc>,
    pub valid_to: Option<DateTime<Utc>>,
    pub event_time: Option<DateTime<Utc>>,
    pub ingested_at: DateTime<Utc>,
    pub source_episode_id: Option<Uuid>,
    pub created_by: String,
    pub tags: Vec<String>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Episode {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub user_id: Option<Uuid>,
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
    pub content: String,
    pub role: String,
    pub event_time: DateTime<Utc>,
    pub ingested_at: DateTime<Utc>,
    pub storage_key: Option<String>,
    pub content_hash: String,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub entity_type: String,
    pub summary: Option<String>,
    pub attributes: serde_json::Value,
    pub valid_from: DateTime<Utc>,
    pub valid_to: Option<DateTime<Utc>>,
    pub status: String,
    pub merged_into: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub source_entity_id: Uuid,
    pub target_entity_id: Uuid,
    pub rel_type: String,
    pub properties: serde_json::Value,
    pub weight: f32,
    pub source_episode_id: Option<Uuid>,
    pub valid_from: DateTime<Utc>,
    pub valid_to: Option<DateTime<Utc>>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tenant {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub plan: String,
    pub config: serde_json::Value,
    pub api_key_hash: String,
    pub usage_ops_month: i32,
    pub usage_limit: Option<i32>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ── RBAC Types ──

/// Tenant-level role for API key access control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TenantRole {
    /// Full access: read, write, delete, admin operations, webhook/policy management.
    Admin,
    /// Read + write access: store memories, search, graph queries. No admin ops.
    Developer,
    /// Read-only access: search, get memory, list entities. No writes.
    ReadOnly,
}

impl TenantRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::Developer => "developer",
            Self::ReadOnly => "read_only",
        }
    }

    /// Parse a role string from the database.
    pub fn from_str_loose(s: &str) -> Self {
        match s {
            "admin" => Self::Admin,
            "developer" => Self::Developer,
            "read_only" | "readonly" => Self::ReadOnly,
            _ => Self::ReadOnly, // default to least privilege
        }
    }

    /// Returns true if this role has at least the given privilege level.
    pub fn has_permission(&self, required: TenantRole) -> bool {
        self.privilege_level() >= required.privilege_level()
    }

    fn privilege_level(&self) -> u8 {
        match self {
            Self::Admin => 3,
            Self::Developer => 2,
            Self::ReadOnly => 1,
        }
    }
}

impl std::fmt::Display for TenantRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ── Request/Response Types ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantContext {
    pub tenant_id: Uuid,
    pub user_id: Option<Uuid>,
    pub plan: String,
    /// Role derived from the API key used for authentication.
    #[serde(default = "default_tenant_role")]
    pub role: TenantRole,
}

fn default_tenant_role() -> TenantRole {
    TenantRole::ReadOnly
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryWriteRequest {
    pub content: String,
    pub role: Option<String>,
    pub user_id: Option<Uuid>,
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySearchRequest {
    pub query: String,
    pub user_id: Option<Uuid>,
    pub scope: Option<String>,
    pub kind: Option<String>,
    pub limit: Option<i64>,
    pub min_confidence: Option<f32>,
    pub temporal_filter: Option<TemporalFilter>,
    /// Session ID for session-scoped retrieval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Agent ID for agent-scoped retrieval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Filter by specific tags (AND logic: memory must have all specified tags).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    /// Filter by entity types (OR logic: memory must reference at least one entity of these types).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_types: Option<Vec<String>>,
    /// Minimum importance score (after decay) for returned memories.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_importance: Option<f32>,
    /// Enable decay-adjusted scoring (default: true when decay is configured).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apply_decay: Option<bool>,
    /// Maximum graph traversal depth for relational queries (default: 2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_depth: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalFilter {
    pub after: Option<DateTime<Utc>>,
    pub before: Option<DateTime<Utc>>,
    pub point_in_time: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub memory: MemoryItem,
    pub score: f64,
    pub vector_score: Option<f64>,
    pub graph_score: Option<f64>,
    pub related_entities: Vec<Entity>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySearchResponse {
    pub results: Vec<SearchResult>,
    pub total: usize,
    pub query_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryWriteResponse {
    pub episode_id: Uuid,
    pub status: String,
}

// ── Export/Import Types ──

/// Request to export memories for a tenant or user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryExportRequest {
    /// Filter by user_id (None = all users).
    pub user_id: Option<Uuid>,
    /// Filter by memory kind.
    pub kind: Option<String>,
    /// Filter by scope.
    pub scope: Option<String>,
    /// Export format: "json" (default), "jsonl"
    #[serde(default = "default_export_format")]
    pub format: String,
    /// Include entities and edges in export.
    #[serde(default = "default_true")]
    pub include_graph: bool,
    /// Include episode history.
    #[serde(default)]
    pub include_episodes: bool,
    /// Maximum number of memories to export (0 = unlimited).
    #[serde(default)]
    pub limit: i64,
}

fn default_export_format() -> String {
    "json".to_string()
}

fn default_true() -> bool {
    true
}

/// Exported memory data (for import/export).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryExport {
    /// Export format version.
    pub version: String,
    /// Export timestamp.
    pub exported_at: DateTime<Utc>,
    /// Tenant ID.
    pub tenant_id: Uuid,
    /// Exported memories.
    pub memories: Vec<MemoryItem>,
    /// Exported entities (if include_graph=true).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entities: Vec<Entity>,
    /// Exported edges (if include_graph=true).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<Edge>,
    /// Statistics.
    pub stats: ExportStats,
}

/// Export statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportStats {
    pub total_memories: usize,
    pub total_entities: usize,
    pub total_edges: usize,
    pub export_duration_ms: u64,
}

/// Request to import memories.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryImportRequest {
    /// Memories to import.
    pub memories: Vec<MemoryImportItem>,
    /// How to handle conflicts.
    #[serde(default = "default_import_strategy")]
    pub conflict_strategy: String,
    /// Generate new IDs for imported memories.
    #[serde(default = "default_true")]
    pub generate_new_ids: bool,
}

fn default_import_strategy() -> String {
    "skip_duplicates".to_string()
}

/// Single memory item for import.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryImportItem {
    pub content: String,
    pub kind: String,
    #[serde(default = "default_scope")]
    pub scope: String,
    #[serde(default)]
    pub confidence: Option<f32>,
    #[serde(default)]
    pub importance: Option<f32>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
    #[serde(default)]
    pub user_id: Option<Uuid>,
}

fn default_scope() -> String {
    "user".to_string()
}

/// Import result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryImportResponse {
    pub imported: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
}

// ── Queue Event Types ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryWriteEvent {
    pub episode_id: Uuid,
    pub tenant_id: Uuid,
    pub user_id: Option<Uuid>,
    pub content: String,
    pub role: String,
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
    pub metadata: serde_json::Value,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionResult {
    pub memories: Vec<ExtractedMemory>,
    pub entities: Vec<ExtractedEntity>,
    pub relationships: Vec<ExtractedRelationship>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedMemory {
    pub content: String,
    pub kind: String,
    pub confidence: f32,
    pub importance: f32,
    pub tags: Vec<String>,
    /// Verbatim quote from the source content that supports this memory.
    /// Populated when citation grounding is enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_quote: Option<String>,
    /// Character offset where the source quote starts in the input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_offset_start: Option<usize>,
    /// Character offset where the source quote ends in the input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_offset_end: Option<usize>,
}

// ── Grounding Types ──

/// Verification status for a grounded memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationStatus {
    Unverified,
    Verified,
    Contested,
    Failed,
}

impl VerificationStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unverified => "unverified",
            Self::Verified => "verified",
            Self::Contested => "contested",
            Self::Failed => "failed",
        }
    }
}

impl std::fmt::Display for VerificationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Result of verifying a single extracted memory against its source content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryVerification {
    /// Index of the memory in the extraction result.
    pub memory_index: usize,
    /// Verification status.
    pub status: VerificationStatus,
    /// Confidence score that the source content supports the memory (0.0-1.0).
    pub score: f32,
    /// Brief reasoning from the verification LLM.
    pub reasoning: String,
}

/// Configuration for the grounding system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroundingConfig {
    /// Enable citation grounding — instruct LLM to extract source quotes.
    pub enable_citations: bool,
    /// Enable factual verification — second LLM pass to verify memories.
    pub enable_verification: bool,
    /// Model to use for verification ("same" = use the extraction model).
    pub verification_model: String,
    /// Minimum verification score to mark a memory as "verified".
    pub min_verification_score: f32,
}

impl Default for GroundingConfig {
    fn default() -> Self {
        Self {
            enable_citations: true,
            enable_verification: false,
            verification_model: "same".to_string(),
            min_verification_score: 0.5,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedEntity {
    pub name: String,
    pub entity_type: String,
    pub summary: Option<String>,
    pub attributes: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedRelationship {
    pub source_entity: String,
    pub target_entity: String,
    pub rel_type: String,
    pub properties: Option<serde_json::Value>,
    pub weight: Option<f32>,
}

// ── Audit Types ──

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditAction {
    Create,
    Update,
    Delete,
    Merge,
    Supersede,
    Restore,
    Archive,
    Decay,
}

impl AuditAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Update => "update",
            Self::Delete => "delete",
            Self::Merge => "merge",
            Self::Supersede => "supersede",
            Self::Restore => "restore",
            Self::Archive => "archive",
            Self::Decay => "decay",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    #[test]
    fn test_memory_kind_as_str() {
        assert_eq!(MemoryKind::Preference.as_str(), "preference");
        assert_eq!(MemoryKind::Fact.as_str(), "fact");
        assert_eq!(MemoryKind::Task.as_str(), "task");
        assert_eq!(MemoryKind::Event.as_str(), "event");
        assert_eq!(MemoryKind::Relationship.as_str(), "relationship");
        assert_eq!(MemoryKind::Summary.as_str(), "summary");
        assert_eq!(MemoryKind::Procedure.as_str(), "procedure");
    }

    #[test]
    fn test_memory_scope_as_str() {
        assert_eq!(MemoryScope::User.as_str(), "user");
        assert_eq!(MemoryScope::Team.as_str(), "team");
        assert_eq!(MemoryScope::Project.as_str(), "project");
        assert_eq!(MemoryScope::Agent.as_str(), "agent");
        assert_eq!(MemoryScope::Org.as_str(), "org");
    }

    #[test]
    fn test_memory_status_as_str() {
        assert_eq!(MemoryStatus::Active.as_str(), "active");
        assert_eq!(MemoryStatus::Superseded.as_str(), "superseded");
        assert_eq!(MemoryStatus::Archived.as_str(), "archived");
        assert_eq!(MemoryStatus::Deleted.as_str(), "deleted");
    }

    #[test]
    fn test_audit_action_as_str() {
        assert_eq!(AuditAction::Create.as_str(), "create");
        assert_eq!(AuditAction::Update.as_str(), "update");
        assert_eq!(AuditAction::Delete.as_str(), "delete");
        assert_eq!(AuditAction::Merge.as_str(), "merge");
        assert_eq!(AuditAction::Supersede.as_str(), "supersede");
        assert_eq!(AuditAction::Restore.as_str(), "restore");
        assert_eq!(AuditAction::Archive.as_str(), "archive");
        assert_eq!(AuditAction::Decay.as_str(), "decay");
    }

    #[test]
    fn test_memory_kind_serde_roundtrip() {
        let kind = MemoryKind::Preference;
        let json = serde_json::to_string(&kind).unwrap();
        assert_eq!(json, "\"preference\"");
        let back: MemoryKind = serde_json::from_str(&json).unwrap();
        assert_eq!(back, kind);
    }

    #[test]
    fn test_memory_scope_serde_roundtrip() {
        let scope = MemoryScope::Team;
        let json = serde_json::to_string(&scope).unwrap();
        assert_eq!(json, "\"team\"");
        let back: MemoryScope = serde_json::from_str(&json).unwrap();
        assert_eq!(back, scope);
    }

    #[test]
    fn test_memory_write_request_serialize() {
        let req = MemoryWriteRequest {
            content: "User prefers dark mode".to_string(),
            role: Some("user".to_string()),
            user_id: Some(Uuid::new_v4()),
            session_id: Some("sess-123".to_string()),
            agent_id: None,
            metadata: Some(serde_json::json!({"source": "chat"})),
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: MemoryWriteRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.content, req.content);
    }

    #[test]
    fn test_memory_search_request_serialize() {
        let req = MemorySearchRequest {
            query: "What does the user prefer?".to_string(),
            user_id: None,
            scope: Some("user".to_string()),
            kind: Some("preference".to_string()),
            limit: Some(10),
            min_confidence: Some(0.5),
            temporal_filter: Some(TemporalFilter {
                after: Some(Utc::now()),
                before: None,
                point_in_time: None,
            }),
            session_id: None,
            agent_id: None,
            tags: None,
            entity_types: None,
            min_importance: None,
            apply_decay: None,
            graph_depth: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("preference"));
    }

    #[test]
    fn test_extraction_result_serialize() {
        let result = ExtractionResult {
            memories: vec![ExtractedMemory {
                content: "User likes Python".to_string(),
                kind: "preference".to_string(),
                confidence: 0.9,
                importance: 0.7,
                tags: vec!["programming".to_string()],
                source_quote: None,
                source_offset_start: None,
                source_offset_end: None,
            }],
            entities: vec![ExtractedEntity {
                name: "Python".to_string(),
                entity_type: "concept".to_string(),
                summary: Some("Programming language".to_string()),
                attributes: None,
            }],
            relationships: vec![ExtractedRelationship {
                source_entity: "User".to_string(),
                target_entity: "Python".to_string(),
                rel_type: "prefers".to_string(),
                properties: None,
                weight: Some(0.9),
            }],
        };
        let json = serde_json::to_string(&result).unwrap();
        let back: ExtractionResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back.memories.len(), 1);
        assert_eq!(back.entities.len(), 1);
        assert_eq!(back.relationships.len(), 1);
    }

    #[test]
    fn test_memory_write_event_serialize() {
        let event = MemoryWriteEvent {
            episode_id: Uuid::new_v4(),
            tenant_id: Uuid::new_v4(),
            user_id: Some(Uuid::new_v4()),
            content: "Hello world".to_string(),
            role: "user".to_string(),
            session_id: Some("sess-1".to_string()),
            agent_id: None,
            metadata: serde_json::json!({}),
            timestamp: Utc::now(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let back: MemoryWriteEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back.episode_id, event.episode_id);
        assert_eq!(back.content, "Hello world");
    }

    #[test]
    fn test_tenant_context_serialize() {
        let ctx = TenantContext {
            tenant_id: Uuid::new_v4(),
            user_id: Some(Uuid::new_v4()),
            plan: "pro".to_string(),
            role: TenantRole::Developer,
        };
        let json = serde_json::to_string(&ctx).unwrap();
        assert!(json.contains("pro"));
        assert!(json.contains("developer"));
    }

    #[test]
    fn test_tenant_role_permission_hierarchy() {
        assert!(TenantRole::Admin.has_permission(TenantRole::Admin));
        assert!(TenantRole::Admin.has_permission(TenantRole::Developer));
        assert!(TenantRole::Admin.has_permission(TenantRole::ReadOnly));
        assert!(!TenantRole::Developer.has_permission(TenantRole::Admin));
        assert!(TenantRole::Developer.has_permission(TenantRole::Developer));
        assert!(TenantRole::Developer.has_permission(TenantRole::ReadOnly));
        assert!(!TenantRole::ReadOnly.has_permission(TenantRole::Admin));
        assert!(!TenantRole::ReadOnly.has_permission(TenantRole::Developer));
        assert!(TenantRole::ReadOnly.has_permission(TenantRole::ReadOnly));
    }

    #[test]
    fn test_tenant_role_from_str_loose() {
        assert_eq!(TenantRole::from_str_loose("admin"), TenantRole::Admin);
        assert_eq!(
            TenantRole::from_str_loose("developer"),
            TenantRole::Developer
        );
        assert_eq!(
            TenantRole::from_str_loose("read_only"),
            TenantRole::ReadOnly
        );
        assert_eq!(TenantRole::from_str_loose("readonly"), TenantRole::ReadOnly);
        // Unknown defaults to ReadOnly (least privilege)
        assert_eq!(TenantRole::from_str_loose("unknown"), TenantRole::ReadOnly);
    }

    #[test]
    fn test_tenant_role_serde_roundtrip() {
        let role = TenantRole::Developer;
        let json = serde_json::to_string(&role).unwrap();
        assert_eq!(json, "\"developer\"");
        let back: TenantRole = serde_json::from_str(&json).unwrap();
        assert_eq!(back, role);
    }

    #[test]
    fn test_tenant_context_default_role() {
        // When deserializing without role field, should default to ReadOnly (least privilege)
        let json =
            r#"{"tenant_id":"550e8400-e29b-41d4-a716-446655440000","user_id":null,"plan":"free"}"#;
        let ctx: TenantContext = serde_json::from_str(json).unwrap();
        assert_eq!(ctx.role, TenantRole::ReadOnly);
    }
}
