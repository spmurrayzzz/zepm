# zepm

_Pronounced "zep-em"_

Rust implementation of middleware for Zed's Edit Prediction
feature. This server is meant to sit between Zed and a llama.cpp server using a FIM-compatible model. The server expects the interface found in the zeta [InlineCompletion struct](https://github.com/zed-industries/zed/blob/6f918ed99bfa107d496f7e6a7101a956494f3153/crates/zeta/src/zeta.rs#L98-L112).

## Endpoints

- `POST /predict_edits/v2` - Main endpoint for text completion
- `GET /health` - Health check endpoint
- `GET /test-completion` - Test endpoint with hardcoded completion
- `POST /diagnostics` - Advanced diagnostics endpoint for troubleshooting

## Special Text Markers

The server recognizes special markers in the text:
- `<|user_cursor_is_here|>` - Cursor position marker
- `<|editable_region_start|>` - Start of editable region
- `<|editable_region_end|>` - End of editable region

## Usage

1. Start the server w/ logging:
```bash
RUST_LOG=info cargo run
```

2. Connect with Zed editor:
```bash
ZED_PREDICT_EDITS_URL=http://localhost:3000/predict_edits/v2 zed
```

For testing with a hardcoded completion:
```bash
ZED_PREDICT_EDITS_URL=http://localhost:3000/test-completion zed
```

## Environment Variables

- `PORT` - Server port (default: 3000)
- `LLAMA_SERVER_URL` - URL of the llama.cpp server (default: http://localhost:8080)
- `MAX_TOKENS` - Maximum tokens to generate (default: 128)
- `TEMPERATURE` - Temperature for generation (default: 0)

## Requirements

- Rust 1.58+
- A running llama.cpp server with infill capabilities

## Troubleshooting

### Common Issues

1. **LLM returns only cursor markers** - Sometimes the LLM returns `<|cursor|>` instead of actual completions. The server now adds fallback text if this happens.

2. **LLM connection errors** - Ensure your llama.cpp server is running and accessible at the URL specified in LLAMA_SERVER_URL.

3. **Empty or insufficient completions** - If completions are empty or not useful:
   - Try increasing MAX_TOKENS (default: 128)
   - Adjust TEMPERATURE (lower is usually what you want for more deterministic completions)
   - Ensure your cursor is placed in a location where completion makes sense

### Diagnostics

Use the `/diagnostics` endpoint to directly test the LLM server integration:

```bash
curl -X POST http://localhost:3000/diagnostics -H "Content-Type: application/json" \
  -d '{"input_text": "your text with <|user_cursor_is_here|> marker"}'
```

This returns detailed information about:
- The context extracted from your input
- The exact request sent to the LLM
- The raw response from the LLM
- The processed output

### Logs

Check server logs for detailed information:
- `DEBUG` messages show exact request/response data
- `INFO` messages show high-level operation details
- `WARN` messages indicate potential issues

If a log entry contains `LLM returned '<|cursor|>' marker, removing it`, the LLM is not generating proper completions.
