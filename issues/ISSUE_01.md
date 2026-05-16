# [Hardcoded Shortcuts] - [HIGH] - Hardcoded API endpoint prevents configuration flexibility

**Labels:** `bug`, `priority-high`, `hardcoded-shortcuts`

## Description

`ApiConfig::default()` hardcodes the API endpoint to `http://127.0.0.1:8000/v1/messages`. There is no mechanism to configure this via environment variable, CLI argument, or config file. The entire agent depends on `ds4-server` running at that exact address and port. Similarly, the model name is hardcoded to `"ds4"` and max_tokens to `8192`. A new instance of `ApiClient` is created per request in `main.rs:145` using `ApiConfig::default()`.

This creates a fragile house of cards: any user running a different inference server (e.g., Ollama, llama.cpp, vLLM on a different port) cannot use hackpi without modifying source code.

## Location

- `hackpi-core/src/types.rs:74-83` — `ApiConfig::default()`
- `hackpi-tui/src/main.rs:60` — `ApiClient::new(ApiConfig::default())`
- `hackpi-tui/src/main.rs:145` — `ApiClient::new(ApiConfig::default())`

## Impact

- Zero configurability without source modification
- Two separate `ApiClient` instances are created (lines 60 and 145), one of which (`_api` on line 60) is completely unused
- Forces all users into a single deployment topology

## Resolution

- Added `ApiConfig::from_env()` that reads `HACKPI_ENDPOINT`, `HACKPI_MODEL`, `HACKPI_MAX_TOKENS` env vars
- Removed unused `_api` variable from `main.rs:60`
- Both `ApiClient` creations now share the same `api_config` loaded once at startup
- Tests added for env var override and default fallback
- `types.rs` — added `impl ApiConfig { pub fn from_env() -> Self }`
- `main.rs` — uses `ApiConfig::from_env()` once, shares via `api_config.clone()`

**Status: RESOLVED**
