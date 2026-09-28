# Agent 第一期实现计划（引导式）

> **执行方式说明：** 本计划为「引导式」——契约（trait 签名、类型定义、测试代码）全部给定，实现由学习者自己编写。
> 每任务流程：写测试 → 跑测试确认失败 → 自己实现 → 跑通 → 提交。
> 实现卡住时可以问 Claude，但先自己尝试 15 分钟。

**目标：** 实现一个基于 DeepSeek + async-openai 的终端 Agent：REPL 对话 + trait 抽象 + function calling ReAct 循环 + 流式输出 + Claude Code 式 Skills。

**架构：** `Agent` 只依赖 `Arc<dyn LlmClient>` / `Arc<dyn Memory>` / `ToolRegistry`；async-openai 只出现在 `llm/client.rs`。流式响应通过自定义 `ChatStreamEvent` 枚举进入 Agent，tool_calls 增量由 `ToolCallAccumulator` 还原。

**技术栈：** Rust 2024 edition、async-openai 0.42、tokio、async-trait、futures、serde_json、chrono。

**已核实的 SDK 事实（写代码时直接照用）：**
- 消息枚举：`ChatCompletionRequestMessage::{System, User, Assistant, Tool}`，且有 `From<具体消息结构体> for ChatCompletionRequestMessage` 转换
- 构建 user 消息：`ChatCompletionRequestUserMessage::from(ChatCompletionRequestUserMessageContent::Text(s))`
- 请求构建：`CreateChatCompletionRequestArgs::default().model(...).messages(...).tools(...).stream(true).build()?`
- 流式：`client.chat().create_stream(request).await?` 返回 `Pin<Box<dyn Stream<Item = Result<CreateChatCompletionStreamResponse, OpenAIError>> + Send>>`
- 流 chunk 结构：`resp.choices[0].delta.content: Option<String>`，`delta.tool_calls: Option<Vec<ChatCompletionMessageToolCallChunk>>`，chunk 字段：`index: u32`、`id: Option<String>`、`function: Option<FunctionCallStream{ name, arguments }>`
- 完整 tool_call（用于回放进历史）：`ChatCompletionMessageToolCalls::Function(ChatCompletionMessageToolCall { id, function: FunctionCall { name, arguments } })`
- tool schema：`ChatCompletionTool { function: FunctionObject { name, description, parameters } }`（`ChatCompletionTool` 有 `Default`）
- tool 结果消息：`ChatCompletionRequestToolMessage { content: ChatCompletionRequestToolMessageContent::Text(s), tool_call_id }`
- assistant 消息结构体字段：`ChatCompletionRequestAssistantMessage { content: Option<...Content::Text(String)>, tool_calls: Option<Vec<ChatCompletionMessageToolCalls>>, ... }`

---

## 文件结构总览

```
src/
├── main.rs            # Task 11：REPL + 组装
├── config.rs          # Task 1
├── agent.rs           # Task 10：ReAct 循环 + build_system_prompt
├── llm/
│   ├── mod.rs         # Task 1：LlmError、ChatStreamEvent、ChatStream 别名
│   └── client.rs      # Task 9：LlmClient trait + OpenAiClient
├── memory/
│   ├── mod.rs
│   └── memory.rs      # Task 2：Memory trait + InMemoryMemory
├── tools/
│   ├── mod.rs         # Task 3：Tool trait
│   ├── registry.rs    # Task 3
│   ├── get_time.rs    # Task 5
│   ├── calculator.rs  # Task 4
│   └── load_skill.rs  # Task 7
└── skill/
    ├── mod.rs
    ├── discovery.rs   # Task 6
    └── accumulator.rs # Task 8

skills/                # Task 12：示例技能（数据目录，非代码）
└── demo-skill/
    └── SKILL.md
```

---

## Task 1: 依赖、config、llm 基础类型

**Files:**
- Modify: `Cargo.toml`
- Create: `src/config.rs`
- Create: `src/llm/mod.rs`
- Modify: `src/main.rs`（清空旧演示代码，暂时只留空 main 让编译通过）

- [ ] **Step 1: 更新 Cargo.toml**

```toml
[dependencies]
async-openai = { version = "0.42", features = ["chat-completion"] }
async-trait = "0.1"
tokio = { version = "1", features = ["macros", "rt-multi-thread", "sync"] }
serde_json = "1"
futures = "0.3"
chrono = "0.4"
```

注意：`byot` feature 不再 needed。跑 `cargo build` 确认依赖能拉下来。

- [ ] **Step 2: config.rs**

```rust
pub const MODEL: &str = "deepseek-chat";
```

- [ ] **Step 3: llm/mod.rs — 公开类型契约（照抄即可）**

```rust
pub mod client;

use std::pin::Pin;
use futures::Stream;

/// LlmClient 返回的流：事件序列，以 Done 结束
pub type ChatStream = Pin<Box<dyn Stream<Item = Result<ChatStreamEvent, LlmError>> + Send>>;

#[derive(Debug)]
pub enum ChatStreamEvent {
    /// 助手正文的增量片段
    Content(String),
    /// 一个 tool_call 的增量（index 标识第几个调用）
    ToolCallDelta {
        index: usize,
        id: Option<String>,
        name: Option<String>,
        args: String,
    },
    /// 流结束
    Done,
}

#[derive(Debug)]
pub enum LlmError {
    OpenAI(async_openai::error::OpenAIError),
    MaxIterations(usize),
}

impl std::fmt::Display for LlmError { /* 自己实现：两个变体各一行 */ }
impl std::error::Error for LlmError { /* source() 对 OpenAI 变体返回 Some */ }
impl From<async_openai::error::OpenAIError> for LlmError { /* 自己实现 */ }
```

提示：`Display` 手动实现即可，不引 thiserror——练一次手写错误类型。

- [ ] **Step 4: main.rs 清空为占位**

```rust
#[tokio::main]
async fn main() {
    println!("agent placeholder");
}
```

- [ ] **Step 5: 提交**

```bash
git add -A && git commit -m "chore: 第 1 期骨架 — 依赖、config、llm 基础类型"
```

---

## Task 2: Memory

**Files:**
- Create: `src/memory/mod.rs`（`pub mod memory;`）
- Create: `src/memory/memory.rs`

- [ ] **Step 1: 写测试（在 memory.rs 底部 `#[cfg(test)] mod tests`）**

```rust
use super::*;
use async_openai::types::chat::{ChatCompletionRequestMessage, ChatCompletionRequestUserMessage, ChatCompletionRequestUserMessageContent};

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
```

- [ ] **Step 2: 跑测试确认失败** — `cargo test` 编译错误（类型不存在）

- [ ] **Step 3: 实现**

契约：

```rust
#[async_trait]
pub trait Memory: Send + Sync {
    async fn messages(&self) -> Vec<ChatCompletionRequestMessage>;
    async fn add(&self, msg: ChatCompletionRequestMessage);
    async fn clear(&self);
}

pub struct InMemoryMemory { /* Mutex<Vec<ChatCompletionRequestMessage>> */ }
impl InMemoryMemory { pub fn new() -> Self }
impl Memory for InMemoryMemory { /* ... */ }
impl Default for InMemoryMemory { /* new() */ }
```

提示：
- 为什么用 `Mutex` 而不是直接 `Vec`？因为 trait 方法是 `&self`，需要**内部可变性**。这是本任务的核心知识点。
- `async_trait` 用法：`#[async_trait] impl Memory for InMemoryMemory { ... }`，方法内 `.lock().unwrap()` 后再操作。

- [ ] **Step 4: `cargo test` 全绿**

- [ ] **Step 5: 提交** — `feat: Memory trait 与内存实现`

---

## Task 3: Tool trait + Registry

**Files:**
- Create: `src/tools/mod.rs`（`pub mod registry; pub mod get_time; pub mod calculator; pub mod load_skill;` + Tool trait 定义）
- Create: `src/tools/registry.rs`

- [ ] **Step 1: 写测试（registry.rs 底部）**

```rust
use super::*;
use serde_json::json;

struct FakeTool;
#[async_trait]
impl Tool for FakeTool {
    fn name(&self) -> &str { "fake" }
    fn description(&self) -> &str { "a fake tool" }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({"type": "object", "properties": {}})
    }
    async fn call(&self, _args: &str) -> Result<String, String> { Ok("ok".into()) }
}

#[test]
fn register_and_get() {
    let mut reg = ToolRegistry::new();
    reg.register(Arc::new(FakeTool));
    assert!(reg.get("fake").is_some());
    assert!(reg.get("nope").is_none());
}

#[test]
fn schemas_match_tools() {
    let mut reg = ToolRegistry::new();
    reg.register(Arc::new(FakeTool));
    let schemas = reg.schemas();
    assert_eq!(schemas.len(), 1);
    assert_eq!(schemas[0].function.name, "fake");
}
```

- [ ] **Step 2: 跑测试确认失败**

- [ ] **Step 3: 实现**

`tools/mod.rs` 中：

```rust
use async_trait::async_trait;

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> serde_json::Value;
    async fn call(&self, arguments: &str) -> Result<String, String>;
}
```

`registry.rs` 中：

```rust
pub struct ToolRegistry { /* HashMap<String, Arc<dyn Tool>> */ }
impl ToolRegistry {
    pub fn new() -> Self
    pub fn register(&mut self, tool: Arc<dyn Tool>)
    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>>
    pub fn schemas(&self) -> Vec<ChatCompletionTool>
}
impl Default for ToolRegistry { /* new() */ }
```

`schemas()` 把每个 tool 的 name/description/parameters 组装成 `ChatCompletionTool { function: FunctionObject { name, description: Some(...), parameters: Some(...) } }`。

- [ ] **Step 4: `cargo test` 全绿**（先注释掉 mod.rs 里还不存在的 get_time/calculator/load_skill 声明）

- [ ] **Step 5: 提交** — `feat: Tool trait 与工具注册表`

---

## Task 4: calculator（自写递归下降解析器）

**Files:**
- Create: `src/tools/calculator.rs`

- [ ] **Step 1: 写测试**

```rust
#[test]
fn basic() { assert_eq!(eval("1+2*3").unwrap(), 7.0); }
#[test]
fn parens() { assert_eq!(eval("(1+2)*3").unwrap(), 9.0); }
#[test]
fn unary() { assert_eq!(eval("-3+5").unwrap(), 2.0); }
#[test]
fn div_zero() { assert!(eval("1/0").is_err()); }
#[test]
fn syntax_err() { assert!(eval("1+").is_err()); assert!(eval("(1").is_err()); }
#[test]
fn spaces() { assert_eq!(eval(" 2 * ( 3 + 4 ) ").unwrap(), 14.0); }
```

- [ ] **Step 2: 确认失败**

- [ ] **Step 3: 实现**

结构建议（可以自由发挥，这是练手重点）：
- 词法分析：把字符串切成 `Vec<Token>`（`Num(f64)` / `Plus` / `Minus` / `Star` / `Slash` / `LParen` / `RParen`），遇到非法字符返回 `Err`
- 语法分析（递归下降，一个 token 游标 `pos: usize`）：
  - `expr := term (('+'|'-') term)*`
  - `term := factor (('*'|'/') factor)*`
  - `factor := Num | '(' expr ')' | '-' factor`
- 对外暴露 `fn eval(input: &str) -> Result<f64, String>`，`pub struct Calculator` 实现 `Tool`（参数名 `expression`）

- [ ] **Step 4: `cargo test` 全绿**

- [ ] **Step 5: 提交** — `feat: calculator 工具（递归下降解析器）`

---

## Task 5: get_time

**Files:**
- Create: `src/tools/get_time.rs`

- [ ] **Step 1: 测试**

```rust
#[tokio::test]
async fn returns_time() {
    let t = GetTime;
    let out = t.call("{}").await.unwrap();
    assert!(!out.is_empty());
}
```

- [ ] **Step 2-3: 实现** — `pub struct GetTime;` 实现 `Tool`：`parameters_schema` 返回空 object；`call` 用 `chrono::Local::now()` 格式化输出。

- [ ] **Step 4: 测试绿**

- [ ] **Step 5: 提交** — `feat: get_time 工具`

---

## Task 6: Skill discovery

**Files:**
- Create: `src/skill/mod.rs`（`pub mod discovery; pub mod accumulator;`）
- Create: `src/skill/discovery.rs`

- [ ] **Step 1: 测试**（用 `std::env::temp_dir()` 造临时目录，写完记得清理）

```rust
// 伪代码思路：
// 1. temp_dir()/unique_name/skills/my-skill/SKILL.md 写入：
//    ---
//    name: my-skill
//    description: 演示技能
//    ---
//    正文内容……
// 2. discover_skills(&root) 返回的 metas 含 name="my-skill"、description="演示技能"
// 3. contents["my-skill"] 以 "正文内容" 开头
// 4. 目录不存在 / 空目录 → 返回空，不 panic
```

- [ ] **Step 2: 确认失败**

- [ ] **Step 3: 实现**

```rust
pub struct SkillMeta {
    pub name: String,
    pub description: String,
}

/// 扫描 dir 下每个 <name>/SKILL.md，返回 (元信息列表, name→正文 映射)
pub fn discover_skills(dir: &Path) -> (Vec<SkillMeta>, HashMap<String, String>)
```

frontmatter 解析提示：文件以 `---` 开头行 → 读到下一个 `---` 行之间的内容按 `key: value` 逐行解析（`split_once(':')` + trim）。不引 YAML 库。

- [ ] **Step 4: 测试绿**

- [ ] **Step 5: 提交** — `feat: skill 目录扫描与 frontmatter 解析`

---

## Task 7: load_skill 工具

**Files:**
- Create: `src/tools/load_skill.rs`

- [ ] **Step 1: 测试**

```rust
#[tokio::test]
async fn loads_existing() {
    let mut contents = HashMap::new();
    contents.insert("demo".to_string(), "做某事的完整指令".to_string());
    let tool = LoadSkillTool::new(Arc::new(contents));
    assert_eq!(tool.name(), "load_skill");
    let out = tool.call(r#"{"name":"demo"}"#).await.unwrap();
    assert_eq!(out, "做某事的完整指令");
}

#[tokio::test]
async fn missing_skill_reports_error() {
    let tool = LoadSkillTool::new(Arc::new(HashMap::new()));
    let out = tool.call(r#"{"name":"nope"}"#).await;
    assert!(out.is_err());   // Err 文本会被回传给模型
}
```

- [ ] **Step 2-3: 实现** — `pub struct LoadSkillTool { contents: Arc<HashMap<String, String>> }`，`new(contents)`；`call` 内用 `serde_json::from_str::<serde_json::Value>(arguments)` 取 `name` 字段。

- [ ] **Step 4: 测试绿**

- [ ] **Step 5: 提交** — `feat: load_skill 工具`

---

## Task 8: ToolCallAccumulator

**Files:**
- Create: `src/skill/accumulator.rs`（放这里是因为下一期可能挪走，本期按文件结构约定）

- [ ] **Step 1: 测试**

```rust
#[test]
fn accumulates_across_chunks() {
    let mut acc = ToolCallAccumulator::new();
    acc.feed(0, Some("call_1".into()), Some("calculator".into()), None);
    acc.feed(0, None, None, Some(r#"{"expres"#));
    acc.feed(0, None, None, Some(r#"sion":"1+2"}"#));
    let calls = acc.into_tool_calls();
    assert_eq!(calls.len(), 1);
    match &calls[0] {
        ChatCompletionMessageToolCalls::Function(c) => {
            assert_eq!(c.id, "call_1");
            assert_eq!(c.function.name, "calculator");
            assert_eq!(c.function.arguments, r#"{"expression":"1+2"}"#);
        }
        _ => panic!("unexpected variant"),
    }
}

#[test]
fn interleaved_calls() {
    let mut acc = ToolCallAccumulator::new();
    acc.feed(0, Some("a".into()), Some("t1".into()), None);
    acc.feed(1, Some("b".into()), Some("t2".into()), None);
    acc.feed(0, None, None, Some("{}".into()));
    acc.feed(1, None, None, Some("{}".into()));
    let calls = acc.into_tool_calls();
    assert_eq!(calls.len(), 2);
}
```

- [ ] **Step 2: 确认失败**

- [ ] **Step 3: 实现** — 内部 `BTreeMap<usize, Slot>`，`Slot { id: Option<String>, name: Option<String>, args: String }`。`feed` 时：`get_or_insert` slot；`id`/`name` 只在 `Some` 时覆盖；`args` 拼接。`into_tool_calls` 跳过没有 `id` 的 slot，按 index 排序产出。

- [ ] **Step 4: 测试绿**

- [ ] **Step 5: 提交** — `feat: 流式 tool_calls 累加器`

---

## Task 9: LlmClient trait + OpenAiClient

**Files:**
- Create: `src/llm/client.rs`

- [ ] **Step 1: trait 契约（照抄）**

```rust
#[async_trait]
pub trait LlmClient: Send + Sync {
    async fn chat(
        &self,
        messages: Vec<ChatCompletionRequestMessage>,
        tools: Vec<ChatCompletionTool>,
    ) -> Result<ChatStream, LlmError>;
}
```

- [ ] **Step 2: 实现 OpenAiClient**

```rust
pub struct OpenAiClient { client: Client<OpenAIConfig> }
impl OpenAiClient { pub fn new() -> Self }
impl Default for OpenAiClient { /* new() */ }
```

`chat` 实现步骤：
1. `CreateChatCompletionRequestArgs::default().model(crate::config::MODEL).messages(messages).tools(tools).stream(true).build()?`（注意 `tools` 为空 vec 时请求里要不要带——本期直接传，DeepSeek 容忍空 tools）
2. `let resp = self.client.chat().create_stream(request).await?;`
3. 用 `futures::stream::unfold` 或 `resp.map(...)` 把 SDK chunk 映射成 `ChatStreamEvent`：
   - `choices[0].delta.content` 有值 → `Content`
   - `delta.tool_calls` 逐个 chunk → `ToolCallDelta { index: chunk.index as usize, id: chunk.id, name: chunk.function.name, args: chunk.function.arguments }`
   - 映射错误 → `LlmError::OpenAI`
4. `Ok(Box::pin(mapped_stream))`

提示：`map` 的闭包里每个 SDK chunk 可能同时含 content 和 tool_calls——但一个闭包只能产出一个事件。两种解法：把 chunk 展平成多个事件（用 `flat_map` + `iter`），或先只处理常见形态。推荐 `flat_map`，是练习 Iterator/Stream 组合子的好机会。

- [ ] **Step 3: 编译验证** — `cargo build` 通过（本任务不写网络测试）

- [ ] **Step 4: 提交** — `feat: LlmClient trait 与 OpenAI 流式实现`

---

## Task 10: Agent — ReAct 循环

**Files:**
- Create: `src/agent.rs`

- [ ] **Step 1: 写测试（mock LlmClient）**

在 `agent.rs` 的 `#[cfg(test)]` 中实现 Mock：

```rust
struct MockLlm {
    scripts: Mutex<VecDeque<Vec<ChatStreamEvent>>>,  // 每次 chat() 弹出一个脚本
}
#[async_trait]
impl LlmClient for MockLlm {
    async fn chat(&self, _m: Vec<ChatCompletionRequestMessage>, _t: Vec<ChatCompletionTool>)
        -> Result<ChatStream, LlmError>
    {
        let script = self.scripts.lock().unwrap().pop_front().unwrap();
        Ok(Box::pin(futures::stream::iter(
            script.into_iter().map(Ok)
        )))
    }
}

fn content_events(s: &str) -> Vec<ChatStreamEvent> {
    vec![ChatStreamEvent::Content(s.to_string()), ChatStreamEvent::Done]
}
```

三个测试：

```rust
#[tokio::test]
async fn plain_reply() {
    // Mock 脚本：只返回正文 → ask 返回该正文；memory 里有 user + assistant 两条
}

#[tokio::test]
async fn react_with_tool() {
    // 脚本1: ToolCallDelta(call_1, calculator, 完整args) + Done
    // 脚本2: content_events("结果是 3")
    // → ask 返回 "结果是 3"；memory 中应有 user / assistant(带tool_calls) / tool 三条
    // 注意：Agent 的 tools 注册表要真的注册 Calculator，否则得到 "工具未找到" 的 tool 消息
}

#[tokio::test]
async fn max_iterations() {
    // Mock 每次都返回同一个 tool_call 脚本 → ask 返回 Err(LlmError::MaxIterations)
}
```

- [ ] **Step 2: 确认失败**

- [ ] **Step 3: 实现**

```rust
pub struct Agent {
    llm: Arc<dyn LlmClient>,
    memory: Arc<dyn Memory>,
    tools: ToolRegistry,
    max_iterations: usize,
}

impl Agent {
    pub fn new(llm: Arc<dyn LlmClient>, memory: Arc<dyn Memory>, tools: ToolRegistry) -> Self
    // max_iterations 默认 8

    pub async fn ask(&self, input: &str) -> Result<String, LlmError> { /* ReAct 循环 */ }
}
```

`ask` 内部流程（伪代码，自己翻译成 Rust）：
1. `memory.add(user 消息)`
2. `for _ in 0..self.max_iterations`：
   - `let mut stream = self.llm.chat(self.memory.messages().await, self.tools.schemas()).await?;`
   - 消费流：`Content(d)` → `print!("{d}"); flush;` 并累加字符串；`ToolCallDelta{..}` → `acc.feed(...)`；`Done` → break
   - 用累加结果构造 assistant 消息：`ChatCompletionRequestAssistantMessage { content: Text(累加正文), tool_calls: Some(acc 结果), ..Default::default() }`，`memory.add`
   - 若 tool_calls 为空 → `println!()` 收尾，返回正文
   - 否则逐个 tool_call：registry 查找 → 找到则 `call(args)`，`Ok/Err` 都包成 `ChatCompletionRequestToolMessage`（`tool_call_id` 用 call 的 id）；未找到 → 错误文本。全部 `memory.add`
3. 循环耗尽 → `Err(LlmError::MaxIterations(self.max_iterations))`

提示：
- 构造 assistant 结构体需要 `..Default::default()` 补其余字段
- 流消费用 `futures::StreamExt` 的 `while let Some(ev) = stream.next().await`
- 这是全项目最难的任务，独立完成成就感最大

- [ ] **Step 4: `cargo test` 全绿**

- [ ] **Step 5: 提交** — `feat: Agent ReAct 循环（function calling + 流式）`

---

## Task 11: main.rs — 组装与 REPL

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: 实现**（无单元测试，Task 12 手动验证）

要点：
1. `discover_skills(Path::new("skills"))` → metas + contents
2. 构建 system prompt：

```
你是终端智能体。可用技能：
- {name}: {description}
当任务匹配某个技能时，先用 load_skill 工具加载完整指令并严格遵循。
```

3. `memory.add(system 消息)`（启动时，先于任何对话）
4. Registry 注册三个工具：`GetTime`、`Calculator`、`LoadSkillTool::new(Arc::new(contents))`
5. REPL 循环：

```
print!("> "); stdout.flush();
read_line → 0 字节（EOF）或 trim=="exit" → break
trim 为空 → continue
agent.ask(&input).await → Ok 时已打印；Err → eprintln! 后继续
```

6. `main` 返回 `Result<(), Box<dyn Error>>` 或自行处理错误

- [ ] **Step 2: `cargo build` 通过**

- [ ] **Step 3: 提交** — `feat: REPL 组装与 system prompt`

---

## Task 12: 端到端手动验证

**Files:**
- Create: `skills/demo-skill/SKILL.md`

- [ ] **Step 1: 写示例技能**

```markdown
---
name: demo-skill
description: 演示技能 —— 把回答翻译成喵星人语言（每句话结尾加"喵"）
---

当用户要求你使用此技能时，把之后的每一句回复都以"喵"结尾。
```

- [ ] **Step 2: 逐项手动验证**

```bash
cargo run
```
- 普通对话 → 流式逐字输出
- 「现在几点」→ 触发 get_time
- 「帮我算 (3+4)*5」→ 触发 calculator，且能看到两轮请求
- 「用 demo-skill 回答我：你好」→ 触发 load_skill，回复带"喵"
- `exit` 退出；输入 garbage 不崩溃

- [ ] **Step 3: 修掉验证中发现的问题**

- [ ] **Step 4: 提交** — `feat: demo-skill 示例与端到端验证`

---

## 完成定义（Definition of Done）

- `cargo test` 全绿、`cargo clippy` 无警告（可选但推荐）
- REPL 四类对话（普通/工具/技能/退出）全部手动验证通过
- 每期任务一次提交，历史清晰
