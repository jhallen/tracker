//! Database connection pool and embedded migrations.

use diesel::{
    prelude::*,
    r2d2::{ConnectionManager, CustomizeConnection, Pool, PooledConnection},
    sqlite::SqliteConnection,
};
use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};

use crate::error::AppError;

pub type DbPool = Pool<ConnectionManager<SqliteConnection>>;
pub type DbConn = PooledConnection<ConnectionManager<SqliteConnection>>;

/// The SQL files under `migrations/` are compiled into the binary so the
/// server can bring a database up to date by itself (`AUTO_MIGRATE=1` or
/// `greenhouse-tracker migrate`). The Diesel CLI uses the same files.
pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");

/// Per-connection SQLite settings.
#[derive(Debug)]
struct SqliteTuning;

impl CustomizeConnection<SqliteConnection, diesel::r2d2::Error> for SqliteTuning {
    fn on_acquire(&self, conn: &mut SqliteConnection) -> Result<(), diesel::r2d2::Error> {
        diesel::sql_query(
            "PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000; PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;",
        )
        .execute(conn)
        .map(|_| ())
        .map_err(diesel::r2d2::Error::QueryError)
    }
}

pub fn build_pool(database_url: &str) -> anyhow::Result<DbPool> {
    let manager = ConnectionManager::<SqliteConnection>::new(database_url);
    let pool = Pool::builder()
        .max_size(8)
        .connection_customizer(Box::new(SqliteTuning))
        .build(manager)?;
    Ok(pool)
}

pub fn run_migrations(pool: &DbPool) -> anyhow::Result<Vec<String>> {
    let mut conn = pool.get()?;
    let applied = conn
        .run_pending_migrations(MIGRATIONS)
        .map_err(|e| anyhow::anyhow!("migration failed: {e}"))?;
    Ok(applied.iter().map(|v| v.to_string()).collect())
}

/// Run blocking Diesel work on the blocking thread pool.
pub async fn run<F, T>(pool: &DbPool, f: F) -> Result<T, AppError>
where
    F: FnOnce(&mut SqliteConnection) -> Result<T, AppError> + Send + 'static,
    T: Send + 'static,
{
    let pool = pool.clone();
    tokio::task::spawn_blocking(move || {
        let mut conn = pool.get().map_err(|e| AppError::Internal(e.to_string()))?;
        f(&mut conn)
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?
}
