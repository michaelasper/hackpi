use crate::types::{ContentBlock, Message, Role};

use super::state::{Agent, PendingToolCall};

impl Agent {
    /// Build an assistant message from accumulated text content and tool calls.
    /// Text blocks come first, followed by `ToolUse` content blocks for each
    /// pending tool call. This ensures the conversation history always contains
    /// the assistant's `tool_use` blocks before any corresponding `tool_result`
    /// blocks.
    pub(crate) fn build_assistant_message(
        conversation: &mut Vec<Message>,
        text: &str,
        pending_tool_calls: &[PendingToolCall],
    ) {
        if text.is_empty() && pending_tool_calls.is_empty() {
            return;
        }

        let mut content: Vec<ContentBlock> =
            Vec::with_capacity(if text.is_empty() { 0 } else { 1 } + pending_tool_calls.len());

        if !text.is_empty() {
            content.push(ContentBlock::text(text));
        }

        for call in pending_tool_calls {
            content.push(ContentBlock::ToolUse {
                id: call.id.clone(),
                name: call.name.clone(),
                input: call.input.clone(),
            });
        }

        conversation.push(Message {
            role: Role::Assistant,
            content,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_assistant_message_adds_message_for_non_empty_text() {
        let mut conversation: Vec<Message> = Vec::new();
        Agent::build_assistant_message(&mut conversation, "Hello, world!", &[]);

        assert_eq!(conversation.len(), 1);
        assert!(
            matches!(conversation[0].role, Role::Assistant),
            "expected Assistant role"
        );
        match &conversation[0].content[0] {
            ContentBlock::Text { text } => {
                assert_eq!(text, "Hello, world!");
            }
            _ => panic!("expected Text block"),
        }
    }

    #[test]
    fn test_build_assistant_message_does_not_add_for_empty_text_and_no_tool_calls() {
        let mut conversation: Vec<Message> = vec![Message {
            role: Role::User,
            content: vec![ContentBlock::text("hi")],
        }];
        Agent::build_assistant_message(&mut conversation, "", &[]);

        assert_eq!(
            conversation.len(),
            1,
            "no new message for empty text with no tool calls"
        );
    }

    #[test]
    fn test_build_assistant_message_adds_for_whitespace_text() {
        let mut conversation: Vec<Message> = Vec::new();
        Agent::build_assistant_message(&mut conversation, "   ", &[]);

        assert_eq!(
            conversation.len(),
            1,
            "whitespace-only text is still non-empty and should be added"
        );
    }

    #[test]
    fn test_build_assistant_message_multiple_calls() {
        let mut conversation: Vec<Message> = Vec::new();
        Agent::build_assistant_message(&mut conversation, "First", &[]);
        Agent::build_assistant_message(&mut conversation, "Second", &[]);

        assert_eq!(conversation.len(), 2);
        match &conversation[0].content[0] {
            ContentBlock::Text { text } => assert_eq!(text, "First"),
            other => panic!("expected Text block, got {other:?}"),
        }
        match &conversation[1].content[0] {
            ContentBlock::Text { text } => assert_eq!(text, "Second"),
            other => panic!("expected Text block, got {other:?}"),
        }
    }

    #[test]
    fn test_build_assistant_message_includes_tool_use_blocks() {
        let mut conversation: Vec<Message> = Vec::new();
        let tool_calls = vec![
            PendingToolCall {
                id: "toolu_abc".into(),
                name: "read_file".into(),
                input: serde_json::json!({"path": "src/main.rs"}),
                parse_error: None,
            },
            PendingToolCall {
                id: "toolu_def".into(),
                name: "grep".into(),
                input: serde_json::json!({"pattern": "TODO"}),
                parse_error: None,
            },
        ];
        Agent::build_assistant_message(&mut conversation, "Let me look.", &tool_calls);

        assert_eq!(conversation.len(), 1);
        assert!(
            matches!(conversation[0].role, Role::Assistant),
            "expected Assistant role"
        );
        assert_eq!(conversation[0].content.len(), 3, "text + 2 tool_use blocks");

        // First block should be text
        match &conversation[0].content[0] {
            ContentBlock::Text { text } => assert_eq!(text, "Let me look."),
            _ => panic!("expected Text block first"),
        }

        // Second block should be first tool_use
        match &conversation[0].content[1] {
            ContentBlock::ToolUse { id, name, input } => {
                assert_eq!(id, "toolu_abc");
                assert_eq!(name, "read_file");
                assert_eq!(input, &serde_json::json!({"path": "src/main.rs"}));
            }
            _ => panic!("expected ToolUse block"),
        }

        // Third block should be second tool_use
        match &conversation[0].content[2] {
            ContentBlock::ToolUse { id, name, input } => {
                assert_eq!(id, "toolu_def");
                assert_eq!(name, "grep");
                assert_eq!(input, &serde_json::json!({"pattern": "TODO"}));
            }
            _ => panic!("expected ToolUse block"),
        }
    }

    #[test]
    fn test_build_assistant_message_adds_message_for_only_tool_calls_with_empty_text() {
        let mut conversation: Vec<Message> = Vec::new();
        let tool_calls = vec![PendingToolCall {
            id: "toolu_ghi".into(),
            name: "search".into(),
            input: serde_json::json!({"query": "foo"}),
            parse_error: None,
        }];
        Agent::build_assistant_message(&mut conversation, "", &tool_calls);

        assert_eq!(
            conversation.len(),
            1,
            "should add message for tool calls even without text"
        );
        assert_eq!(
            conversation[0].content.len(),
            1,
            "only tool_use block, no text"
        );
        match &conversation[0].content[0] {
            ContentBlock::ToolUse { id, name, .. } => {
                assert_eq!(id, "toolu_ghi");
                assert_eq!(name, "search");
            }
            _ => panic!("expected ToolUse block"),
        }
    }

    #[test]
    fn test_build_assistant_message_does_not_add_for_empty_tool_calls_with_empty_text() {
        let mut conversation: Vec<Message> = vec![Message {
            role: Role::User,
            content: vec![ContentBlock::text("hi")],
        }];
        Agent::build_assistant_message(&mut conversation, "", &[]);

        assert_eq!(
            conversation.len(),
            1,
            "no new message when both text and tool_calls are empty"
        );
    }
}
