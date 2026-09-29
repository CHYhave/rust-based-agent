use async_openai::types::chat::{ChatCompletionRequestMessage,
    ChatCompletionRequestUserMessage,
    ChatCompletionRequestUserMessageContent,
};
use async_trait::async_trait;

#[async_trait]
pub trait Memory: Send + Sync {
    async fn messages(&self) -> Vec<ChatCompletionRequestMessage>;
    async fn add(&self, msg: ChatCompletionRequestMessage);
    async fn clear(&self);
    async fn is_empty(&self) -> bool {
        self.messages().await.is_empty()
    }
}

pub struct InMemoryMemory {
    messages: tokio::sync::Mutex<Vec<ChatCompletionRequestMessage>>,
}

impl InMemoryMemory { 
    pub fn new() -> Self {
        Self { messages: tokio::sync::Mutex::new(Vec::new()) }
    }
}

#[async_trait]
impl Memory for InMemoryMemory {
    
    async fn messages(&self) -> Vec<ChatCompletionRequestMessage> {
        self.messages.lock().await.clone()
    }

    async fn add(&self, msg: ChatCompletionRequestMessage) {
        self.messages.lock().await.push(msg);
    }

    async fn clear(&self) {
        self.messages.lock().await.clear();
    }
}

impl Default for InMemoryMemory {
    fn default() -> Self {
        Self::new()
    }
}


#[cfg(test)]
mod test {
    use super::*;

    fn user_msg(s: &str) -> ChatCompletionRequestMessage {
        ChatCompletionRequestUserMessage::from(
            ChatCompletionRequestUserMessageContent::Text(s.to_string()),
        ).into()
    }

    #[tokio::test]
    async fn add_and_messages() {
        let m = InMemoryMemory::new();
        m.add(user_msg("你好")).await;
        m.add(user_msg("在吗")).await;
        let msgs = m.messages().await;
        assert_eq!(msgs.len(), 2);
    }

    #[tokio::test]
    async fn clear_works() {
        let m = InMemoryMemory::new();
        m.add(user_msg("x")).await;
        m.clear().await;
        assert!(m.messages().await.is_empty());
    }
}