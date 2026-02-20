//! Database layer: connection pool, migration runner, and RLS helpers.

use sqlx::postgres::{PgPool, PgPoolOptions};
use tracing::info;
use uuid::Uuid;

/// Create a new Postgres connection pool.
pub async fn create_pool(database_url: &str, max_connections: u32) -> Result<PgPool, sqlx::Error> {
    let max_connections = std::env::var("DB_MAX_CONNECTIONS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(max_connections);
    let min_connections = std::env::var("DB_MIN_CONNECTIONS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(1);

    let pool = PgPoolOptions::new()
        .min_connections(min_connections.min(max_connections))
        .max_connections(max_connections)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .idle_timeout(Some(std::time::Duration::from_secs(300)))
        .max_lifetime(Some(std::time::Duration::from_secs(1800)))
        .connect(database_url)
        .await?;
    info!(
        "Database pool created with min_connections={} max_connections={}",
        min_connections.min(max_connections),
        max_connections
    );
    Ok(pool)
}

/// Run embedded migrations from the migrations/ directory.
pub async fn run_migrations(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    info!("Running database migrations...");
    let mut migrator = sqlx::migrate!("../../migrations");
    // OSS and enterprise services can use different migration sets but the same
    // database. Ignore already-applied versions that are not in this set.
    migrator.set_ignore_missing(true);
    migrator.run(pool).await?;
    info!("Migrations complete");
    Ok(())
}

/// Set the RLS tenant context on a connection.
/// Must be called before any tenant-scoped query.
///
/// Note: PostgreSQL `SET LOCAL` does not support parameterized queries ($1),
/// so we use `format!` with the UUID type which guarantees a safe string
/// representation (hyphenated hex only, no SQL metacharacters).
pub async fn set_tenant_context(
    conn: &mut sqlx::PgConnection,
    tenant_id: Uuid,
) -> Result<(), sqlx::Error> {
    // Uuid::to_string() is guaranteed to produce only [0-9a-f-], safe for interpolation
    let tid = tenant_id.to_string();
    debug_assert!(
        tid.chars().all(|c| c.is_ascii_hexdigit() || c == '-'),
        "UUID produced unexpected characters"
    );
    sqlx::query(&format!("SET LOCAL app.tenant_id = '{}'", tid))
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Acquire a connection with tenant context already set.
pub async fn acquire_tenant_conn(
    pool: &PgPool,
    tenant_id: Uuid,
) -> Result<sqlx::pool::PoolConnection<sqlx::Postgres>, sqlx::Error> {
    let mut conn = pool.acquire().await?;
    set_tenant_context(conn.as_mut(), tenant_id).await?;
    Ok(conn)
}

/// Helper to begin a transaction with tenant context set.
pub async fn begin_tenant_tx(
    pool: &PgPool,
    tenant_id: Uuid,
) -> Result<sqlx::Transaction<'static, sqlx::Postgres>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let tid = tenant_id.to_string();
    debug_assert!(
        tid.chars().all(|c| c.is_ascii_hexdigit() || c == '-'),
        "UUID produced unexpected characters"
    );
    sqlx::query(&format!("SET LOCAL app.tenant_id = '{}'", tid))
        .execute(&mut *tx)
        .await?;
    Ok(tx)
}
