use async_openai::{
    config::OpenAIConfig,
    types::chat::{ChatCompletionRequestMessage, ChatCompletionTool, ChatCompletionTools, CreateChatCompletionRequestArgs},
    Client,
};
use async_trait::async_trait;
use futures::stream::{self, StreamExt};

use crate::llm::{ChatStream, ChatStreamEvent, LlmError};

#[async_trait]
pub trait LlmClient: Send + Sync {
    async fn chat(
        &self,
        messages: Vec<ChatCompletionRequestMessage>,
        tools: Vec<ChatCompletionTool>,
    ) -> Result<ChatStream, LlmError>;
}

pub struct OpenAiClient {
    client: Client<OpenAIConfig>,
}

impl OpenAiClient {
    pub fn new() -> Self {
        Self { client: Client::new() }
    }
}

impl Default for OpenAiClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl LlmClient for OpenAiClient {
    async fn chat(
        &self,
        messages: Vec<ChatCompletionRequestMessage>,
        tools: Vec<ChatCompletionTool>,
    ) -> Result<ChatStream, LlmError> {
        let request = CreateChatCompletionRequestArgs::default()
            .model(crate::config::MODEL)
            .messages(messages)
            // SDK 的 tools 字段是 Vec<ChatCompletionTools> 枚举，把每个工具包一层 Function 变体
            .tools(tools.into_iter().map(ChatCompletionTools::Function).collect::<Vec<_>>())
            .stream(true)
            .build()?;

        let resp = self.client.chat().create_stream(request).await?;

        // 把 SDK 的流映射成我们自己的 ChatStreamEvent 流。
        // 一个 chunk 可能同时带 content 和 tool_calls，所以用 flat_map
        // 把每个 chunk 展开成 0..n 个事件。
        let events = resp.flat_map(|result| {
            let mut out: Vec<Result<ChatStreamEvent, LlmError>> = Vec::new();
            match result {
                Err(e) => out.push(Err(LlmError::OpenAI(e))),
                Ok(chunk) => {
                    // chunk.choices 理论上至少有一个；防御性地取第一个
                    if let Some(choice) = chunk.choices.into_iter().next() {
                        let delta = choice.delta;
                        if let Some(content) = delta.content {
                            out.push(Ok(ChatStreamEvent::Content(content)));
                        }
                        if let Some(tool_calls) = delta.tool_calls {
                            for tc in tool_calls {
                                // name 和 args 都装在 function 里，拆开喂给事件
                                let (name, args) = match tc.function {
                                    Some(f) => (f.name, f.arguments.unwrap_or_default()),
                                    None => (None, String::new()),
                                };
                                out.push(Ok(ChatStreamEvent::ToolCallDelta {
                                    index: tc.index as usize,
                                    id: tc.id,
                                    name,
                                    args,
                                }));
                            }
                        }
                    }
                }
            }
            stream::iter(out)
        });

        Ok(Box::pin(events))
    }
}