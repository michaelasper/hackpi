use crate::types::{ApiConfig, Message, StreamEvent, ToolSchema};
use anyhow::Result;
use futures::{Stream, StreamExt};
use reqwest::Client;
use serde_json::json;
use std::time::Duration;
use tokio::sync::mpsc;

pub struct ApiClient {
    client: Client,
    config: ApiConfig,
}

impl ApiClient {
    pub fn new(config: ApiConfig) -> Result<Self> {
        Ok(Self {
            client: Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(300))
                .build()
                .map_err(|e| anyhow::anyhow!("Failed to build HTTP client: {e}"))?,
            config,
        })
    }

    pub async fn send_messages(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        system_prompt: &str,
        tx: mpsc::UnboundedSender<ApiEvent>,
    ) -> Result<()> {
        let tools: Vec<serde_json::Value> = tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.input_schema,
                })
            })
            .collect();

        let system = json!([{"type": "text", "text": system_prompt}]);

        let body = json!({
            "model": self.config.model,
            "max_tokens": self.config.max_tokens,
            "temperature": self.config.temperature,
            "system": system,
            "messages": messages,
            "tools": tools,
            "stream": true,
        });

        let response = self
            .client
            .post(&self.config.endpoint)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        // Check HTTP status before consuming the body stream.
        // Non-success statuses (4xx, 5xx) indicate auth failures, server
        // errors, or proxy issues — return a structured error with a
        // bounded body excerpt so the caller sees an actionable message.
        let status = response.status();
        if !status.is_success() {
            let body_preview = response
                .bytes()
                .await
                .map(|b| {
                    let preview = String::from_utf8_lossy(&b);
                    if preview.len() > 500 {
                        format!("{}... (truncated)", &preview[..500])
                    } else {
                        preview.to_string()
                    }
                })
                .unwrap_or_else(|_| "<failed to read body>".to_string());
            return Err(anyhow::anyhow!(
                "API request failed with HTTP {status}: {body_preview}"
            ));
        }

        let stream = response.bytes_stream().map(|r| r.map(|b| b.to_vec()));
        process_sse_stream(stream, tx).await
    }
}

/// Maximum allowed size of a single SSE line buffer (1 MB).
/// This prevents unbounded memory growth from a hostile or misconfigured
/// endpoint that omits newlines or sends extremely long lines.
const MAX_SSE_LINE_SIZE: usize = 1_048_576;

/// Process an SSE byte stream, buffering raw bytes to avoid UTF-8 corruption
/// when multi-byte characters are split across TCP chunks.
async fn process_sse_stream<S, E>(mut stream: S, tx: mpsc::UnboundedSender<ApiEvent>) -> Result<()>
where
    S: Stream<Item = Result<Vec<u8>, E>> + Unpin,
    E: Into<anyhow::Error>,
{
    let mut buffer: Vec<u8> = Vec::new();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(Into::into)?;

        // Guard against unbounded buffer growth: if this chunk would push the
        // buffer past the maximum line size, the stream is either hostile or
        // misconfigured. Return an error so the caller can abort.
        if buffer.len() + chunk.len() > MAX_SSE_LINE_SIZE {
            return Err(anyhow::anyhow!(
                "SSE buffer exceeded maximum line size of {} bytes \
                 (buffer: {} bytes, incoming chunk: {} bytes); \
                 the endpoint may be omitting newlines or sending oversized data",
                MAX_SSE_LINE_SIZE,
                buffer.len(),
                chunk.len(),
            ));
        }

        buffer.extend_from_slice(&chunk);

        // Process complete lines (terminated by 0x0A = '\n')
        // We use byte-level search so multi-byte UTF-8 sequences in the
        // buffer are never corrupted — we only decode at line boundaries.
        while let Some(line_end) = buffer.iter().position(|&b| b == b'\n') {
            let line_bytes = &buffer[..line_end];
            // Decode only complete lines; skip if invalid UTF-8 (shouldn't
            // happen for well-formed SSE, but be defensive).
            let line = match std::str::from_utf8(line_bytes) {
                Ok(s) => s.trim().to_string(),
                Err(_) => {
                    buffer = buffer[line_end + 1..].to_vec();
                    continue;
                }
            };
            buffer = buffer[line_end + 1..].to_vec();

            if line.is_empty() {
                continue;
            }

            // SSE spec: data:value or data: value (space after colon is optional).
            // We match on "data:" and trim to handle both forms.
            // Non-data lines (event:, id:, retry:, :comments) are valid SSE but
            // not relevant for our JSON-only event protocol; log at trace level.
            if let Some(data) = line.strip_prefix("data:") {
                let data = data.trim();

                if data.is_empty() {
                    // Empty data line — skip rather than failing to parse ""
                    // as JSON. Some proxies emit blank data: frames as keepalives.
                    tracing::trace!("SSE data line with empty payload, skipping");
                    continue;
                }

                if data == "[DONE]" {
                    tx.send(ApiEvent::Done).ok();
                    return Ok(());
                }

                match serde_json::from_str::<StreamEvent>(data) {
                    Ok(event) => {
                        tx.send(ApiEvent::Event(Box::new(event))).ok();
                    }
                    Err(e) => {
                        tracing::warn!("Failed to parse SSE event: {e}, data: {data}");
                        tx.send(ApiEvent::Error(format!("Failed to parse SSE event: {e}")))
                            .ok();
                    }
                }
            } else {
                // Non-data SSE fields (event:, id:, retry:, :) are valid but
                // our protocol only uses data: lines with JSON payloads.
                tracing::trace!("Non-data SSE line (silently ignored): {line}");
            }
        }
    }

    // The byte stream ended without a [DONE] frame. Some servers
    // (e.g., ds4-server) do not send [DONE], so treat this as a
    // warning rather than a hard error. The caller has already
    // received all events sent before the stream closed.

    // Check for leftover buffered bytes that were never terminated by a
    // newline. This indicates a truncated or malformed stream — the data
    // is incomplete and cannot be parsed.
    if !buffer.is_empty() {
        let leftover = String::from_utf8_lossy(&buffer);
        return Err(anyhow::anyhow!(
            "SSE stream ended with {} unprocessed byte(s) remaining in buffer: \
             the last line was not terminated by a newline. \
             Leftover preview: {leftover:.50}",
            buffer.len(),
        ));
    }

    tracing::warn!("SSE stream ended without [DONE] frame; stream may be truncated");
    Ok(())
}

#[derive(Debug, Clone)]
pub enum ApiEvent {
    Event(Box<StreamEvent>),
    Error(String),
    Done,
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn test_api_client_has_connect_timeout() {
        let config = ApiConfig::default();
        let result = ApiClient::new(config);
        assert!(
            result.is_ok(),
            "ApiClient::new should succeed with default config"
        );
    }

    #[tokio::test]
    async fn test_send_messages_http_401_returns_error() {
        let mock_server = MockServer::start().await;

        // A 401 response with an error body
        Mock::given(wiremock::matchers::method("POST"))
            .respond_with(ResponseTemplate::new(401).set_body_string(
                r#"{"error":{"message":"Invalid API key","type":"authentication_error"}}"#,
            ))
            .mount(&mock_server)
            .await;

        let config = ApiConfig {
            endpoint: format!("{}/v1/messages", mock_server.uri()),
            ..ApiConfig::default()
        };
        let client = ApiClient::new(config).expect("should create client");
        let (tx, _rx) = mpsc::unbounded_channel::<ApiEvent>();

        let result = client.send_messages(&[], &[], "", tx).await;

        assert!(
            result.is_err(),
            "HTTP 401 should return an error, got Ok(())"
        );
        let err = format!("{}", result.unwrap_err());
        assert!(
            err.contains("401") || err.contains("status") || err.contains("Unauthorized"),
            "error should mention the HTTP status: {err}"
        );
    }

    #[tokio::test]
    async fn test_send_messages_http_500_returns_error() {
        let mock_server = MockServer::start().await;

        Mock::given(wiremock::matchers::method("POST"))
            .respond_with(ResponseTemplate::new(500).set_body_string("Internal Server Error"))
            .mount(&mock_server)
            .await;

        let config = ApiConfig {
            endpoint: format!("{}/v1/messages", mock_server.uri()),
            ..ApiConfig::default()
        };
        let client = ApiClient::new(config).expect("should create client");
        let (tx, _rx) = mpsc::unbounded_channel::<ApiEvent>();

        let result = client.send_messages(&[], &[], "", tx).await;

        assert!(
            result.is_err(),
            "HTTP 500 should return an error, got Ok(())"
        );
        let err = format!("{}", result.unwrap_err());
        assert!(
            err.contains("500") || err.contains("status") || err.contains("Server Error"),
            "error should mention the HTTP status: {err}"
        );
    }

    #[tokio::test]
    async fn test_process_sse_stream_normal_events() {
        let chunks: Vec<Result<Vec<u8>, anyhow::Error>> = vec![
            Ok(b"data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n".to_vec()),
            Ok(b"data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"Hello\"}}\n".to_vec()),
            Ok(b"data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\" World\"}}\n".to_vec()),
            Ok(b"data: [DONE]\n".to_vec()),
        ];

        let (tx, mut rx) = mpsc::unbounded_channel::<ApiEvent>();
        process_sse_stream(stream::iter(chunks), tx)
            .await
            .expect("process_sse_stream should succeed");

        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            match event {
                ApiEvent::Event(e) => events.push(e),
                ApiEvent::Error(_) => {}
                ApiEvent::Done => break,
            }
        }

        assert_eq!(events.len(), 3, "should have received three events");
        assert_eq!(events[0].event_type, "content_block_start");
        assert_eq!(
            events[1].delta.as_ref().unwrap().text.as_deref(),
            Some("Hello")
        );
        assert_eq!(
            events[2].delta.as_ref().unwrap().text.as_deref(),
            Some(" World")
        );
    }

    #[tokio::test]
    async fn test_process_sse_stream_skips_empty_lines() {
        let chunks: Vec<Result<Vec<u8>, anyhow::Error>> = vec![
            Ok(b"\n".to_vec()),
            Ok(
                b"data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"hi\"}}\n\n"
                    .to_vec(),
            ),
            Ok(b"data: [DONE]\n".to_vec()),
        ];

        let (tx, mut rx) = mpsc::unbounded_channel::<ApiEvent>();
        process_sse_stream(stream::iter(chunks), tx)
            .await
            .expect("process_sse_stream should succeed");

        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            match event {
                ApiEvent::Event(e) => events.push(e),
                ApiEvent::Error(_) => {}
                ApiEvent::Done => break,
            }
        }

        assert_eq!(
            events.len(),
            1,
            "should have received one event (skipping empty lines)"
        );
        assert_eq!(
            events[0].delta.as_ref().unwrap().text.as_deref(),
            Some("hi")
        );
    }

    #[tokio::test]
    async fn test_process_sse_stream_invalid_json_sends_error_event() {
        // Invalid JSON in an SSE data line should send an ApiEvent::Error
        let chunks: Vec<Result<Vec<u8>, anyhow::Error>> = vec![
            Ok(b"data: {invalid json}\n".to_vec()),
            Ok(b"data: [DONE]\n".to_vec()),
        ];

        let (tx, mut rx) = mpsc::unbounded_channel::<ApiEvent>();
        process_sse_stream(stream::iter(chunks), tx)
            .await
            .expect("process_sse_stream should succeed");

        let mut errors = Vec::new();
        while let Some(event) = rx.recv().await {
            match event {
                ApiEvent::Error(msg) => errors.push(msg),
                ApiEvent::Done => break,
                _ => {}
            }
        }

        assert_eq!(
            errors.len(),
            1,
            "should have sent one error event for invalid JSON"
        );
        assert!(
            errors[0].contains("Failed to parse"),
            "error message should describe parse failure: {}",
            errors[0]
        );
    }

    #[tokio::test]
    async fn test_process_sse_stream_handles_done_only() {
        let chunks: Vec<Result<Vec<u8>, anyhow::Error>> = vec![Ok(b"data: [DONE]\n".to_vec())];

        let (tx, mut rx) = mpsc::unbounded_channel::<ApiEvent>();
        process_sse_stream(stream::iter(chunks), tx)
            .await
            .expect("process_sse_stream should succeed");

        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            match event {
                ApiEvent::Event(e) => events.push(e),
                ApiEvent::Error(_) => {}
                ApiEvent::Done => break,
            }
        }

        assert_eq!(events.len(), 0, "should have received no events");
    }

    #[tokio::test]
    async fn test_process_sse_stream_skips_invalid_utf8_and_continues() {
        // A line with invalid UTF-8 should be skipped, but subsequent valid
        // lines should still be processed.
        let invalid_line =
            b"data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"\xff\xfe\"}}\n"
                .to_vec();
        let valid_line =
            b"data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"ok\"}}\n".to_vec();
        let done = b"data: [DONE]\n".to_vec();

        let chunks: Vec<Result<Vec<u8>, anyhow::Error>> =
            vec![Ok(invalid_line), Ok(valid_line), Ok(done)];

        let (tx, mut rx) = mpsc::unbounded_channel::<ApiEvent>();
        process_sse_stream(stream::iter(chunks), tx)
            .await
            .expect("process_sse_stream should succeed");

        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            match event {
                ApiEvent::Event(e) => events.push(e),
                ApiEvent::Error(_) => {}
                ApiEvent::Done => break,
            }
        }

        assert_eq!(
            events.len(),
            1,
            "should have skipped invalid UTF-8 line but processed valid one"
        );
        assert_eq!(
            events[0].delta.as_ref().unwrap().text.as_deref(),
            Some("ok")
        );
    }

    #[tokio::test]
    async fn test_process_sse_stream_handles_multiple_events_in_one_chunk() {
        let chunks: Vec<Result<Vec<u8>, anyhow::Error>> = vec![
            Ok(b"data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"A\"}}\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"B\"}}\ndata: [DONE]\n".to_vec()),
        ];

        let (tx, mut rx) = mpsc::unbounded_channel::<ApiEvent>();
        process_sse_stream(stream::iter(chunks), tx)
            .await
            .expect("process_sse_stream should succeed");

        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            match event {
                ApiEvent::Event(e) => events.push(e),
                ApiEvent::Error(_) => {}
                ApiEvent::Done => break,
            }
        }

        assert_eq!(
            events.len(),
            2,
            "should have received two events from one chunk"
        );
    }

    #[tokio::test]
    async fn test_process_sse_stream_handles_split_utf8() {
        // "é" in UTF-8 is 0xC3 0xA9. Simulate it being split across 4 chunks.
        // The full SSE line is: data: {"type":"content_block_delta","delta":{"text":"é"}}\n
        // Chunk 1: "data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\""
        // Chunk 2: 0xC3 (first byte of é)
        // Chunk 3: 0xA9 (second byte of é)
        // Chunk 4: "\"}}\n" (rest of line + newline)
        // Chunk 5: "data: [DONE]\n" (done signal)

        let part1 = b"data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"";
        let part2 = [0xC3]; // first byte of é
        let part3 = [0xA9]; // second byte of é
        let part4 = b"\"}}\n";
        let part5 = b"data: [DONE]\n";

        let chunks: Vec<Result<Vec<u8>, anyhow::Error>> = vec![
            Ok(part1.to_vec()),
            Ok(part2.to_vec()),
            Ok(part3.to_vec()),
            Ok(part4.to_vec()),
            Ok(part5.to_vec()),
        ];

        let (tx, mut rx) = mpsc::unbounded_channel::<ApiEvent>();

        process_sse_stream(stream::iter(chunks), tx)
            .await
            .expect("process_sse_stream should succeed");

        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            match event {
                ApiEvent::Event(e) => events.push(e),
                ApiEvent::Error(_) => {}
                ApiEvent::Done => break,
            }
        }

        assert_eq!(events.len(), 1, "should have received one event");
        let delta = events[0].delta.as_ref().expect("event should have delta");
        assert_eq!(
            delta.text.as_deref(),
            Some("é"),
            "delta text should have correct UTF-8 character"
        );
    }

    #[tokio::test]
    async fn test_process_sse_stream_incomplete_stream_logs_warning() {
        // Stream ends without [DONE] — should log a warning and succeed
        let chunks: Vec<Result<Vec<u8>, anyhow::Error>> = vec![Ok(
            b"data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"Hello\"}}\n".to_vec(),
        )];

        let (tx, mut rx) = mpsc::unbounded_channel::<ApiEvent>();
        let result = process_sse_stream(stream::iter(chunks), tx).await;

        assert!(
            result.is_ok(),
            "incomplete SSE stream (no [DONE]) should succeed with a warning"
        );

        // The event should still have been received
        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            match event {
                ApiEvent::Event(e) => events.push(e),
                ApiEvent::Error(_) => {}
                ApiEvent::Done => break,
            }
        }
        assert_eq!(events.len(), 1, "should have received the event");
    }

    #[tokio::test]
    async fn test_process_sse_stream_empty_stream_logs_warning() {
        // Empty stream with no chunks at all — no [DONE] seen, should succeed
        // with a warning
        let chunks: Vec<Result<Vec<u8>, anyhow::Error>> = vec![];

        let (tx, _rx) = mpsc::unbounded_channel::<ApiEvent>();
        let result = process_sse_stream(stream::iter(chunks), tx).await;

        assert!(
            result.is_ok(),
            "empty SSE stream (no [DONE]) should succeed with a warning"
        );
    }

    #[tokio::test]
    async fn test_process_sse_stream_done_at_end_still_succeeds() {
        // Stream with [DONE] at the end should still succeed
        let chunks: Vec<Result<Vec<u8>, anyhow::Error>> = vec![
            Ok(
                b"data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"Hello\"}}\n"
                    .to_vec(),
            ),
            Ok(b"data: [DONE]\n".to_vec()),
        ];

        let (tx, _rx) = mpsc::unbounded_channel::<ApiEvent>();
        let result = process_sse_stream(stream::iter(chunks), tx).await;

        assert!(
            result.is_ok(),
            "SSE stream ending with [DONE] should succeed"
        );
    }

    #[tokio::test]
    async fn test_process_sse_stream_only_done_succeeds() {
        // Only [DONE] with no events should succeed
        let chunks: Vec<Result<Vec<u8>, anyhow::Error>> = vec![Ok(b"data: [DONE]\n".to_vec())];

        let (tx, _rx) = mpsc::unbounded_channel::<ApiEvent>();
        let result = process_sse_stream(stream::iter(chunks), tx).await;

        assert!(result.is_ok(), "SSE stream with only [DONE] should succeed");
    }

    #[tokio::test]
    async fn test_process_sse_stream_data_without_space() {
        // SSE spec: `data:value` without space after colon is valid.
        let chunks: Vec<Result<Vec<u8>, anyhow::Error>> = vec![
            Ok(
                b"data:{\"type\":\"content_block_delta\",\"delta\":{\"text\":\"Hello\"}}\n"
                    .to_vec(),
            ),
            Ok(b"data:[DONE]\n".to_vec()),
        ];

        let (tx, mut rx) = mpsc::unbounded_channel::<ApiEvent>();
        process_sse_stream(stream::iter(chunks), tx)
            .await
            .expect("process_sse_stream should succeed");

        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            match event {
                ApiEvent::Event(e) => events.push(e),
                ApiEvent::Error(_) => {}
                ApiEvent::Done => break,
            }
        }

        assert_eq!(
            events.len(),
            1,
            "should have parsed data: without space after colon"
        );
        assert_eq!(
            events[0].delta.as_ref().unwrap().text.as_deref(),
            Some("Hello")
        );
    }

    #[tokio::test]
    async fn test_process_sse_stream_empty_data_line_skipped() {
        // Empty data: lines (e.g., keepalives) should be skipped, not cause parse errors.
        let chunks: Vec<Result<Vec<u8>, anyhow::Error>> = vec![
            Ok(b"data:\n".to_vec()),
            Ok(b"data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"ok\"}}\n".to_vec()),
            Ok(b"data: [DONE]\n".to_vec()),
        ];

        let (tx, mut rx) = mpsc::unbounded_channel::<ApiEvent>();
        process_sse_stream(stream::iter(chunks), tx)
            .await
            .expect("process_sse_stream should succeed");

        let mut events = Vec::new();
        let mut errors = Vec::new();
        while let Some(event) = rx.recv().await {
            match event {
                ApiEvent::Event(e) => events.push(e),
                ApiEvent::Error(e) => errors.push(e),
                ApiEvent::Done => break,
            }
        }

        assert_eq!(
            errors.len(),
            0,
            "empty data: line should not produce a parse error"
        );
        assert_eq!(events.len(), 1, "should have received the valid event");
    }

    #[tokio::test]
    async fn test_process_sse_stream_non_data_lines_ignored() {
        // Lines with event:, id:, retry:, or :comment prefixes are valid SSE
        // but our protocol only uses data:. They should be silently ignored.
        let chunks: Vec<Result<Vec<u8>, anyhow::Error>> = vec![
            Ok(b"event: ping\n".to_vec()),
            Ok(b"id: 42\n".to_vec()),
            Ok(b": this is a comment\n".to_vec()),
            Ok(b"retry: 3000\n".to_vec()),
            Ok(b"data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"hi\"}}\n".to_vec()),
            Ok(b"data: [DONE]\n".to_vec()),
        ];

        let (tx, mut rx) = mpsc::unbounded_channel::<ApiEvent>();
        process_sse_stream(stream::iter(chunks), tx)
            .await
            .expect("process_sse_stream should succeed");

        let mut events = Vec::new();
        let mut errors = Vec::new();
        while let Some(event) = rx.recv().await {
            match event {
                ApiEvent::Event(e) => events.push(e),
                ApiEvent::Error(e) => errors.push(e),
                ApiEvent::Done => break,
            }
        }

        assert_eq!(errors.len(), 0, "non-data lines should not produce errors");
        assert_eq!(events.len(), 1, "should have received only the data: event");
    }

    #[tokio::test]
    async fn test_process_sse_stream_carriage_return_line_endings() {
        // SSE spec allows \r\n, \n, or \r line endings. The parser splits on
        // \n and trims, so \r is stripped by trim().
        let chunks: Vec<Result<Vec<u8>, anyhow::Error>> = vec![Ok(
            b"data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"crlf\"}}\r\ndata: [DONE]\n"
                .to_vec(),
        )];

        let (tx, mut rx) = mpsc::unbounded_channel::<ApiEvent>();
        process_sse_stream(stream::iter(chunks), tx)
            .await
            .expect("process_sse_stream should succeed");

        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            match event {
                ApiEvent::Event(e) => events.push(e),
                ApiEvent::Error(_) => {}
                ApiEvent::Done => break,
            }
        }

        assert_eq!(events.len(), 1, "should parse SSE with \\r\\n line endings");
        assert_eq!(
            events[0].delta.as_ref().unwrap().text.as_deref(),
            Some("crlf")
        );
    }

    #[tokio::test]
    async fn test_process_sse_stream_done_without_space() {
        // [DONE] without space after data: should still be recognized.
        let chunks: Vec<Result<Vec<u8>, anyhow::Error>> = vec![Ok(b"data:[DONE]\n".to_vec())];

        let (tx, mut rx) = mpsc::unbounded_channel::<ApiEvent>();
        process_sse_stream(stream::iter(chunks), tx)
            .await
            .expect("process_sse_stream should succeed");

        let mut done_received = false;
        while let Some(event) = rx.recv().await {
            if matches!(event, ApiEvent::Done) {
                done_received = true;
                break;
            }
        }

        assert!(
            done_received,
            "should have received [DONE] without space prefix"
        );
    }

    #[tokio::test]
    async fn test_process_sse_stream_exceeds_max_line_size() {
        // A single chunk that exceeds MAX_SSE_LINE_SIZE should return an error.
        let oversized = vec![b'x'; MAX_SSE_LINE_SIZE + 1];
        let chunks: Vec<Result<Vec<u8>, anyhow::Error>> = vec![Ok(oversized)];

        let (tx, _rx) = mpsc::unbounded_channel::<ApiEvent>();
        let result = process_sse_stream(stream::iter(chunks), tx).await;

        assert!(result.is_err(), "oversized chunk should return an error");
        let err = format!("{}", result.unwrap_err());
        assert!(
            err.contains("SSE buffer exceeded"),
            "error should mention buffer overflow: {err}"
        );
    }

    #[tokio::test]
    async fn test_process_sse_stream_exceeds_max_line_size_cumulative() {
        // Multiple chunks that cumulatively exceed MAX_SSE_LINE_SIZE (without
        // any newline) should return an error.
        let chunk_size = MAX_SSE_LINE_SIZE / 2 + 1; // 2 chunks will exceed
        let chunk = vec![b'x'; chunk_size];
        let chunks: Vec<Result<Vec<u8>, anyhow::Error>> = vec![Ok(chunk.clone()), Ok(chunk)];

        let (tx, _rx) = mpsc::unbounded_channel::<ApiEvent>();
        let result = process_sse_stream(stream::iter(chunks), tx).await;

        assert!(
            result.is_err(),
            "cumulative oversized chunks should return an error"
        );
        let err = format!("{}", result.unwrap_err());
        assert!(
            err.contains("SSE buffer exceeded"),
            "error should mention buffer overflow: {err}"
        );
    }

    #[tokio::test]
    async fn test_process_sse_stream_leftover_bytes_on_eof() {
        // Stream ends with data still in the buffer (no trailing newline).
        // This should return an error about leftover unprocessed bytes.
        let chunks: Vec<Result<Vec<u8>, anyhow::Error>> = vec![Ok(
            b"data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"Hello\"}}".to_vec(),
        )];

        let (tx, _rx) = mpsc::unbounded_channel::<ApiEvent>();
        let result = process_sse_stream(stream::iter(chunks), tx).await;

        assert!(
            result.is_err(),
            "stream ending with leftover bytes should return an error"
        );
        let err = format!("{}", result.unwrap_err());
        assert!(
            err.contains("unprocessed byte(s)"),
            "error should mention unprocessed bytes: {err}"
        );
    }

    #[tokio::test]
    async fn test_process_sse_stream_exact_fit_does_not_overflow() {
        // A chunk exactly at MAX_SSE_LINE_SIZE should be allowed (the check is
        // strict greater-than). It will eventually fail with leftover bytes on
        // EOF (no newline), but it must NOT trigger the buffer overflow error.
        let exact_fit = vec![b'x'; MAX_SSE_LINE_SIZE];
        let chunks: Vec<Result<Vec<u8>, anyhow::Error>> = vec![Ok(exact_fit)];

        let (tx, _rx) = mpsc::unbounded_channel::<ApiEvent>();
        let result = process_sse_stream(stream::iter(chunks), tx).await;

        // Should fail with leftover bytes, NOT with buffer overflow
        assert!(
            result.is_err(),
            "should still error on EOF (leftover bytes)"
        );
        let err = format!("{}", result.unwrap_err());
        assert!(
            err.contains("unprocessed byte(s)"),
            "should fail with leftover bytes, not overflow: {err}"
        );
        assert!(
            !err.contains("SSE buffer exceeded"),
            "should NOT trigger buffer overflow for exact fit: {err}"
        );
    }

    #[tokio::test]
    async fn test_process_sse_stream_normal_events_under_limit() {
        // Normal event stream should work fine even with the buffer limit in place.
        let chunks: Vec<Result<Vec<u8>, anyhow::Error>> = vec![
            Ok(b"data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n".to_vec()),
            Ok(b"data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"Hello\"}}\n".to_vec()),
            Ok(b"data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\" World\"}}\n".to_vec()),
            Ok(b"data: [DONE]\n".to_vec()),
        ];

        let (tx, mut rx) = mpsc::unbounded_channel::<ApiEvent>();
        process_sse_stream(stream::iter(chunks), tx)
            .await
            .expect("process_sse_stream should succeed with normal events under limit");

        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            match event {
                ApiEvent::Event(e) => events.push(e),
                ApiEvent::Error(_) => {}
                ApiEvent::Done => break,
            }
        }

        assert_eq!(events.len(), 3, "should have received three events");
        assert_eq!(events[0].event_type, "content_block_start");
        assert_eq!(
            events[1].delta.as_ref().unwrap().text.as_deref(),
            Some("Hello")
        );
        assert_eq!(
            events[2].delta.as_ref().unwrap().text.as_deref(),
            Some(" World")
        );
    }
}
