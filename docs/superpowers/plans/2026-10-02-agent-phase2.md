# Agent 第二期实现计划（引导式）

> **执行方式说明：** 本计划为「引导式」——契约（类型定义、函数签名、测试代码）全部给定，实现由学习者自己编写。
> 每任务流程：写测试 → 跑测试确认失败 → 自己实现 → 跑通 → 提交。
> 实现卡住时可以问 Claude，但先自己尝试 15 分钟。

**目标：** 在第一期 ReAct Agent 上新增两个经典范式——Reflection（执行→反思→优化）与 Plan-and-Solve（规划→逐步执行），REPL 斜杠命令切换模式与 verbose，中间过程可控可见。

**架构：** `agent.rs` 拆为 `agent/` 目录（mod/react/reflect/plan + 测试 mock）；`Mode` 枚举 + `ask` 分发；共享 memory 只存 user + 最终答复，中间过程走草稿 memory；REPL 抽到 `repl.rs`，斜杠命令解析为纯函数。

**对应设计文档：** `docs/superpowers/specs/2026-10-02-agent-phase2-design.md`

**技术栈：** 无新增依赖。

**关键前置事实（写代码时直接照用）：**
- 现有 `Agent::new(llm, memory, tools)` 三参签名将在 Task 1 变为四参（加 `system_prompt: String`）
- 消息构造函数已有：`user_msg` / `assistant_msg` / `system_msg` / `tool_msg`（`memory/memory.rs`）
- 现有 MockLlm 在 `src/agent.rs` 测试模块中，Task 1 把它搬到 `agent/mock.rs` 供所有子模块复用
- 子模块（如 `agent/reflect.rs`）中的 `impl Agent { ... }` 需要 `use super::Agent;`——同一模块树的多个文件可以各自为同一类型开 impl 块
- 测试模块是所在模块的后代，可以直接读写 `Agent` 的私有字段（如 `agent.max_reflections = 2;`）

---

## 文件结构总览

```
src/
├── main.rs            # Task 8：瘦身，只剩组装
├── repl.rs            # Task 7：REPL 循环 + 斜杠命令解析
├── agent/
│   ├── mod.rs         # Task 1-3：Agent、Mode、ask 分发、chat_once
│   ├── react.rs       # Task 1：ReAct 循环（参数化 memory/quiet）+ 第一期测试搬入
│   ├── mock.rs        # Task 1：MockLlm（#[cfg(test)]）
│   ├── reflect.rs     # Task 4：Reflection 范式
│   └── plan.rs        # Task 5-6：parse_plan + Plan-and-Solve 范式
└── （config / llm / memory / skill / tools 不动）
```

---

## Task 1: agent.rs → agent/ 目录拆分 + react_loop 参数化

**Files:**
- Delete: `src/agent.rs`
- Create: `src/agent/mod.rs`、`src/agent/react.rs`、`src/agent/mock.rs`
- Modify: `src/main.rs`（Agent::new 调用加参数）

本任务是**纯重构**：不新增行为，所有现有测试必须保持绿。

- [ ] **Step 1: agent/mod.rs — Agent 结构体与新字段**

```rust
pub mod react;
pub mod reflect;
pub mod plan;
#[cfg(test)]
pub(crate) mod mock;

use std::sync::Arc;
use crate::llm::LlmError;
use crate::llm::client::LlmClient;
use crate::memory::memory::Memory;
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
        system_prompt: String,
    ) -> Self {
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

    pub fn set_mode(&mut self, mode: Mode) { self.mode = mode; }
    pub fn mode(&self) -> Mode { self.mode }
    pub fn set_verbose(&mut self, on: bool) { self.verbose = on; }
    pub fn verbose(&self) -> bool { self.verbose }

    // ask 在 Task 2 改为分发；本任务先保持现有 ReAct 行为能跑
}
```

注意：字段改为 `pub(crate)` 是为了让子模块（react/reflect/plan）的 `impl Agent` 和测试能访问。

另外：mod.rs 现在就声明了 `pub mod reflect; pub mod plan;`，但这两个文件要到 Task 4/5 才有内容——本任务先**各建一个空文件**占位，否则编译不过。

- [ ] **Step 2: agent/mock.rs — 搬移 MockLlm（照抄现有代码，改 pub）**

```rust
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use async_openai::types::chat::{ChatCompletionRequestMessage, ChatCompletionTool};
use async_trait::async_trait;
use crate::llm::{ChatStream, ChatStreamEvent, LlmError};
use crate::llm::client::LlmClient;

pub(crate) struct MockLlm {
    scripts: Mutex<VecDeque<Vec<ChatStreamEvent>>>,
}

impl MockLlm {
    pub(crate) fn new(scripts: Vec<Vec<ChatStreamEvent>>) -> Self { /* 照抄原实现 */ }
}

#[async_trait]
impl LlmClient for MockLlm { /* 照抄原实现 */ }

pub(crate) fn content_events(s: &str) -> Vec<ChatStreamEvent> { /* 照抄原实现 */ }

pub(crate) fn tool_call_script() -> Vec<ChatStreamEvent> { /* 照抄原实现 */ }
```

- [ ] **Step 3: agent/react.rs — ReAct 循环参数化**

契约（把现 `ask` 的循环体改造成独立方法，差异：`memory` 变为参数、打印受 `quiet` 控制、不再自己 add user 消息）：

```rust
use super::Agent;
// …其余 use

impl Agent {
    /// ReAct 循环：在指定 memory 上跑「请求→工具→回传」直到模型不再调工具。
    /// quiet=true 时不打印正文流（仍累加返回）。
    pub(crate) async fn react_loop(
        &self,
        memory: &Arc<dyn Memory>,
        quiet: bool,
    ) -> Result<String, LlmError> { /* 自己实现 */ }
}
```

实现提示：
- 循环体与现 `ask` 几乎相同，三处改动：`self.memory` → 参数 `memory`；`ChatStreamEvent::Content` 分支的打印包在 `if !quiet { ... }` 里（累加 `text` 不受 quiet 影响）；纯文本收尾的 `println!()` 同样受 quiet 控制
- `flush_stdout` 一并搬入本文件

- [ ] **Step 4: 第一期三个测试搬入 react.rs 底部**

原样保留 `plain_reply` / `react_with_tool` / `max_iterations`，`make_agent` 更新为四参签名：

```rust
use super::*;
use crate::agent::mock::{MockLlm, content_events, tool_call_script};
use crate::memory::memory::InMemoryMemory;
use crate::tools::calculator::Calculator;
use std::sync::Arc;

fn make_agent(scripts: Vec<Vec<ChatStreamEvent>>) -> (Agent, Arc<dyn Memory>) {
    let llm: Arc<dyn LlmClient> = Arc::new(MockLlm::new(scripts));
    let memory: Arc<dyn Memory> = Arc::new(InMemoryMemory::new());
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(Calculator {}));
    (
        Agent::new(llm, memory.clone(), tools, "测试系统提示".to_string()),
        memory,
    )
}
```

注意：`ChatStreamEvent` / `ToolRegistry` 等类型需要在 react.rs 测试模块里自行 `use`（原来靠 `super::*` 带进来的不再免费）。

- [ ] **Step 5: ask 临时桥接 + main.rs 修调用**

`agent/mod.rs` 中（Task 2 会替换成分发；`user_msg` 需 `use crate::memory::memory::user_msg;`）：

```rust
impl Agent {
    pub async fn ask(&self, input: &str) -> Result<String, LlmError> {
        self.memory.add(user_msg(input)).await;
        self.react_loop(&self.memory, false).await
    }
}
```

`main.rs` 中 `Agent::new(...)` 加第四参 `system_prompt`（注意它前面已 `memory.add(system_msg(&system_prompt))`，直接移动所有权即可，顺序对的话不用 clone）。

- [ ] **Step 6: `cargo test` 全绿（28 个测试，与重构前一致）→ 提交**

`refactor: agent 模块拆分，react_loop 参数化 memory/quiet`

---

## Task 2: ask 分发 + Mode 切换

**Files:**
- Modify: `src/agent/mod.rs`

- [ ] **Step 1: 写测试（mod.rs 底部 `#[cfg(test)] mod tests`）**

```rust
use super::*;
use crate::agent::mock::{MockLlm, content_events};
use crate::memory::memory::InMemoryMemory;
use std::sync::Arc;

fn bare_agent(scripts: Vec<Vec<ChatStreamEvent>>) -> Agent {
    let llm: Arc<dyn LlmClient> = Arc::new(MockLlm::new(scripts));
    let memory: Arc<dyn Memory> = Arc::new(InMemoryMemory::new());
    Agent::new(llm, memory, ToolRegistry::new(), "sys".to_string())
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
    agent.set_verbose(true);
    assert_eq!(agent.mode(), Mode::Reflect);
    assert!(agent.verbose());
}
```

缺的类型自行 `use`（`ChatStreamEvent`、`ToolRegistry` 等）。

- [ ] **Step 2: 跑测试确认编译失败**（mock 模块/Task 1 未建时）或失败

- [ ] **Step 3: ask 改为分发**

```rust
pub async fn ask(&self, input: &str) -> Result<String, LlmError> {
    self.memory.add(user_msg(input)).await;
    match self.mode {
        Mode::ReAct => self.react_loop(&self.memory, false).await,
        // 占位：Task 4 / Task 6 会替换为真正的实现
        Mode::Reflect | Mode::PlanSolve => self.react_loop(&self.memory, false).await,
    }
}
```

- [ ] **Step 4: `cargo test` 全绿 → 提交** — `feat: Mode 枚举与 ask 分发骨架`

---

## Task 3: chat_once — 一次性静默调用

**Files:**
- Modify: `src/agent/mod.rs`

- [ ] **Step 1: 写测试（mod.rs 测试模块追加）**

```rust
#[tokio::test]
async fn chat_once_collects_stream() {
    let agent = bare_agent(vec![vec![
        ChatStreamEvent::Content("你好".into()),
        ChatStreamEvent::Content("世界".into()),
        ChatStreamEvent::Done,
    ]]);
    let out = agent
        .chat_once(vec![user_msg("打招呼")], "[测试] ")
        .await
        .unwrap();
    assert_eq!(out, "你好世界");
}

#[tokio::test]
async fn chat_once_passes_empty_tools() {
    // MockLlm 忽略 tools 参数，此测试主要验证调用不报错、能走通
    let agent = bare_agent(vec![content_events("ok")]);
    let out = agent.chat_once(vec![user_msg("x")], "").await.unwrap();
    assert_eq!(out, "ok");
}
```

（`user_msg` 需 `use crate::memory::memory::user_msg;`）

- [ ] **Step 2: 确认失败**

- [ ] **Step 3: 实现**

契约：

```rust
impl Agent {
    /// 一次性调用：不带工具，消费流收集正文为 String。
    /// verbose 时先打印一次 prefix，再逐 chunk 打印正文。
    async fn chat_once(
        &self,
        messages: Vec<ChatCompletionRequestMessage>,
        prefix: &str,
    ) -> Result<String, LlmError> { /* 自己实现 */ }
}
```

提示：
- `self.llm.chat(messages, vec![])`——空 tools
- 流里只需处理 `Content` 和 `Done`；`ToolCallDelta` 不该出现，忽略即可
- verbose 打印：用一个 `bool` 记录 prefix 是否已打印，只在首个 Content 前打印一次

- [ ] **Step 4: 测试绿 → 提交** — `feat: chat_once 一次性静默调用`

---

## Task 4: Reflection 范式

**Files:**
- Create: `src/agent/reflect.rs`

- [ ] **Step 1: 写测试（reflect.rs 底部）**

```rust
use super::*;
use crate::agent::mock::{MockLlm, content_events};
use crate::agent::{Agent, Mode};
use crate::memory::memory::InMemoryMemory;
use crate::tools::registry::ToolRegistry;
use crate::llm::client::LlmClient;
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
    // draft → 评审「无需改进」→ 直接收敛
    let (agent, memory) = reflect_agent(vec![
        content_events("初稿"),
        content_events("无需改进"),
    ]);
    let reply = agent.ask("一个问题").await.unwrap();
    assert_eq!(reply, "初稿");
    // 共享 memory 只有 user + assistant 两条
    assert_eq!(memory.messages().await.len(), 2);
}

#[tokio::test]
async fn refines_once_then_converges() {
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
```

缺 `ChatStreamEvent` 的 use 自己补。

- [ ] **Step 2: 确认失败**

- [ ] **Step 3: 实现**

契约：

```rust
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
    pub(crate) async fn reflect_run(&self, input: &str) -> Result<String, LlmError> { /* 自己实现 */ }
}
```

流程伪代码（自己翻译成 Rust）：
1. 建草稿 memory：`let scratch: Arc<dyn Memory> = Arc::new(InMemoryMemory::new());`，播种 `system_msg(&self.system_prompt)` + `user_msg(input)`
2. `draft = self.react_loop(&scratch, !self.verbose).await?`
3. `for _ in 0..self.max_reflections`：
   - `critique = self.chat_once(vec![system_msg(CRITIC_PROMPT), user_msg(&draft)], "[反思] 评审: ").await?`
   - `critique.contains("无需改进")` → break
   - `draft = self.chat_once(vec![system_msg(REFINE_PROMPT), user_msg(&format!("原回答：\n{draft}\n\n评审意见：\n{critique}"))], "[反思] 优化: ").await?`
4. `if !self.verbose { println!("{draft}"); }`（verbose 时中间过程已带前缀打印，不重复）
5. `self.memory.add(assistant_msg(&draft, None)).await;`（user 消息 ask 已加）
6. `Ok(draft)`

- [ ] **Step 4: ask 分发替换占位**

`Mode::Reflect => self.reflect_run(input).await,`

- [ ] **Step 5: 测试绿 → 提交** — `feat: Reflection 范式（反思-优化循环）`

---

## Task 5: parse_plan 纯函数

**Files:**
- Create: `src/agent/plan.rs`

- [ ] **Step 1: 写测试**

```rust
use super::parse_plan;

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
```

- [ ] **Step 2: 确认失败**

- [ ] **Step 3: 实现**

```rust
/// 从规划文本中提取编号步骤：支持 "1. xxx" 与 "1、xxx"，跳过其余行
pub(crate) fn parse_plan(text: &str) -> Vec<String> { /* 自己实现 */ }
```

提示：逐行 trim → 找第一个 `.` 或 `、` → 分隔符前的部分必须**非空且全是 ASCII 数字** → 取分隔符后的部分 trim，非空才收集。

- [ ] **Step 4: 测试绿 → 提交** — `feat: parse_plan 编号步骤解析`

---

## Task 6: Plan-and-Solve 范式

**Files:**
- Modify: `src/agent/plan.rs`

- [ ] **Step 1: 写测试（plan.rs 测试模块追加）**

```rust
use super::*;
use crate::agent::mock::{MockLlm, content_events};
use crate::agent::{Agent, Mode};
use crate::memory::memory::InMemoryMemory;
use crate::tools::registry::ToolRegistry;
use crate::llm::client::LlmClient;
use std::sync::Arc;

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
```

缺 `ChatStreamEvent` 的 use 自己补。

- [ ] **Step 2: 确认失败**

- [ ] **Step 3: 实现**

契约：

```rust
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
    pub(crate) async fn plan_run(&self, input: &str) -> Result<String, LlmError> { /* 自己实现 */ }
}
```

`plan_run` 流程伪代码：
1. `plan_text = self.chat_once(vec![system_msg(PLANNER_PROMPT), user_msg(input)], "[计划] ").await?`
2. `steps = parse_plan(&plan_text)`；为空 → `println!("[计划] 无法解析计划，降级为直接回答");` 并 `return self.react_loop(&self.memory, false).await;`（注意：这条路径下 assistant 消息由 react_loop 自己写入，不要重复 add）
3. 建草稿 memory（播种 `system_msg(&self.system_prompt)`），`history = String::new()`
4. `for (i, step) in steps.iter().enumerate()`：
   - `quiet = !self.verbose && i + 1 < steps.len()`（非 verbose 时只有最后一步打印）
   - 把 `user_msg(&executor_prompt(input, &steps, &history, step))` 加入草稿 memory
   - `result = self.react_loop(&scratch, quiet).await?`
   - `history += &format!("第 {} 步（{}）：\n{}\n", i + 1, step, result);`
5. `answer = history 里最后那个 result`（循环里留个变量记住）；`if !self.verbose { println!("{answer}"); }` 不需要——最后一步已经打印过了（quiet=false）。仅 `self.memory.add(assistant_msg(&answer, None)).await; Ok(answer)`

- [ ] **Step 4: ask 分发替换占位**

`Mode::PlanSolve => self.plan_run(input).await,`

- [ ] **Step 5: 测试绿 → 提交** — `feat: Plan-and-Solve 范式（规划-分步执行）`

---

## Task 7: repl.rs — REPL 抽出 + 斜杠命令

**Files:**
- Create: `src/repl.rs`
- Modify: `src/main.rs`（暂留 REPL 调用，Task 8 瘦身）

- [ ] **Step 1: 写测试（repl.rs 底部）**

```rust
use super::*;
use crate::agent::Mode;

#[test]
fn parses_mode_with_arg() {
    assert_eq!(parse_command("/mode reflect"), Some(Ok(ReplCommand::Mode(Some(Mode::Reflect)))));
    assert_eq!(parse_command("/mode plan"), Some(Ok(ReplCommand::Mode(Some(Mode::PlanSolve)))));
    assert_eq!(parse_command("/mode react"), Some(Ok(ReplCommand::Mode(Some(Mode::ReAct)))));
}

#[test]
fn parses_mode_query() {
    assert_eq!(parse_command("/mode"), Some(Ok(ReplCommand::Mode(None))));
}

#[test]
fn parses_verbose() {
    assert_eq!(parse_command("/verbose on"), Some(Ok(ReplCommand::Verbose(Some(true)))));
    assert_eq!(parse_command("/verbose off"), Some(Ok(ReplCommand::Verbose(Some(false)))));
    assert_eq!(parse_command("/verbose"), Some(Ok(ReplCommand::Verbose(None))));
}

#[test]
fn parses_help() {
    assert_eq!(parse_command("/help"), Some(Ok(ReplCommand::Help)));
}

#[test]
fn rejects_unknown_and_bad_args() {
    assert!(matches!(parse_command("/foo"), Some(Err(_))));
    assert!(matches!(parse_command("/mode fly"), Some(Err(_))));
    assert!(matches!(parse_command("/verbose maybe"), Some(Err(_))));
}

#[test]
fn non_command_returns_none() {
    assert_eq!(parse_command("你好"), None);
    assert_eq!(parse_command("exit"), None);
}
```

（`ReplCommand` 与 `Mode` 都要 derive `Debug, PartialEq`，否则 assert_eq 不工作）

- [ ] **Step 2: 确认失败**

- [ ] **Step 3: 实现 parse_command**

```rust
#[derive(Debug, PartialEq)]
pub enum ReplCommand {
    /// None = 查询当前模式
    Mode(Option<Mode>),
    /// None = 切换（取反）
    Verbose(Option<bool>),
    Help,
}

/// None = 非命令（普通对话）；Some(Err) = 未知命令/参数错误
pub fn parse_command(input: &str) -> Option<Result<ReplCommand, String>> { /* 自己实现 */ }
```

提示：`input.strip_prefix('/')`? → `split_whitespace()` 收集 → 按第一个词 match，`"mode" / "verbose" / "help"` 各自校验参数个数与取值，其余 `Some(Err(format!("未知命令: /{cmd}，输入 /help 查看可用命令")))`。

- [ ] **Step 4: 实现 repl::run**

契约（主体从 main.rs 现有循环搬来，加命令分支）：

```rust
use std::io::{self, BufRead, Write};
use crate::agent::{Agent, Mode};

pub const HELP: &str = "可用命令：
  /mode [react|reflect|plan]  切换/查询 Agent 范式
  /verbose [on|off]           开关/切换中间过程输出（反思细节、计划步骤）
  /help                       显示本帮助
  exit                        退出";

pub async fn run(mut agent: Agent) -> io::Result<()> { /* 自己实现 */ }
```

循环逻辑伪代码：
```
loop {
    print!("> "); flush; read_line → 0 break
    input = trim；"exit" break；空 continue
    match parse_command(input) {
        None => agent.ask(input).await 处理（现状：Err → eprintln! 继续）
        Some(Err(e)) => eprintln!("{e}")
        Some(Ok(cmd)) => match cmd {
            Mode(Some(m)) => { agent.set_mode(m); println!("已切换到 {m:?} 模式"); }
            Mode(None) => println!("当前模式: {:?}", agent.mode()),
            Verbose(Some(on)) => { agent.set_verbose(on); println!("verbose: {on}"); }
            Verbose(None) => { let v = !agent.verbose(); agent.set_verbose(v); println!("verbose: {v}"); }
            Help => println!("{HELP}"),
        }
    }
}
```

- [ ] **Step 5: 测试绿 → 提交** — `feat: REPL 斜杠命令（/mode /verbose /help）`

---

## Task 8: main.rs 瘦身 + 端到端手动验证

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: main.rs 瘦身为纯组装**

```rust
mod agent;
mod config;
mod llm;
mod memory;
mod repl;
mod skill;
mod tools;

// …use

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let (metas, contents) = discover_skills(Path::new("skills"));
    let system_prompt = build_system_prompt(&metas);

    let memory = InMemoryMemory::new();
    memory.add(system_msg(&system_prompt)).await;

    let llm = OpenAiClient::new();

    let mut tool_registry = ToolRegistry::new();
    tool_registry.register(Arc::new(GetTime {}));
    tool_registry.register(Arc::new(Calculator {}));
    tool_registry.register(Arc::new(LoadSkillTool::new(Arc::new(contents))));

    let agent = Agent::new(Arc::new(llm), Arc::new(memory), tool_registry, system_prompt);
    repl::run(agent).await?;
    Ok(())
}

// build_system_prompt 保留在此（照抄现有实现）
```

- [ ] **Step 2: `cargo build` 通过、`cargo test` 全绿 → 提交** — `refactor: main 瘦身，REPL 移交 repl 模块`

- [ ] **Step 3: 手动验证（`cargo run`）**

- `/help` → 打印命令列表
- 普通对话 → 行为与第一期一致（ReAct 默认模式）
- `/mode` → 显示当前模式；`/mode reflect` → 问一个开放问题（如「Rust 的所有权机制讲清楚」）→ 只输出最终答案
- `/verbose on` 再问一次 → 能看到 `[反思] 评审:` / `[反思] 优化:` 过程
- `/mode plan` → 「先告诉我现在几点，再算 (3+4)*5」→ 看到两步执行；`/verbose off` 后只看到最后一步
- 计划无法解析时（如闲聊输入）→ 打印降级提示并正常回答
- `/mode fly` → 错误提示不崩溃；`exit` 退出

- [ ] **Step 4: 修掉验证中发现的问题 → 提交** — `feat: 第二期端到端验证`

---

## 完成定义（Definition of Done）

- `cargo test` 全绿（第一期 28 个 + 本期新增约 15 个）
- REPL 三种模式 + verbose 开关 + 斜杠命令全部手动验证通过
- 每任务一次提交，历史清晰
