use actix_web::{App, HttpResponse, HttpServer, Responder, middleware::Logger, web};
use chrono::Utc;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::env;
use uuid::Uuid;

use log::info;

const CURSOR_MARKER: &str = "<|user_cursor_is_here|>";
const EDITABLE_REGION_START_MARKER: &str = "<|editable_region_start|>";
const EDITABLE_REGION_END_MARKER: &str = "<|editable_region_end|>";

#[derive(Clone)]
struct Config {
    llama_server_url: String,
    debug: bool,
    max_tokens: u32,
    temperature: f32,
    fallback_mode: bool,
}

#[derive(Clone)]
struct AppState {
    config: Config,
    client: Client,
}

#[derive(Deserialize, Clone)]
struct PredictEditsRequest {
    input_excerpt: Option<String>,
    #[allow(dead_code)]
    input_events: Option<serde_json::Value>,
    #[allow(dead_code)]
    outline: Option<String>,
    #[allow(dead_code)]
    speculated_output: Option<String>,
}

#[derive(Serialize, Clone)]
struct PredictEditsResponse {
    request_id: String,
    output_excerpt: String,
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    env_logger::init();

    let port: u16 = env::var("PORT")
        .unwrap_or_else(|_| "3000".to_string())
        .parse()
        .unwrap_or(3000);

    let config = Config {
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
    };

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("failed to build reqwest client");

    let state = web::Data::new(AppState {
        config: config.clone(),
        client,
    });

    let _ = env_logger::try_init();

    info!(
        "\n┌───────────────────────────────────────────┐\
         \n│    Zed FIM Server - Troubleshooting Mode  │\
         \n└───────────────────────────────────────────┘\n"
    );
    info!("Server running at http://localhost:{port}");
    info!("Using llama.cpp server at: {}", config.llama_server_url);
    info!(
        "\nDebug mode: {}",
        if config.debug { "ENABLED" } else { "disabled" }
    );
    info!(
        "Fallback mode: {}",
        if config.fallback_mode {
            "ENABLED"
        } else {
            "disabled"
        }
    );
    info!("Temperature: {}", config.temperature);
    info!("Max tokens: {}", config.max_tokens);
    info!("\n=== USAGE INSTRUCTIONS ===");
    info!("To use with Zed, run:");
    info!("ZED_PREDICT_EDITS_URL=http://localhost:{port}/predict_edits/v2 zed\n");
    info!("=== TROUBLESHOOTING OPTIONS ===");
    info!("For testing with a hardcoded completion, run:");
    info!("ZED_PREDICT_EDITS_URL=http://localhost:{port}/test-completion zed\n");
    info!("URL parameters for debugging:");
    info!("  ?debug=true  - Enable detailed debug output");
    info!("  ?fallback=true - Use static completions without calling LLM");
    info!("  ?force=true  - Force completions regardless of cursor position");

    HttpServer::new(move || {
        App::new()
            .app_data(state.clone())
            .app_data(web::JsonConfig::default().limit(10 * 1024 * 1024))
            .wrap(Logger::default())
            .service(web::resource("/predict_edits/v2").route(web::post().to(predict_edits_v2)))
            .service(web::resource("/health").route(web::get().to(health)))
            .service(web::resource("/test-completion").route(
                web::post().to(|body: web::Json<PredictEditsRequest>| test_completion(Some(body))),
            ))
            .service(
                web::resource("/test-completion").route(web::get().to(|| test_completion(None))),
            )
    })
    .bind(("0.0.0.0", port))?
    .run()
    .await
}

async fn predict_edits_v2(
    state: web::Data<AppState>,
    req: actix_web::HttpRequest,
    body: web::Json<PredictEditsRequest>,
) -> impl Responder {
    log::info!("\n==== Predict Edits Request Started ====");
    let query = req.query_string();
    let force_debug = query.contains("debug=true");
    let force_fallback = query.contains("fallback=true") || state.config.fallback_mode;
    let force_completion = query.contains("force=true");

    let should_debug = state.config.debug || force_debug;

    if should_debug {
        log::debug!(
            "[{}] POST /predict_edits/v2 (query: {})",
            Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            query
        );
    }

    let input_excerpt = match &body.input_excerpt {
        Some(text) => text.clone(),
        _none => {
            return HttpResponse::BadRequest().json(serde_json::json!({
                "error": "Missing input_excerpt"
            }));
        }
    };

    log::debug!("\n==== REQUEST INFO ====");
    log::debug!("Input excerpt length: {} chars", input_excerpt.len());
    log::debug!(
        "Contains editable start: {}",
        input_excerpt.contains(EDITABLE_REGION_START_MARKER)
    );
    log::debug!(
        "Contains editable end: {}",
        input_excerpt.contains(EDITABLE_REGION_END_MARKER)
    );
    log::debug!(
        "Contains cursor marker: {}",
        input_excerpt.contains(CURSOR_MARKER)
    );

    if force_fallback {
        log::info!("\nUsing FALLBACK mode (static completions)");

        if let Ok((_, _, cursor_pos)) = extract_context(&input_excerpt) {
            let fallback_code = get_fallback_completion(&input_excerpt);

            log::info!(
                "Inserting fallback completion at cursor position {}",
                cursor_pos
            );
            log::info!("Fallback completion length: {} chars", fallback_code.len());

            let output_excerpt = format!(
                "{}{}{}",
                &input_excerpt[..cursor_pos],
                fallback_code,
                &input_excerpt[cursor_pos + CURSOR_MARKER.len()..]
            );

            return HttpResponse::Ok().json(PredictEditsResponse {
                request_id: Uuid::new_v4().to_string(),
                output_excerpt,
            });
        } else {
            log::warn!("Cursor marker not found, using complete template instead");
            return async_test_completion().await;
        }
    }

    let mut config_copy = state.config.clone();
    if force_debug {
        config_copy.debug = true;
    }

    match generate_completion(
        &state.client,
        &config_copy,
        &input_excerpt,
        force_completion,
    )
    .await
    {
        Ok(output_excerpt) => {
            log::debug!("\n==== RESPONSE INFO ====");
            log::debug!("Output excerpt length: {} chars", output_excerpt.len());
            log::debug!(
                "Contains editable start: {}",
                output_excerpt.contains(EDITABLE_REGION_START_MARKER)
            );
            log::debug!(
                "Contains editable end: {}",
                output_excerpt.contains(EDITABLE_REGION_END_MARKER)
            );
            log::debug!(
                "Contains cursor marker: {}",
                output_excerpt.contains(CURSOR_MARKER)
            );
            log::info!("Returning completion to IDE");

            let response_obj = PredictEditsResponse {
                request_id: Uuid::new_v4().to_string(),
                output_excerpt,
            };

            HttpResponse::Ok()
                .insert_header(("Cache-Control", "no-cache"))
                .json(response_obj)
        }
        Err(err) => {
            log::error!("Error generating completion: {err}");
            log::warn!("Error occurred, falling back to static completion");
            async_test_completion().await
        }
    }
}

async fn health(state: web::Data<AppState>) -> impl Responder {
    HttpResponse::Ok().json(serde_json::json!({
        "status": "ok",
        "llamaServerUrl": state.config.llama_server_url
    }))
}

async fn test_completion(body: Option<web::Json<PredictEditsRequest>>) -> impl Responder {
    match body {
        Some(json_body) => {
            if let Some(input_excerpt) = &json_body.input_excerpt {
                if input_excerpt.contains(CURSOR_MARKER) {
                    if let Ok((_, _, cursor_pos)) = extract_context(input_excerpt) {
                        let fallback_code = "// Test completion inserted\nconsole.log(\"This is a test completion\");\n// End of test completion";

                        let output_excerpt = format!(
                            "{}{}{}",
                            &input_excerpt[..cursor_pos],
                            fallback_code,
                            &input_excerpt[cursor_pos + CURSOR_MARKER.len()..]
                        );

                        return HttpResponse::Ok().json(PredictEditsResponse {
                            request_id: Uuid::new_v4().to_string(),
                            output_excerpt,
                        });
                    }
                }
            }
        }
        _none => {}
    }

    async_test_completion().await
}

async fn async_test_completion() -> HttpResponse {
    const SAMPLE_OUTPUT_EXCERPT: &str = "<|editable_region_start|>\n// Generated function example\nfunction processData(input) {\n  // Validate input\n  if (!input) return { error: \"Input is required\" };\n  \n  // Process the data\n  const result = {\n    processed: true,\n    timestamp: new Date().toISOString(),\n    data: input\n  };\n  \n  console.log(\"Data processed successfully\");\n  return result;\n}\n<|editable_region_end|>";

    HttpResponse::Ok().json(PredictEditsResponse {
        request_id: Uuid::new_v4().to_string(),
        output_excerpt: SAMPLE_OUTPUT_EXCERPT.into(),
    })
}

fn get_fallback_completion(input_excerpt: &str) -> String {
    let file_extension = if input_excerpt.contains(".js")
        || input_excerpt.contains("function")
        || input_excerpt.contains("const")
    {
        "js"
    } else if input_excerpt.contains(".py")
        || input_excerpt.contains("def ")
        || input_excerpt.contains("import ")
    {
        "py"
    } else if input_excerpt.contains(".rs")
        || input_excerpt.contains("fn ")
        || input_excerpt.contains("pub ")
        || input_excerpt.contains("struct ")
    {
        "rs"
    } else {
        "txt"
    };

    match file_extension {
        "js" => "// Generated JS code\nconsole.log(\"Processing data...\");\nconst result = {};\nreturn result;".into(),
        "py" => "# Generated Python code\nprint(\"Processing data...\")\nresult = {}\nreturn result".into(),
        "rs" => "// Generated Rust code\nprintln!(\"Processing data...\");\nlet result = String::from(\"done\");\nresult".into(),
        _ => "// Generated code\n// Replace with your implementation".into(),
    }
}

async fn generate_completion(
    client: &Client,
    config: &Config,
    input_excerpt: &str,
    force_completion: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    if config.debug {
        log::debug!(
            "\n==== RAW INPUT EXCERPT (to generate_completion) ====\n{}\n===================================================",
            input_excerpt
        );
    }

    let (llm_prefix_clean, llm_suffix_clean, _original_cursor_pos) =
        extract_context(input_excerpt)?;

    if config.debug {
        log::debug!("==== REQUEST DETAILS ====");
        log::debug!(
            "Calling llama.cpp infill with prefix ({} chars) and suffix ({} chars)",
            llm_prefix_clean.len(),
            llm_suffix_clean.len()
        );
        log::debug!(
            "Prefix sample: \"{}\"",
            llm_prefix_clean.chars().take(50).collect::<String>()
        );
        log::debug!(
            "Suffix sample: \"{}\"",
            llm_suffix_clean.chars().take(50).collect::<String>()
        );
        log::debug!("Using temperature: {}", config.temperature);
        log::debug!("Max tokens: {}", config.max_tokens);
    }

    let request_body = serde_json::json!({
        "input_prefix": llm_prefix_clean,
        "input_suffix": llm_suffix_clean,
        "n_predict": config.max_tokens,
        "temperature": config.temperature,
        "stream": false,
        "cache_prompt": true,
        "t_max_predict_ms": 5_000,
        "top_k": 40,
        "top_p": 0.99,
        "samplers": ["top_k", "top_p", "infill"]
    });

    if config.debug {
        log::debug!(
            "Request payload: {}",
            serde_json::to_string_pretty(&request_body).unwrap_or_default()
        );
    }

    let resp = match client
        .post(format!("{}/infill", config.llama_server_url))
        .json(&request_body)
        .send()
        .await
    {
        Ok(response) => {
            if !response.status().is_success() {
                let status = response.status();
                let error_text = response
                    .text()
                    .await
                    .unwrap_or_else(|_| "Failed to get error body".into());
                log::error!(
                    "Error from llama.cpp server: Status {}, Body: {}",
                    status,
                    error_text
                );
                return Err(format!("LLM server error: HTTP {}", status).into());
            }
            response
        }
        Err(e) => {
            log::error!("Failed to connect to llama.cpp server: {e}");
            return Err(format!("Connection error: {}", e).into());
        }
    };

    let json = match resp.json::<serde_json::Value>().await {
        Ok(json) => json,
        Err(e) => {
            log::error!("Failed to parse JSON response: {e}");
            return Err(format!("JSON parsing error: {}", e).into());
        }
    };

    let completion = match json.get("content") {
        Some(content) => content.as_str().unwrap_or_default(),
        _none => {
            log::warn!("No 'content' field in response: {:?}", json);
            ""
        }
    };

    log::info!(
        "Got completion ({} chars): \"{}...\"",
        completion.len(),
        completion.chars().take(50).collect::<String>()
    );

    let clean_completion = if completion.contains(CURSOR_MARKER) {
        completion.replace(CURSOR_MARKER, "")
    } else {
        completion.to_string()
    };

    let is_useful_completion = true;

    if !is_useful_completion || force_completion {
        log::warn!(
            "Received low-quality completion or force mode enabled, using defined completion"
        );

        let fallback_completion = if force_completion {
            "    // Generated fields\n    server_name: String,\n    version: String,\n    initialized_at: std::time::SystemTime,\n    \n    // Configuration\n    max_connections: usize,\n    timeout_seconds: u64,".to_string()
        } else {
            get_fallback_completion(input_excerpt)
        };

        log::debug!(
            "Using {} completion",
            if force_completion {
                "forced"
            } else {
                "fallback"
            }
        );
        log::debug!(
            "At position: {} in a string of length: {}",
            _original_cursor_pos,
            input_excerpt.len()
        );

        let output_excerpt = if _original_cursor_pos + CURSOR_MARKER.len() <= input_excerpt.len() {
            format!(
                "{}{}{}",
                &input_excerpt[.._original_cursor_pos],
                fallback_completion,
                &input_excerpt[_original_cursor_pos + CURSOR_MARKER.len()..]
            )
        } else {
            format!(
                "{}{}",
                &input_excerpt[.._original_cursor_pos],
                fallback_completion
            )
        };

        return Ok(output_excerpt);
    }

    log::info!("Using generated completion: \"{}\"", clean_completion);

    let cursor_marker_len = CURSOR_MARKER.len();
    let output_excerpt = if _original_cursor_pos + cursor_marker_len <= input_excerpt.len() {
        format!(
            "{}{}{}",
            &input_excerpt[.._original_cursor_pos],
            clean_completion,
            &input_excerpt[_original_cursor_pos + cursor_marker_len..]
        )
    } else {
        format!(
            "{}{}",
            &input_excerpt[.._original_cursor_pos],
            clean_completion
        )
    };

    Ok(output_excerpt)
}

fn extract_context(
    input_excerpt: &str,
) -> Result<(String, String, usize), Box<dyn std::error::Error>> {
    let editable_start_tag = EDITABLE_REGION_START_MARKER;
    let editable_end_tag = EDITABLE_REGION_END_MARKER;
    let cursor_tag = CURSOR_MARKER;

    let editable_content_start_offset = input_excerpt
        .find(editable_start_tag)
        .map(|pos| pos + editable_start_tag.len())
        .ok_or_else(|| {
            format!(
                "EDITABLE_REGION_START_MARKER ('{}') not found",
                editable_start_tag
            )
        })?;

    let editable_content_end_offset = input_excerpt.find(editable_end_tag).ok_or_else(|| {
        format!(
            "EDITABLE_REGION_END_MARKER ('{}') not found",
            editable_end_tag
        )
    })?;

    if editable_content_start_offset >= editable_content_end_offset {
        return Err(format!(
            "Editable region is invalid or empty: start_offset ({}) >= end_offset ({})",
            editable_content_start_offset, editable_content_end_offset
        )
        .into());
    }

    let editable_content_slice =
        &input_excerpt[editable_content_start_offset..editable_content_end_offset];

    let cursor_pos_in_editable_slice = editable_content_slice.rfind(cursor_tag)
         .ok_or_else(|| format!("CURSOR_MARKER ('{}') not found within the isolated editable content slice of length {}", cursor_tag, editable_content_slice.len()))?;

    let global_cursor_pos = editable_content_start_offset + cursor_pos_in_editable_slice;

    if !(editable_content_start_offset <= global_cursor_pos
        && (global_cursor_pos + cursor_tag.len()) <= editable_content_end_offset)
    {
        return Err(format!(
            "CURSOR_MARKER (found at global_pos {} via rfind) is not within the original editable \
            region bounds ({}..{})",
            global_cursor_pos, editable_content_start_offset, editable_content_end_offset
        )
        .into());
    }

    let llm_prefix = editable_content_slice[..cursor_pos_in_editable_slice].to_string();

    let llm_suffix_start_pos_in_slice = cursor_pos_in_editable_slice + cursor_tag.len();
    let llm_suffix = editable_content_slice[llm_suffix_start_pos_in_slice..].to_string();

    Ok((llm_prefix, llm_suffix, global_cursor_pos))
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{App, test, web};
    use reqwest::Client;
    use serde_json::json;
    use std::sync::Once;

    static INIT: Once = Once::new();

    fn init_logger() {
        INIT.call_once(|| {
            let _ = env_logger::builder().is_test(true).try_init();
        });
    }

    fn test_config() -> Config {
        Config {
            llama_server_url: "http://localhost:1234".to_string(),
            debug: true,
            max_tokens: 64,
            temperature: 0.5,
            fallback_mode: false,
        }
    }

    fn test_app_state() -> AppState {
        AppState {
            config: test_config(),
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .expect("failed to build test reqwest client"),
        }
    }

    #[tokio::test]
    async fn test_health_ok() {
        init_logger();
        let state = web::Data::new(test_app_state());
        let resp = health(state.clone())
            .await
            .respond_to(&test::TestRequest::default().to_http_request());
        assert_eq!(resp.status(), actix_web::http::StatusCode::OK);
    }

    #[tokio::test]
    async fn test_get_fallback_completion_js() {
        let input = "function foo() {}";
        let result = get_fallback_completion(input);
        assert!(result.contains("Generated JS code"));
    }

    #[tokio::test]
    async fn test_get_fallback_completion_py() {
        let input = "def foo(): pass";
        let result = get_fallback_completion(input);
        assert!(result.contains("Generated Python code"));
    }

    #[tokio::test]
    async fn test_get_fallback_completion_rs() {
        let input = "fn foo() {}";
        let result = get_fallback_completion(input);
        assert!(result.contains("Generated Rust code"));
    }

    #[tokio::test]
    async fn test_extract_context_success() {
        let input = format!(
            "{}\nfn foo() {{}}\n{}\n{}",
            EDITABLE_REGION_START_MARKER, CURSOR_MARKER, EDITABLE_REGION_END_MARKER
        );
        let res = extract_context(&input);
        assert!(res.is_ok());
        let (prefix, suffix, pos) = res.unwrap();
        assert!(prefix.contains("fn foo()"));
        assert_eq!(suffix, "\n");
        assert!(pos > 0);
    }

    #[tokio::test]
    async fn test_extract_context_missing_markers() {
        let input = "let x = 1;";
        let res = extract_context(input);
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn test_async_test_completion_returns_sample() {
        let resp = async_test_completion().await;
        let body = actix_web::body::to_bytes(resp.into_body()).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            json["output_excerpt"]
                .as_str()
                .unwrap()
                .contains("Generated function example")
        );
    }

    #[tokio::test]
    async fn test_test_completion_with_cursor() {
        let input = format!(
            "{}\nfn foo() {{}}\n{}\n{}",
            EDITABLE_REGION_START_MARKER, CURSOR_MARKER, EDITABLE_REGION_END_MARKER
        );
        let req = PredictEditsRequest {
            input_excerpt: Some(input),
            input_events: None,
            outline: None,
            speculated_output: None,
        };
        let body = Some(web::Json(req));
        let resp = test_completion(body)
            .await
            .respond_to(&test::TestRequest::default().to_http_request());
        assert_eq!(resp.status(), actix_web::http::StatusCode::OK);
    }

    // Stub for generate_completion: would require HTTP mocking for full test
    #[tokio::test]
    async fn test_generate_completion_error() {
        let client = Client::builder()
            .timeout(std::time::Duration::from_millis(10))
            .build()
            .unwrap();
        let config = test_config();
        let input_excerpt = format!(
            "{}\nlet x = 1;\n{}\n{}",
            EDITABLE_REGION_START_MARKER, CURSOR_MARKER, EDITABLE_REGION_END_MARKER
        );
        // Use an unreachable URL to force error
        let result = generate_completion(&client, &config, &input_excerpt, false).await;
        assert!(result.is_err());
    }

    // Integration test stub for predict_edits_v2 (requires actix_web::test and HTTP mocking)
    #[tokio::test]
    async fn test_predict_edits_v2_fallback() {
        init_logger();
        let mut app = test::init_service(
            App::new()
                .app_data(web::Data::new(test_app_state()))
                .service(
                    web::resource("/predict_edits/v2").route(web::post().to(predict_edits_v2)),
                ),
        )
        .await;

        let input = format!(
            "{}\nfn foo() {{}}\n{}\n{}",
            EDITABLE_REGION_START_MARKER, CURSOR_MARKER, EDITABLE_REGION_END_MARKER
        );
        let req_body = json!({
            "input_excerpt": input,
            "input_events": null,
            "outline": null,
            "speculated_output": null
        });

        let req = test::TestRequest::post()
            .uri("/predict_edits/v2?fallback=true")
            .set_json(&req_body)
            .to_request();

        let resp = test::call_service(&mut app, req).await;
        assert_eq!(resp.status(), actix_web::http::StatusCode::OK);
        let body = actix_web::body::to_bytes(resp.into_body()).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let output_excerpt = json["output_excerpt"].as_str().unwrap();
        if !output_excerpt.contains("Generated Rust code") {
            println!("DEBUG: output_excerpt = {:?}", output_excerpt);
        }
        assert!(output_excerpt.contains("Generated Rust code"));
    }

    #[tokio::test]
    async fn test_predict_edits_v2_real_llm_path() {
        use httpmock::Method::POST;
        use httpmock::MockServer;

        init_logger();

        // Start mock LLM server
        let server = MockServer::start_async().await;

        // Mock /infill endpoint
        let mock = server
            .mock_async(|when, then| {
                when.method(POST).path("/infill");
                then.status(200).json_body_obj(&serde_json::json!({
                    "content": "// LLM completion here"
                }));
            })
            .await;

        // Prepare AppState with mock server URL
        let config = Config {
            llama_server_url: server.url(""),
            debug: false,
            max_tokens: 64,
            temperature: 0.5,
            fallback_mode: false,
        };
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap();
        let state = web::Data::new(AppState { config, client });

        let mut app =
            test::init_service(App::new().app_data(state).service(
                web::resource("/predict_edits/v2").route(web::post().to(predict_edits_v2)),
            ))
            .await;

        let input = format!(
            "{}\nfn foo() {{}}\n{}\n{}",
            EDITABLE_REGION_START_MARKER, CURSOR_MARKER, EDITABLE_REGION_END_MARKER
        );
        let req_body = serde_json::json!({
            "input_excerpt": input,
            "input_events": null,
            "outline": null,
            "speculated_output": null
        });

        let req = test::TestRequest::post()
            .uri("/predict_edits/v2")
            .set_json(&req_body)
            .to_request();

        let resp = test::call_service(&mut app, req).await;
        assert_eq!(resp.status(), actix_web::http::StatusCode::OK);
        let body = actix_web::body::to_bytes(resp.into_body()).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let output_excerpt = json["output_excerpt"].as_str().unwrap();
        assert!(output_excerpt.contains("// LLM completion here"));
        mock.assert_async().await;
    }
}
