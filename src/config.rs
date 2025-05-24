use serde::{Deserialize, Serialize};
use std::env;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub llama_server_url: String,
    pub debug: bool,
    pub max_tokens: u32,
    pub temperature: f32,
    pub fallback_mode: bool,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            llama_server_url: env::var("LLAMA_SERVER_URL")
                .unwrap_or_else(|_| "http://localhost:8080".into()),
            debug: env::var("DEBUG")
                .map(|v| v == "1" || v == "true")
                .unwrap_or(true),
            max_tokens: env::var("MAX_TOKENS")
                .unwrap_or_else(|_| "128".into())
                .parse()
                .unwrap_or(128),
            temperature: env::var("TEMPERATURE")
                .unwrap_or_else(|_| "0.1".into())
                .parse()
                .unwrap_or(0.2),
            fallback_mode: env::var("FALLBACK_MODE")
                .map(|v| v == "1" || v == "true")
                .unwrap_or(false),
        }
    }
}
