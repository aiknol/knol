//! Knowledge graph operations: entity/edge CRUD, traversal, temporal filtering.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use tracing::debug;
use uuid::Uuid;

// ── Entity Operations ──

/// Upsert an entity (insert or update if exists with same name+type for tenant).
pub async fn upsert_entity(
    pool: &PgPool,
    tenant_id: Uuid,
    name: &str,
    entity_type: &str,
    summary: Option<&str>,
    attributes: Option<&serde_json::Value>,
) -> Result<Uuid, GraphError> {
    let attrs = attributes.cloned().unwrap_or_else(|| serde_json::json!({}));

    let id = sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO entities (tenant_id, name, entity_type, summary, attributes)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (tenant_id, name, entity_type) WHERE status = 'active'
        DO UPDATE SET
            summary = COALESCE(EXCLUDED.summary, entities.summary),
            attributes = entities.attributes || EXCLUDED.attributes,
            updated_at = now()
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(name)
    .bind(entity_type)
    .bind(summary)
    .bind(&attrs)
    .fetch_one(pool)
    .await
    .map_err(GraphError::Database)?;

    debug!("Upserted entity '{}' ({}) -> {}", name, entity_type, id);
    Ok(id)
}

/// Get an entity by ID.
pub async fn get_entity(pool: &PgPool, id: Uuid) -> Result<Option<EntityRow>, GraphError> {
    let row = sqlx::query_as::<_, EntityRow>(
        "SELECT * FROM entities WHERE id = $1 AND status = 'active'",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(GraphError::Database)?;
    Ok(row)
}

/// Find entity by name and type for a tenant.
pub async fn find_entity(
    pool: &PgPool,
    tenant_id: Uuid,
    name: &str,
    entity_type: &str,
) -> Result<Option<EntityRow>, GraphError> {
    let row = sqlx::query_as::<_, EntityRow>(
        r#"
        SELECT * FROM entities
        WHERE tenant_id = $1 AND name = $2 AND entity_type = $3 AND status = 'active'
        "#,
    )
    .bind(tenant_id)
    .bind(name)
    .bind(entity_type)
    .fetch_optional(pool)
    .await
    .map_err(GraphError::Database)?;
    Ok(row)
}

/// List entities for a tenant with optional type filter.
pub async fn list_entities(
    pool: &PgPool,
    tenant_id: Uuid,
    entity_type: Option<&str>,
    limit: i64,
) -> Result<Vec<EntityRow>, GraphError> {
    let rows = sqlx::query_as::<_, EntityRow>(
        r#"
        SELECT * FROM entities
        WHERE tenant_id = $1
          AND status = 'active'
          AND ($2::text IS NULL OR entity_type = $2)
        ORDER BY updated_at DESC
        LIMIT $3
        "#,
    )
    .bind(tenant_id)
    .bind(entity_type)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(GraphError::Database)?;
    Ok(rows)
}

// ── Edge Operations ──

/// Insert or update an edge between two entities.
#[allow(clippy::too_many_arguments)]
pub async fn upsert_edge(
    pool: &PgPool,
    tenant_id: Uuid,
    source_entity_id: Uuid,
    target_entity_id: Uuid,
    rel_type: &str,
    properties: Option<&serde_json::Value>,
    weight: f32,
    source_episode_id: Option<Uuid>,
) -> Result<Uuid, GraphError> {
    let props = properties.cloned().unwrap_or_else(|| serde_json::json!({}));

    let id = sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO edges (tenant_id, source_entity_id, target_entity_id, rel_type, properties, weight, source_episode_id)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT DO NOTHING
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(source_entity_id)
    .bind(target_entity_id)
    .bind(rel_type)
    .bind(&props)
    .bind(weight)
    .bind(source_episode_id)
    .fetch_optional(pool)
    .await
    .map_err(GraphError::Database)?
    .unwrap_or_else(Uuid::new_v4);

    debug!(
        "Upserted edge {} -[{}]-> {}",
        source_entity_id, rel_type, target_entity_id
    );
    Ok(id)
}

/// Get all edges from a given entity (1-hop outgoing).
pub async fn get_edges_from(
    pool: &PgPool,
    tenant_id: Uuid,
    entity_id: Uuid,
) -> Result<Vec<EdgeRow>, GraphError> {
    let rows = sqlx::query_as::<_, EdgeRow>(
        r#"
        SELECT * FROM edges
        WHERE tenant_id = $1
          AND source_entity_id = $2
          AND status = 'active'
          AND (valid_to IS NULL OR valid_to > now())
        ORDER BY weight DESC
        "#,
    )
    .bind(tenant_id)
    .bind(entity_id)
    .fetch_all(pool)
    .await
    .map_err(GraphError::Database)?;
    Ok(rows)
}

/// Get all edges to a given entity (1-hop incoming).
pub async fn get_edges_to(
    pool: &PgPool,
    tenant_id: Uuid,
    entity_id: Uuid,
) -> Result<Vec<EdgeRow>, GraphError> {
    let rows = sqlx::query_as::<_, EdgeRow>(
        r#"
        SELECT * FROM edges
        WHERE tenant_id = $1
          AND target_entity_id = $2
          AND status = 'active'
          AND (valid_to IS NULL OR valid_to > now())
        ORDER BY weight DESC
        "#,
    )
    .bind(tenant_id)
    .bind(entity_id)
    .fetch_all(pool)
    .await
    .map_err(GraphError::Database)?;
    Ok(rows)
}

/// N-hop graph expansion from an entity: returns connected entity IDs with distance.
/// This is a key differentiator over Mem0 (which has no multi-hop traversal)
/// and improves on the fixed 2-hop limitation.
///
/// # Arguments
/// * `max_depth` — Maximum number of hops. Clamped to [1, config_max_depth].
/// * `rel_types` — Optional filter: only traverse edges of these relationship types.
/// * `max_results` — Maximum number of entity IDs to return.
/// * `config_max_depth` — Configurable ceiling for depth (default 5 if None).
pub async fn expand_nhop(
    pool: &PgPool,
    tenant_id: Uuid,
    entity_id: Uuid,
    max_depth: u32,
    rel_types: Option<&[&str]>,
    max_results: i64,
    config_max_depth: Option<u32>,
) -> Result<Vec<(Uuid, u32)>, GraphError> {
    let ceiling = config_max_depth.unwrap_or(5);
    let depth = max_depth.clamp(1, ceiling);

    // Parameterized rel_type filter: use $5::text[] IS NULL OR rel_type = ANY($5)
    let rel_type_vec: Option<Vec<String>> = rel_types
        .filter(|t| !t.is_empty())
        .map(|types| types.iter().map(|t| t.to_string()).collect());

    // Use recursive CTE for variable-depth traversal with parameterized queries only
    let rows = sqlx::query_as::<_, (Uuid, i32)>(
        r#"
        WITH RECURSIVE graph_walk(eid, depth) AS (
            -- Seed: direct neighbors (hop 1)
            SELECT DISTINCT
                CASE WHEN source_entity_id = $2 THEN target_entity_id ELSE source_entity_id END AS eid,
                1 AS depth
            FROM edges
            WHERE tenant_id = $1
              AND (source_entity_id = $2 OR target_entity_id = $2)
              AND status = 'active'
              AND (valid_to IS NULL OR valid_to > now())
              AND ($5::text[] IS NULL OR rel_type = ANY($5))

            UNION

            -- Recursive: expand from previous hop
            SELECT DISTINCT
                CASE WHEN e.source_entity_id = gw.eid THEN e.target_entity_id ELSE e.source_entity_id END,
                gw.depth + 1
            FROM edges e
            JOIN graph_walk gw ON (e.source_entity_id = gw.eid OR e.target_entity_id = gw.eid)
            WHERE e.tenant_id = $1
              AND e.status = 'active'
              AND (e.valid_to IS NULL OR e.valid_to > now())
              AND gw.depth < $3
              AND ($5::text[] IS NULL OR e.rel_type = ANY($5))
        )
        SELECT DISTINCT eid, MIN(depth) as depth
        FROM graph_walk
        WHERE eid != $2
        GROUP BY eid
        ORDER BY depth, eid
        LIMIT $4
        "#,
    )
    .bind(tenant_id)
    .bind(entity_id)
    .bind(depth as i32)
    .bind(max_results)
    .bind(&rel_type_vec)
    .fetch_all(pool)
    .await
    .map_err(GraphError::Database)?;

    let result: Vec<(Uuid, u32)> = rows.into_iter().map(|(id, d)| (id, d as u32)).collect();
    debug!(
        "N-hop expansion (depth={}) from {}: {} entities",
        depth,
        entity_id,
        result.len()
    );
    Ok(result)
}

/// Find shortest path between two entities.
/// Returns the path as a list of entity IDs (including start and end).
/// `config_max_depth` — Configurable ceiling for depth (default 10 if None).
pub async fn find_path(
    pool: &PgPool,
    tenant_id: Uuid,
    start_entity_id: Uuid,
    end_entity_id: Uuid,
    max_depth: u32,
    config_max_depth: Option<u32>,
) -> Result<Option<Vec<Uuid>>, GraphError> {
    let ceiling = config_max_depth.unwrap_or(10);
    let depth = max_depth.clamp(1, ceiling);

    let rows = sqlx::query_as::<_, (Vec<Uuid>,)>(
        r#"
        WITH RECURSIVE path_search(eid, path, depth) AS (
            SELECT $2::uuid, ARRAY[$2::uuid], 0
            UNION ALL
            SELECT
                CASE WHEN e.source_entity_id = ps.eid THEN e.target_entity_id ELSE e.source_entity_id END,
                ps.path || CASE WHEN e.source_entity_id = ps.eid THEN e.target_entity_id ELSE e.source_entity_id END,
                ps.depth + 1
            FROM edges e
            JOIN path_search ps ON (e.source_entity_id = ps.eid OR e.target_entity_id = ps.eid)
            WHERE e.tenant_id = $1
              AND e.status = 'active'
              AND (e.valid_to IS NULL OR e.valid_to > now())
              AND ps.depth < $4
              AND NOT (CASE WHEN e.source_entity_id = ps.eid THEN e.target_entity_id ELSE e.source_entity_id END = ANY(ps.path))
        )
        SELECT path FROM path_search
        WHERE eid = $3
        ORDER BY depth
        LIMIT 1
        "#,
    )
    .bind(tenant_id)
    .bind(start_entity_id)
    .bind(end_entity_id)
    .bind(depth as i32)
    .fetch_optional(pool)
    .await
    .map_err(GraphError::Database)?;

    Ok(rows.map(|(path,)| path))
}

/// Get entity neighbors with full edge details (relationship type, weight, properties).
/// Useful for exploring the graph in the admin panel or API.
pub async fn get_neighbors(
    pool: &PgPool,
    tenant_id: Uuid,
    entity_id: Uuid,
    rel_type: Option<&str>,
    limit: i64,
) -> Result<Vec<NeighborInfo>, GraphError> {
    let rows = sqlx::query_as::<_, NeighborRow>(
        r#"
        SELECT
            CASE WHEN e.source_entity_id = $2 THEN e.target_entity_id ELSE e.source_entity_id END AS neighbor_id,
            ent.name AS neighbor_name,
            ent.entity_type AS neighbor_type,
            e.rel_type,
            e.weight,
            e.properties,
            CASE WHEN e.source_entity_id = $2 THEN 'outgoing' ELSE 'incoming' END AS direction
        FROM edges e
        JOIN entities ent ON ent.id = CASE WHEN e.source_entity_id = $2 THEN e.target_entity_id ELSE e.source_entity_id END
        WHERE e.tenant_id = $1
          AND (e.source_entity_id = $2 OR e.target_entity_id = $2)
          AND e.status = 'active'
          AND ent.status = 'active'
          AND (e.valid_to IS NULL OR e.valid_to > now())
          AND ($3::text IS NULL OR e.rel_type = $3)
        ORDER BY e.weight DESC
        LIMIT $4
        "#,
    )
    .bind(tenant_id)
    .bind(entity_id)
    .bind(rel_type)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(GraphError::Database)?;

    Ok(rows
        .into_iter()
        .map(|r| NeighborInfo {
            entity_id: r.neighbor_id,
            name: r.neighbor_name,
            entity_type: r.neighbor_type,
            rel_type: r.rel_type,
            weight: r.weight,
            properties: r.properties,
            direction: r.direction,
        })
        .collect())
}

/// Neighbor information for graph exploration.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NeighborInfo {
    pub entity_id: Uuid,
    pub name: String,
    pub entity_type: String,
    pub rel_type: String,
    pub weight: f32,
    pub properties: serde_json::Value,
    pub direction: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct NeighborRow {
    neighbor_id: Uuid,
    neighbor_name: String,
    neighbor_type: String,
    rel_type: String,
    weight: f32,
    properties: serde_json::Value,
    direction: String,
}

/// 2-hop graph expansion from an entity: returns connected entity IDs.
pub async fn expand_2hop(
    pool: &PgPool,
    tenant_id: Uuid,
    entity_id: Uuid,
) -> Result<Vec<Uuid>, GraphError> {
    let rows = sqlx::query_scalar::<_, Uuid>(
        r#"
        WITH hop1 AS (
            SELECT target_entity_id AS eid FROM edges
            WHERE tenant_id = $1 AND source_entity_id = $2
              AND status = 'active' AND (valid_to IS NULL OR valid_to > now())
            UNION
            SELECT source_entity_id AS eid FROM edges
            WHERE tenant_id = $1 AND target_entity_id = $2
              AND status = 'active' AND (valid_to IS NULL OR valid_to > now())
        ),
        hop2 AS (
            SELECT target_entity_id AS eid FROM edges
            WHERE tenant_id = $1 AND source_entity_id IN (SELECT eid FROM hop1)
              AND status = 'active' AND (valid_to IS NULL OR valid_to > now())
            UNION
            SELECT source_entity_id AS eid FROM edges
            WHERE tenant_id = $1 AND target_entity_id IN (SELECT eid FROM hop1)
              AND status = 'active' AND (valid_to IS NULL OR valid_to > now())
        )
        SELECT DISTINCT eid FROM (
            SELECT eid FROM hop1
            UNION
            SELECT eid FROM hop2
        ) combined
        WHERE eid != $2
        "#,
    )
    .bind(tenant_id)
    .bind(entity_id)
    .fetch_all(pool)
    .await
    .map_err(GraphError::Database)?;

    debug!(
        "2-hop expansion from {}: {} entities",
        entity_id,
        rows.len()
    );
    Ok(rows)
}

/// Find entities by name pattern (fuzzy search).
pub async fn search_entities_by_name(
    pool: &PgPool,
    tenant_id: Uuid,
    query: &str,
    limit: i64,
) -> Result<Vec<EntityRow>, GraphError> {
    // Escape SQL LIKE wildcards to prevent entity enumeration attacks
    let escaped = query
        .to_lowercase()
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    let pattern = format!("%{}%", escaped);
    let rows = sqlx::query_as::<_, EntityRow>(
        r#"
        SELECT * FROM entities
        WHERE tenant_id = $1
          AND status = 'active'
          AND LOWER(name) LIKE $2 ESCAPE '\'
        ORDER BY updated_at DESC
        LIMIT $3
        "#,
    )
    .bind(tenant_id)
    .bind(&pattern)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(GraphError::Database)?;
    Ok(rows)
}

// ── Row Types ──

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct EntityRow {
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

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct EdgeRow {
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

#[derive(Debug, thiserror::Error)]
pub enum GraphError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}
