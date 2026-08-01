use std::str::FromStr;

use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod, Runtime};
use tokio_postgres::NoTls;

pub fn create_pool(database_url: &str) -> Result<Pool, String> {
    let postgres = tokio_postgres::Config::from_str(database_url)
        .map_err(|error| format!("invalid DATABASE_URL: {error}"))?;
    let manager = Manager::from_config(
        postgres,
        NoTls,
        ManagerConfig {
            recycling_method: RecyclingMethod::Fast,
        },
    );
    Pool::builder(manager)
        .runtime(Runtime::Tokio1)
        .max_size(16)
        .build()
        .map_err(|error| format!("cannot create database pool: {error}"))
}

pub async fn migrate(pool: &Pool) -> Result<(), String> {
    let client = pool
        .get()
        .await
        .map_err(|error| format!("cannot acquire database connection: {error}"))?;
    client
        .batch_execute(include_str!("../migrations/0001_remote_summarization.sql"))
        .await
        .map_err(|error| format!("database migration failed: {error}"))
}
