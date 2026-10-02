pub mod react;
// pub mod reflect;
// pub mod plan;

#[cfg(test)]
pub (crate) mod mock;


use std::sync::Arc;
use crate::llm::LlmError;
use crate::llm::client::LlmClient;
use crate::memory::memory::{Memory, user_msg};
use crate::tools::registry::ToolRegistry;


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    ReAct,
    Reflect,
    PlanSolve,
}

pub struct Agent {
    pub(crate) llm: Arc<dyn LlmClient>,
    pub(crate) memory: Arc<dyn Memory>,
    pub(crate) tools: ToolRegistry,
    pub(crate) max_iterations: usize,
    pub(crate) max_reflections: usize,
    pub(crate) mode: Mode,
    pub(crate) verbose: bool,
    pub(crate) system_prompt: String,
}

impl Agent {
    pub fn new(
        llm: Arc<dyn LlmClient>,
        memory: Arc<dyn Memory>,
        tools: ToolRegistry, 
        system_prompt: String
    ) -> Self {
        // max_iterations 默认 8
        Self {
            llm,
            memory,
            tools,
            max_iterations: 8,
            max_reflections: 3,
            mode: Mode::ReAct,
            verbose: false,
            system_prompt,
        }
    }

    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn set_verbose(&mut self, on: bool) {
        self.verbose = on;
    }

    pub fn verbose(&self) -> bool {
        self.verbose
    }

    pub async fn ask(&self, input: &str) -> Result<String, LlmError> {
        self.memory.add(user_msg(input)).await;
        match self.mode {
            Mode::ReAct => self.react_loop(&self.memory, false).await,
            Mode::Reflect => unimplemented!("Reflect 模式尚未实现"),
            Mode::PlanSolve => unimplemented!("PlanSolve 模式尚未实现"),
        }
    }
}


#[cfg(test)]
mod test {
    use super::*;

    use crate::agent::mock::{MockLlm, content_events, tool_call_script};
    use crate::llm::ChatStreamEvent;
    use crate::memory::memory::InMemoryMemory;
    use crate::tools::calculator::Calculator;
    use async_openai::types::chat::{ChatCompletionRequestMessage, ChatCompletionRequestToolMessageContent};


    /// 组装一个注册了 calculator 的 Agent，返回 agent 和可供检查的 memory
    fn make_agent(scripts: Vec<Vec<ChatStreamEvent>>) -> (Agent, Arc<dyn Memory>) {
        let llm: Arc<dyn LlmClient> = Arc::new(MockLlm::new(scripts));
        let memory: Arc<dyn Memory> = Arc::new(InMemoryMemory::new());
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(Calculator {}));
        (
            Agent::new(llm, memory.clone(), tools, "you are helpful assistant".to_string()),
            memory,
        )
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

    fn bare_agent(scripts: Vec<Vec<ChatStreamEvent>>) -> Agent {
        let llm: Arc<dyn LlmClient> = Arc::new(MockLlm::new(scripts));
        let memory: Arc<dyn Memory> = Arc::new(InMemoryMemory::new());
        let tools = ToolRegistry::new();
        Agent::new(llm, memory, tools, "you are helpful assistant".to_string())
    }

    #[test]
    fn default_mode_is_react() {
        let agent = bare_agent(vec![]);
        assert_eq!(agent.mode(), Mode::ReAct);
        assert!(!agent.verbose());
    }

    #[test]
    fn set_mode_and_verbose() {
        let mut agent = bare_agent(vec![]);
        agent.set_mode(Mode::Reflect);
        assert_eq!(agent.mode(), Mode::Reflect);
        agent.set_verbose(true);
        assert!(agent.verbose());
    }
}
