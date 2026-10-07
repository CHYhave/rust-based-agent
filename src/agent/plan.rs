use std::sync::Arc;

use crate::{agent::Agent, llm::LlmError, memory::memory::{InMemoryMemory, Memory, assistant_msg, system_msg, user_msg}};


pub(crate) const PLANNER_PROMPT: &str =
    "你是一个顶级的规划专家。把用户问题分解为有序步骤，每步一行，格式为\"1. xxx\"。\
     只输出步骤列表，不要输出其他内容。";

/// 执行器提示词四要素：原始问题、完整计划、历史步骤结果、当前步骤
fn executor_prompt(question: &str, plan: &[String], history: &str, step: &str) -> String {
    format!(
        "原始问题：{question}\n\n完整计划：\n{}\n\n已完成的步骤及结果：\n{}\n\n当前要执行的步骤：{step}\n\
         严格按照计划执行当前步骤，只输出该步骤的答案。",
        plan.join("\n"),
        if history.is_empty() { "（无）" } else { history },
    )
}

impl Agent {
    pub(crate) async fn plan_run(&self, input: &str) -> Result<String, LlmError> { 
        let plan_text = self.chat_once(vec![system_msg(PLANNER_PROMPT), user_msg(input)], "[计划]").await?;
        let steps = parse_plan(&plan_text);
        if steps.is_empty() {
            println!("[计划] 无法解析执行计划，降级为直接回答");
            return self.react_loop(&self.memory, false).await;
        }
        let scratch: Arc<dyn Memory> = Arc::new(InMemoryMemory::new());
        scratch.add(system_msg(&self.system_prompt)).await;
        let mut history = String::new();
        let mut answer = String::new();
        for (i, step) in steps.iter().enumerate() {
            let quiet = !self.verbose && (i + 1 < steps.len());
            scratch.add(user_msg(&executor_prompt(input, &steps, &history, step))).await;
            let result = self.react_loop(&scratch, quiet).await?;
            history.push_str(&format!("第 {} 步（{}）：\n{}\n", i + 1, step, result));
            answer = result;
        }
        self.memory.add(assistant_msg(&answer, None)).await;
        Ok(answer)    
    }
}


pub(crate) fn parse_plan(text: &str) -> Vec<String> {
    let mut plans = Vec::new();
    if text.is_empty() {
        return plans;
    }
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some((prefix, step)) = split_once_any(line, &['.', '、']) {
            let step = step.trim();
            if is_all_ascii_digits(prefix.trim()) && !step.is_empty() {
                plans.push(step.to_string());
            }
        }
    }
    plans
}

fn split_once_any<'a>(s: &'a str, delims: &[char]) -> Option<(&'a str, &'a str)> {
    let idx = s.find(|c| delims.contains(&c))?;
    let d = s[idx..].chars().next().unwrap();
    let rest = &s[idx + d.len_utf8()..];
    Some((&s[..idx], rest))
}

fn is_all_ascii_digits(s: &str) -> bool {
    // 注意：空字符串的 all() 返回 true，必须显式排除
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod test {
    use std::sync::Arc;

use crate::{agent::{Agent, Mode, mock::{MockLlm, content_events}}, llm::{ChatStreamEvent, client::LlmClient}, memory::memory::{InMemoryMemory, Memory}, tools::registry::ToolRegistry};

use super::*;

    fn plan_agent(scripts: Vec<Vec<ChatStreamEvent>>) -> (Agent, Arc<dyn Memory>) {
        let llm: Arc<dyn LlmClient> = Arc::new(MockLlm::new(scripts));
        let memory: Arc<dyn Memory> = Arc::new(InMemoryMemory::new());
        let mut agent = Agent::new(llm, memory.clone(), ToolRegistry::new(), "sys".to_string());
        agent.set_mode(Mode::PlanSolve);
        (agent, memory)
    }   

    #[tokio::test]
    async fn two_steps_returns_last_result() {
        // 规划两步 → 逐步执行 → 返回最后一步结果
        let (agent, memory) = plan_agent(vec![
            content_events("1. 查当前时间\n2. 计算 1+1"),
            content_events("现在是下午"),
            content_events("最终答案：2"),
        ]);
        let reply = agent.ask("现在几点，顺便算 1+1").await.unwrap();
        assert_eq!(reply, "最终答案：2");
        // 共享 memory 只有 user + assistant 两条
        assert_eq!(memory.messages().await.len(), 2);
    }

    #[tokio::test]
    async fn empty_plan_falls_back_to_react() {
        // 规划无法解析 → 降级 ReAct
        let (agent, memory) = plan_agent(vec![
            content_events("这个问题我没法分解"),
            content_events("直接回答"),
        ]);
        let reply = agent.ask("随便聊聊").await.unwrap();
        assert_eq!(reply, "直接回答");
        assert_eq!(memory.messages().await.len(), 2);
    }

    #[test]
    fn parses_numbered_lines() {
        let text = "1. 先查时间\n2. 再计算\n3. 总结结果";
        assert_eq!(parse_plan(text), vec!["先查时间", "再计算", "总结结果"]);
    }

    #[test]
    fn skips_junk_lines() {
        let text = "好的，计划如下：\n1. 步骤一\n\n2、步骤二\n希望对你有帮助";
        assert_eq!(parse_plan(text), vec!["步骤一", "步骤二"]);
    }

    #[test]
    fn empty_when_no_steps() {
        assert!(parse_plan("没有任何编号").is_empty());
        assert!(parse_plan("").is_empty());
    }

    #[test]
    fn rejects_empty_prefix_and_empty_step() {
        // ". 没有编号前缀"：分隔符前为空，不是步骤
        // "1. "：步骤内容为空，不收集
        assert!(parse_plan(". 没有编号前缀").is_empty());
        assert!(parse_plan("1. ").is_empty());
    }
}