use crate::config::Config;
use crate::handlers::{async_test_completion, health, predict_edits_v2, test_completion};
use crate::state::AppState;
use crate::types::PredictEditsRequest;
use crate::utils::{extract_context, get_fallback_completion};
use actix_web::{App, Responder, test, web};
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
    use crate::utils::{CURSOR_MARKER, EDITABLE_REGION_END_MARKER, EDITABLE_REGION_START_MARKER};
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
    use crate::utils::{CURSOR_MARKER, EDITABLE_REGION_END_MARKER, EDITABLE_REGION_START_MARKER};
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

#[tokio::test]
async fn test_generate_completion_error() {
    use crate::utils::generate_completion;
    let client = Client::builder()
        .timeout(std::time::Duration::from_millis(10))
        .build()
        .unwrap();
    let config = test_config();
    use crate::utils::{CURSOR_MARKER, EDITABLE_REGION_END_MARKER, EDITABLE_REGION_START_MARKER};
    let input_excerpt = format!(
        "{}\nlet x = 1;\n{}\n{}",
        EDITABLE_REGION_START_MARKER, CURSOR_MARKER, EDITABLE_REGION_END_MARKER
    );
    let result = generate_completion(&client, &config, &input_excerpt, false).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_predict_edits_v2_fallback() {
    use crate::utils::{CURSOR_MARKER, EDITABLE_REGION_END_MARKER, EDITABLE_REGION_START_MARKER};
    init_logger();
    let mut app = test::init_service(
        App::new()
            .app_data(web::Data::new(test_app_state()))
            .service(web::resource("/predict_edits/v2").route(web::post().to(predict_edits_v2))),
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
    use crate::utils::{CURSOR_MARKER, EDITABLE_REGION_END_MARKER, EDITABLE_REGION_START_MARKER};
    use httpmock::Method::POST;
    use httpmock::MockServer;

    init_logger();

    let server = MockServer::start_async().await;

    let mock = server
        .mock_async(|when, then| {
            when.method(POST).path("/infill");
            then.status(200).json_body_obj(&serde_json::json!({
                "content": "// LLM completion here"
            }));
        })
        .await;

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

    let mut app = test::init_service(
        App::new()
            .app_data(state)
            .service(web::resource("/predict_edits/v2").route(web::post().to(predict_edits_v2))),
    )
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
