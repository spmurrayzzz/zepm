use crate::state::AppState;
use crate::types::{PredictEditsRequest, PredictEditsResponse};
use crate::utils::{CURSOR_MARKER, EDITABLE_REGION_END_MARKER, EDITABLE_REGION_START_MARKER};
use crate::utils::{extract_context, generate_completion, get_fallback_completion};
use actix_web::{HttpResponse, Responder, web};
use chrono::Utc;
use uuid::Uuid;

pub async fn predict_edits_v2(
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

pub async fn health(state: web::Data<AppState>) -> impl Responder {
    HttpResponse::Ok().json(serde_json::json!({
        "status": "ok",
        "llamaServerUrl": state.config.llama_server_url
    }))
}

pub async fn test_completion(body: Option<web::Json<PredictEditsRequest>>) -> impl Responder {
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

pub async fn async_test_completion() -> HttpResponse {
    const SAMPLE_OUTPUT_EXCERPT: &str = "<|editable_region_start|>\n// Generated function example\nfunction processData(input) {\n  // Validate input\n  if (!input) return { error: \"Input is required\" };\n  \n  // Process the data\n  const result = {\n    processed: true,\n    timestamp: new Date().toISOString(),\n    data: input\n  };\n  \n  console.log(\"Data processed successfully\");\n  return result;\n}\n<|editable_region_end|>";

    HttpResponse::Ok().json(PredictEditsResponse {
        request_id: Uuid::new_v4().to_string(),
        output_excerpt: SAMPLE_OUTPUT_EXCERPT.into(),
    })
}
