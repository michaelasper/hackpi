use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};

use hackpi_core::agent::{Agent, AgentEvent};
use hackpi_core::api::ApiClient;
use hackpi_core::tools::ToolRegistry;
use hackpi_core::types::{ApiConfig, ContentBlock, Message, Role};

fn is_ds4_running() -> bool {
    std::net::TcpStream::connect_timeout(&"127.0.0.1:8000".parse().unwrap(), Duration::from_secs(2))
        .is_ok()
}

#[tokio::test]
async fn test_agent_basic_conversation() {
    if !is_ds4_running() {
        eprintln!("SKIP: ds4-server not running on 127.0.0.1:8000");
        return;
    }

    let config = ApiConfig {
        endpoint: "http://127.0.0.1:8000/v1/messages".into(),
        model: "ds4".into(),
        max_tokens: 256,
        temperature: 0.0,
    };

    let api = ApiClient::new(config).unwrap();
    let tools = Arc::new(ToolRegistry::new());
    let system_prompt = "You are a helpful assistant. Respond concisely.";
    let workspace_root = PathBuf::from("/tmp");

    let agent = Agent::new(api, tools, system_prompt.into(), workspace_root);

    let mut conversation: Vec<Message> = vec![Message {
        role: Role::User,
        content: vec![ContentBlock::text("Say hello in one word.")],
    }];

    let (tx, mut rx) = mpsc::unbounded_channel();
    let (_cancel_tx, signal) = watch::channel(false);
    let cancelled = Arc::new(AtomicBool::new(false));

    tokio::time::timeout(Duration::from_secs(30), async {
        agent
            .run("Say hello in one word.", &mut conversation, tx, signal, cancelled)
            .await;
    })
    .await
    .expect("Agent timed out");

    let mut events: Vec<AgentEvent> = Vec::new();
    while let Ok(event) = rx.try_recv() {
        events.push(event);
    }

    let has_text = events.iter().any(|e| matches!(e, AgentEvent::TextChunk(_)));
    let has_done = events.iter().any(|e| matches!(e, AgentEvent::Done));

    assert!(has_text, "Agent should produce text output");
    assert!(has_done, "Agent should signal completion");

    let assistant_messages: Vec<&Message> = conversation
        .iter()
        .filter(|m| matches!(m.role, Role::Assistant))
        .collect();
    assert!(
        !assistant_messages.is_empty(),
        "Agent should add assistant messages"
    );
}
