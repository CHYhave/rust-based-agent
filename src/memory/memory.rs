use async_openai::types::chat::{ChatCompletionMessageToolCalls, ChatCompletionRequestAssistantMessage, ChatCompletionRequestAssistantMessageContent, ChatCompletionRequestAssistantMessageContentPart, ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage, ChatCompletionRequestSystemMessageContent, ChatCompletionRequestSystemMessageContentPart, ChatCompletionRequestToolMessage, ChatCompletionRequestToolMessageContent, ChatCompletionRequestToolMessageContentPart, ChatCompletionRequestUserMessage, ChatCompletionRequestUserMessageContent, ChatCompletionRequestUserMessageContentPart,
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
    /// 全量替换（默认实现：clear + 逐条 add，FileMemory 的文件也能自动保持同步）
    async fn replace(&self, messages: Vec<ChatCompletionRequestMessage>) {
        self.clear().await;
        for m in messages {
            self.add(m).await;
        }
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

pub fn user_msg(s: &str) -> ChatCompletionRequestMessage {
    ChatCompletionRequestUserMessage::from(
        ChatCompletionRequestUserMessageContent::Text(s.to_string()),
    ).into()
}

pub fn assistant_msg(
    s: &str,
    tool_calls: Option<Vec<ChatCompletionMessageToolCalls>>,
) -> ChatCompletionRequestMessage {
    ChatCompletionRequestAssistantMessage {
        content: Some(ChatCompletionRequestAssistantMessageContent::Text(s.to_string())),
        tool_calls,
        ..Default::default()
    }.into()
}

pub fn system_msg(
    s: &str
) -> ChatCompletionRequestMessage {
    ChatCompletionRequestSystemMessage {
        content: ChatCompletionRequestSystemMessageContent::Text(s.to_string()).into(),
        name: None,
    }.into()
}

pub fn tool_msg(s: &str, tool_call_id: &str) -> ChatCompletionRequestMessage {
    ChatCompletionRequestToolMessage {
        content: ChatCompletionRequestToolMessageContent::Text(s.to_string()),
        tool_call_id: tool_call_id.to_string(),
    }.into()
}

/// 拼接文本片段，全空返回 None
fn join_or_none(parts: impl Iterator<Item = String>) -> Option<String> {
    let text: String = parts.collect();
    (!text.is_empty()).then_some(text)
}

/// 提取消息的可读文本，如 "user: 你好"；无正文（如纯 tool_calls 消息）返回 None
pub fn render_message(msg: &ChatCompletionRequestMessage) -> Option<String> {
    let (role, text) = match msg {
        ChatCompletionRequestMessage::System(m) => ("system", match &m.content {
            ChatCompletionRequestSystemMessageContent::Text(t) => Some(t.clone()),
            ChatCompletionRequestSystemMessageContent::Array(parts) => join_or_none(parts.iter().map(|p| match p {
                ChatCompletionRequestSystemMessageContentPart::Text(t) => t.text.clone(),
            })),
        }),
        ChatCompletionRequestMessage::User(m) => ("user", match &m.content {
            ChatCompletionRequestUserMessageContent::Text(t) => Some(t.clone()),
            ChatCompletionRequestUserMessageContent::Array(parts) => join_or_none(parts.iter().filter_map(|p| match p {
                ChatCompletionRequestUserMessageContentPart::Text(t) => Some(t.text.clone()),
                _ => None, // 图片/音频/文件部分跳过
            })),
        }),
        ChatCompletionRequestMessage::Assistant(m) => ("assistant", match &m.content {
            None => None, // 纯 tool_calls 消息
            Some(ChatCompletionRequestAssistantMessageContent::Text(t)) => Some(t.clone()),
            Some(ChatCompletionRequestAssistantMessageContent::Array(parts)) => join_or_none(parts.iter().map(|p| match p {
                ChatCompletionRequestAssistantMessageContentPart::Text(t) => t.text.clone(),
                ChatCompletionRequestAssistantMessageContentPart::Refusal(r) => r.refusal.clone(),
            })),
        }),
        ChatCompletionRequestMessage::Tool(m) => ("tool", match &m.content {
            ChatCompletionRequestToolMessageContent::Text(t) => Some(t.clone()),
            ChatCompletionRequestToolMessageContent::Array(parts) => join_or_none(parts.iter().map(|p| match p {
                ChatCompletionRequestToolMessageContentPart::Text(t) => t.text.clone(),
            })),
        }),
        _ => ("other", None), // Developer/Function 等其余变体
    };
    text.map(|t| format!("{role}: {t}"))
}


#[cfg(test)]
mod test {
    use super::*;

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

    #[tokio::test]
    async fn replace_swaps_all_messages() {
        let m = InMemoryMemory::new();
        m.add(user_msg("旧消息1")).await;
        m.add(user_msg("旧消息2")).await;
        m.replace(vec![system_msg("摘要"), user_msg("新消息")]).await;
        let msgs = m.messages().await;
        // 消息类型 derive 了 PartialEq，直接整值比较
        assert_eq!(msgs, vec![system_msg("摘要"), user_msg("新消息")]);
    }

    #[test]
    fn render_message_extracts_text() {
        assert_eq!(render_message(&user_msg("你好")), Some("user: 你好".to_string()));
        assert_eq!(render_message(&assistant_msg("回答", None)), Some("assistant: 回答".to_string()));
        assert_eq!(render_message(&tool_msg("3", "call_1")), Some("tool: 3".to_string()));
    }
}