use crate::types::{ApiConfig, Message, StreamEvent, ToolSchema};
use anyhow::Result;
use futures::StreamExt;
use reqwest::Client;
use serde_json::json;
use tokio::sync::mpsc;

pub struct ApiClient {
    client: Client,
    config: ApiConfig,
}

impl ApiClient {
    pub fn new(config: ApiConfig) -> Self {
        Self {
            client: Client::new(),
            config,
        }
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

        let mut stream = response.bytes_stream();
        let mut buffer = String::new();

        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result?;
            let chunk_str = String::from_utf8_lossy(&chunk);
            buffer.push_str(&chunk_str);

            while let Some(line_end) = buffer.find('\n') {
                let line = buffer[..line_end].trim().to_string();
                buffer = buffer[line_end + 1..].to_string();

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
}

#[derive(Debug, Clone)]
pub enum ApiEvent {
    Event(Box<StreamEvent>),
    Done,
}
