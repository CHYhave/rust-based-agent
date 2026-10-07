use super::Agent;
use crate::llm::LlmError;
use crate::memory::memory::{InMemoryMemory, Memory, assistant_msg, system_msg, user_msg};
use std::sync::Arc;

pub(crate) const CRITIC_PROMPT: &str =
    "你是一位极其严格的评审专家。检查以下回答的事实错误、逻辑漏洞、遗漏信息。\
     如果回答已经足够好，只回复\"无需改进\"四个字，否则给出具体改进意见。";

pub(crate) const REFINE_PROMPT: &str =
    "你正在根据评审专家的反馈优化你的回答。请直接输出优化后的完整回答，不要输出解释。";

impl Agent {
    pub(crate) async fn reflect_run(&self, input: &str) -> Result<String, LlmError> { 
        let scratch: Arc<dyn Memory> = Arc::new(InMemoryMemory::new());
        scratch.add(system_msg(&self.system_prompt)).await;
        scratch.add(user_msg(input)).await;
        let mut draft = self.react_loop(&scratch, !self.verbose).await?;
        for _ in 0..self.max_reflections {
            let critique = self.chat_once(vec![system_msg(CRITIC_PROMPT), user_msg(&draft)], "[反思] 评审:").await?;
            if critique.trim().contains("无需改进")  {
                break;
            }
            draft = self.chat_once(vec![system_msg(REFINE_PROMPT), user_msg(&format!("原回答: \n{draft}\n\n评审意见: \n{critique}"))], "[反思] 优化:").await?;
        }
        if !self.verbose {
            // 静默模式下中间过程都没打印，循环结束后一次性输出最终稿
            println!("{draft}");
        }
        self.memory.add(assistant_msg(&draft, None)).await;
        Ok(draft)
     }
}


#[cfg(test)]
mod test {
    use crate::agent::mock::{MockLlm, content_events};
    use crate::agent::{Agent, Mode};
    use crate::llm::ChatStreamEvent;
    use crate::llm::client::LlmClient;
    use crate::memory::memory::{InMemoryMemory, Memory};
    use crate::tools::registry::ToolRegistry;
    use std::sync::Arc;

    fn reflect_agent(scripts: Vec<Vec<ChatStreamEvent>>) -> (Agent, Arc<dyn Memory>) {
        let llm: Arc<dyn LlmClient> = Arc::new(MockLlm::new(scripts));
        let memory: Arc<dyn Memory> = Arc::new(InMemoryMemory::new());
        let mut agent = Agent::new(llm, memory.clone(), ToolRegistry::new(), "sys".to_string());
        agent.set_mode(Mode::Reflect);
        (agent, memory)
    }

    #[tokio::test]
    async fn converges_in_first_round() {
        let (agent, memory) = reflect_agent(vec![
            content_events("初稿"),
            content_events("无需改进"),
        ]);
        let reply = agent.ask("一个问题").await.unwrap();
        assert_eq!(reply, "初稿");
        assert_eq!(memory.messages().await.len(), 2);
    }

    #[tokio::test]
    async fn refine_once_then_converges() {
        // draft → 批评 → 优化稿 → 「无需改进」→ 返回优化稿
        let (agent, _memory) = reflect_agent(vec![
            content_events("初稿"),
            content_events("批评：不够具体"),
            content_events("优化稿"),
            content_events("无需改进"),
        ]);
        let reply = agent.ask("一个问题").await.unwrap();
        assert_eq!(reply, "优化稿");
    }

    #[tokio::test]
    async fn stops_at_max_reflections() {
        // 评审永远不收敛：跑满 max_reflections=2 后返回最后一稿
        let (mut agent, _memory) = reflect_agent(vec![
            content_events("初稿"),
            content_events("批评1"),
            content_events("改稿1"),
            content_events("批评2"),
            content_events("改稿2"),
        ]);
        agent.max_reflections = 2;
        let reply = agent.ask("一个问题").await.unwrap();
        assert_eq!(reply, "改稿2");
    }
}