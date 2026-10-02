use std::sync::Arc;

use async_trait::async_trait;
use crate::skill::accumulator::ToolCallAccumulator;
use crate::tools::registry::ToolRegistry;
use crate::memory::memory::{Memory, assistant_msg, tool_msg, user_msg};
use crate::llm::client::LlmClient;
use crate::llm::{ChatStream, ChatStreamEvent, LlmError};
use async_openai::types::chat::{ChatCompletionMessageToolCalls, ChatCompletionRequestMessage, ChatCompletionTool};
use futures::StreamExt;

pub struct Agent {
    llm: Arc<dyn LlmClient>,
    memory: Arc<dyn Memory>,
    tools: ToolRegistry,
    max_iterations: usize,
}

impl Agent {
    pub fn new(llm: Arc<dyn LlmClient>, memory: Arc<dyn Memory>, tools: ToolRegistry) -> Self {
        // max_iterations 默认 8
        Self {
            llm,
            memory,
            tools,
            max_iterations: 8,
        }
    }

    pub async fn ask(&self, input: &str) -> Result<String, LlmError> { 
        self.memory.add(user_msg(input)).await;
        let schema = self.tools.schemas();
        for _ in 0..self.max_iterations {
            let mut stream = self.llm.chat(self.memory.messages().await, schema.clone()).await?;
            let mut text = String::new();
            let mut acc = ToolCallAccumulator::new();
            while let Some(event) = stream.next().await {
                let event = event?;
                match event {
                    ChatStreamEvent::Content(d) => {
                        print!("{d}");
                        flush_stdout();
                        text.push_str(&d);
                    }
                    ChatStreamEvent::ToolCallDelta {index, id, name, args}  => {
                        acc.feed(index, id, name, Some(&args));
                    }
                    ChatStreamEvent::Done => break,
                }
            }
            let tool_calls = acc.into_tool_calls();
            if tool_calls.is_empty() {
                // 纯文本回复：assistant 消息入 memory，换行收尾，返回
                self.memory.add(assistant_msg(&text, None)).await;
                println!();
                return Ok(text);
            }

            // 有工具调用：先执行，收集结果消息
            // 注意 history 顺序必须是 assistant(带 tool_calls) 在前、tool 消息在后，
            // 所以等执行完再一起存
            let mut tool_msgs = Vec::new();
            for call in &tool_calls {
                let ChatCompletionMessageToolCalls::Function(fc) = call else { continue };
                let result = match self.tools.get(&fc.function.name) {
                    Some(tool) => tool.call(&fc.function.arguments).await,
                    None => Err(format!("工具未找到: {}", fc.function.name)),
                };
                // Ok 和 Err 都回传给模型，让它有机会自我纠正
                let content = result.unwrap_or_else(|e| format!("执行出错: {e}"));
                tool_msgs.push(tool_msg(&content, &fc.id));
            }

            self.memory.add(assistant_msg(&text, Some(tool_calls))).await;
            for m in tool_msgs {
                self.memory.add(m).await;
            }
            // 工具结果已入 memory，进入下一轮迭代
        }
        Err(LlmError::MaxIterations(self.max_iterations))
    }
}

fn flush_stdout() {
    use std::io::Write;
    std::io::stdout().flush().expect("flush stdout");
}



#[cfg(test)]
mod test {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use crate::memory::memory::InMemoryMemory;
    use crate::tools::calculator::Calculator;
    use async_openai::types::chat::ChatCompletionRequestToolMessageContent;

    struct MockLlm {
        scripts: Mutex<VecDeque<Vec<ChatStreamEvent>>>,
    }

    impl MockLlm {
        fn new(scripts: Vec<Vec<ChatStreamEvent>>) -> Self {
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

    fn content_events(s: &str) -> Vec<ChatStreamEvent> {
        vec![
            ChatStreamEvent::Content(s.to_string()),
            ChatStreamEvent::Done
        ]
    }

    /// 一轮"调用 calculator"的模型脚本
    fn tool_call_script() -> Vec<ChatStreamEvent> {
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

    /// 组装一个注册了 calculator 的 Agent，返回 agent 和可供检查的 memory
    fn make_agent(scripts: Vec<Vec<ChatStreamEvent>>) -> (Agent, Arc<dyn Memory>) {
        let llm: Arc<dyn LlmClient> = Arc::new(MockLlm::new(scripts));
        let memory: Arc<dyn Memory> = Arc::new(InMemoryMemory::new());
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(Calculator {}));
        (Agent::new(llm, memory.clone(), tools), memory)
    }

    #[tokio::test]
    async fn plain_reply() {
        let (agent, memory) = make_agent(vec![content_events("你好，我是 Agent")]);

        let reply = agent.ask("在吗").await.unwrap();
        assert_eq!(reply, "你好，我是 Agent");

        let msgs = memory.messages().await;
        assert_eq!(msgs.len(), 2);
        assert!(matches!(msgs[0], ChatCompletionRequestMessage::User(_)));
        assert!(matches!(msgs[1], ChatCompletionRequestMessage::Assistant(_)));
    }

    #[tokio::test]
    async fn react_with_tool() {
        let (agent, memory) = make_agent(vec![
            tool_call_script(),
            content_events("结果是 3"),
        ]);

        let reply = agent.ask("帮我算 1+2").await.unwrap();
        assert_eq!(reply, "结果是 3");

        // 历史应是 4 条：user / assistant(带tool_calls) / tool / assistant
        let msgs = memory.messages().await;
        assert_eq!(msgs.len(), 4);
        assert!(matches!(msgs[0], ChatCompletionRequestMessage::User(_)));

        match &msgs[1] {
            ChatCompletionRequestMessage::Assistant(m) => {
                let calls = m.tool_calls.as_ref().expect("assistant 应带 tool_calls");
                assert_eq!(calls.len(), 1);
            }
            other => panic!("第二条应是 assistant，实际: {other:?}"),
        }

        match &msgs[2] {
            ChatCompletionRequestMessage::Tool(m) => {
                assert_eq!(m.tool_call_id, "call_1");
                match &m.content {
                    ChatCompletionRequestToolMessageContent::Text(s) => assert_eq!(s, "3"),
                    other => panic!("tool 内容应是文本，实际: {other:?}"),
                }
            }
            other => panic!("第三条应是 tool，实际: {other:?}"),
        }

        assert!(matches!(msgs[3], ChatCompletionRequestMessage::Assistant(_)));
    }

    #[tokio::test]
    async fn max_iterations() {
        // 9 份相同脚本：max_iterations=8 轮全部耗尽，第 9 次 chat 不该发生
        let scripts: Vec<_> = (0..9).map(|_| tool_call_script()).collect();
        let (agent, _memory) = make_agent(scripts);

        let err = agent.ask("无限循环").await.unwrap_err();
        match err {
            LlmError::MaxIterations(n) => assert_eq!(n, 8),
            other => panic!("期待 MaxIterations(8)，实际: {other:?}"),
        }
    }
}