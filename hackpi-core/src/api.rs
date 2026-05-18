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

        let stream = response.bytes_stream().map(|r| r.map(|b| b.to_vec()));
        process_sse_stream(stream, tx).await
    }
}

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

            if let Some(data) = line.strip_prefix("data: ") {
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
                    }
                }
            }
        }
    }

    Ok(())
}

#[derive(Debug, Clone)]
pub enum ApiEvent {
    Event(Box<StreamEvent>),
    Done,
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream;

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
}
