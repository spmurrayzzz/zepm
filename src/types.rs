use serde::{Deserialize, Serialize};

#[derive(Deserialize, Clone)]
pub struct PredictEditsRequest {
    pub input_excerpt: Option<String>,
    #[allow(dead_code)]
    pub input_events: Option<serde_json::Value>,
    #[allow(dead_code)]
    pub outline: Option<String>,
    #[allow(dead_code)]
    pub speculated_output: Option<String>,
}

#[derive(Serialize, Clone)]
pub struct PredictEditsResponse {
    pub request_id: String,
    pub output_excerpt: String,
}
