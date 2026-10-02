use std::collections::VecDeque;
use std::sync::Mutex;
use async_openai::types::chat::{ChatCompletionRequestMessage, ChatCompletionTool};
use async_trait::async_trait;
use crate::llm::{ChatStream, ChatStreamEvent, LlmError};
use crate::llm::client::LlmClient;

pub(crate) struct MockLlm {
    scripts: Mutex<VecDeque<Vec<ChatStreamEvent>>>,
}

impl MockLlm {
    pub(crate) fn new(scripts: Vec<Vec<ChatStreamEvent>>) -> Self {
        Self {
            scripts: Mutex::new(scripts.into_iter().collect()),
        }
    }
}

#[async_trait]
impl LlmClient for MockLlm { 
    async fn chat(
        &self,
        _messages: Vec<ChatCompletionRequestMessage>,
        _tools: Vec<ChatCompletionTool>,
    ) -> Result<ChatStream, LlmError> {
        let script = self.scripts.lock().unwrap()
                        .pop_front()
                        .unwrap();
        Ok(Box::pin(futures::stream::iter(
            script.into_iter().map(Ok)
        )))
    }
}

pub(crate) fn content_events(s: &str) -> Vec<ChatStreamEvent> { 
        vec![
            ChatStreamEvent::Content(s.to_string()),
            ChatStreamEvent::Done
        ]
}

pub(crate) fn tool_call_script() -> Vec<ChatStreamEvent> {
    vec![
        ChatStreamEvent::ToolCallDelta {
            index: 0,
            id: Some("call_1".into()),
            name: Some("calculator".into()),
            args: r#"{"expression":"1+2"}"#.into(),
        },
        ChatStreamEvent::Done,
    ]
}
