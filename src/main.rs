use actix_web::{App, HttpServer, middleware::Logger, web};
use log::info;
use reqwest::Client;
use std::env;
use std::sync::Arc;
use zepm::{config::Config, handlers, state::AppState, types};

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    env_logger::init();

    let port: u16 = env::var("PORT")
        .unwrap_or_else(|_| "3000".to_string())
        .parse()
        .unwrap_or(3000);

    let config = Config::from_env();

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("failed to build reqwest client");

    let state = Arc::new(AppState {
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
            .app_data(web::Data::from(state.clone()))
            .app_data(web::JsonConfig::default().limit(10 * 1024 * 1024))
            .wrap(Logger::default())
            .service(
                web::resource("/predict_edits/v2")
                    .route(web::post().to(handlers::predict_edits_v2)),
            )
            .service(web::resource("/health").route(web::get().to(handlers::health)))
            .service(
                web::resource("/test-completion")
                    .route(
                        web::post().to(|body: web::Json<types::PredictEditsRequest>| {
                            handlers::test_completion(Some(body))
                        }),
                    )
                    .route(web::get().to(|| handlers::test_completion(None))),
            )
    })
    .bind(("0.0.0.0", port))?
    .run()
    .await
}
