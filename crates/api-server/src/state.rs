use std::sync::Arc;

use aissence::Summarizer;
use deadpool_postgres::Pool;

use crate::config::Config;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub pool: Pool,
    pub summarizer: Arc<Summarizer>,
}
