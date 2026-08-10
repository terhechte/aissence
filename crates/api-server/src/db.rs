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
        .batch_execute(&format!(
            "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
            include_str!("../../../frontend/supabase/migrations/202608010002_remote_ingestion.sql"),
            include_str!("../../../frontend/supabase/migrations/202608020001_summary_tiers.sql"),
            include_str!(
                "../../../frontend/supabase/migrations/202608020002_optional_project_copy.sql"
            ),
            include_str!(
                "../../../frontend/supabase/migrations/202608050001_public_local_prompt_summaries.sql"
            ),
            include_str!(
                "../../../frontend/supabase/migrations/202608080001_commit_subline.sql"
            ),
            include_str!(
                "../../../frontend/supabase/migrations/202608080002_summary_quality_and_drafts.sql"
            ),
            include_str!(
                "../../../frontend/supabase/migrations/202608080003_atomic_publish.sql"
            ),
            include_str!(
                "../../../frontend/supabase/migrations/202608090001_summary_only_commit_copy.sql"
            ),
        ))
        .await
        .map_err(|error| format!("database migration failed: {error}"))
}
