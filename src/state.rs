use crate::config::Config;
use reqwest::Client;

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub client: Client,
}
