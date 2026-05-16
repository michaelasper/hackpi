# [Security] - [MEDIUM] - No input timeout on reqwest HTTP client

**Labels:** `security`, `priority-medium`, `api-design`

## Description

In `api.rs:14-18`, the `ApiClient` creates a `reqwest::Client` using `Client::new()`, which uses default settings. The default `reqwest::Client` has **no connect timeout and no read timeout**. If the ds4-server hangs or takes too long to respond, the agent loop will block indefinitely on `response.bytes_stream()` without any timeout.

Additionally, there is no per-request timeout or cancellation mechanism except for the signal channel (which requires the agent loop to check it between events — but if the SSE stream is stuck between events, the loop waits forever).

## Location

- `hackpi-core/src/api.rs:15-16` — `Client::new()` with no timeout configuration
- `hackpi-core/src/api.rs:51-57` — `self.client.post(...)...send().await?` — no timeout
- `hackpi-core/src/api.rs:59` — `response.bytes_stream()` — no timeout on streaming

## Impact

- Agent can hang indefinitely if the API server is unresponsive
- Ctrl+C may not work if the HTTP stream is blocked (the signal is only checked between events)
- A misconfigured server can permanently block the agent

## Resolution

- Changed `Client::new()` to `Client::builder().connect_timeout(10s).timeout(300s).build()`
- `ApiClient::new()` now returns `Result<Self>` to propagate build errors
- Updated all callers (`main.rs`, `integration_test.rs`) to use `?` or `.unwrap()`
- Added `test_api_client_has_connect_timeout` test

**Status: RESOLVED**
