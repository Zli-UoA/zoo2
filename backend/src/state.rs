use crate::config::Config;

#[derive(Clone)]
pub struct AppState {
    pub db: toasty::Db,
    pub config: Config,
}

impl AppState {
    pub fn new(db: toasty::Db, config: Config) -> Self {
        Self { db, config }
    }
}
