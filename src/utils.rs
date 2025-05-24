use crate::config::Config;
use reqwest::Client;

pub const CURSOR_MARKER: &str = "<|user_cursor_is_here|>";
pub const EDITABLE_REGION_START_MARKER: &str = "<|editable_region_start|>";
pub const EDITABLE_REGION_END_MARKER: &str = "<|editable_region_end|>";

pub fn extract_context(
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

pub fn get_fallback_completion(input_excerpt: &str) -> String {
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

pub async fn generate_completion(
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
