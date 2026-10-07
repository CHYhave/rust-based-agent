# Agent 第三期实现计划（引导式）

> **执行方式说明：** 本计划为「引导式」——契约（类型定义、函数签名、测试代码）全部给定，实现由学习者自己编写。
> 每任务流程：写测试 → 跑测试确认失败 → 自己实现 → 跑通 → 提交。
> 实现卡住时可以问 Claude，但先自己尝试 15 分钟。

**目标：** 为 Agent 增加长期记忆与上下文工程能力：JSONL 持久化（FileMemory）、关键词检索（Retriever trait + KeywordRetriever + search_memory 工具）、上下文策略（ContextStrategy trait + Window/Summarize 两实现 + /strategy /compact 命令）。

**架构：** `Memory` trait 不动只加 `replace` 默认实现，`FileMemory` 是第二个实现；`ContextStrategy.apply(&memory)` 在 react_loop 每次请求前产出实际发送的消息列表（Window=纯视图，Summarize=就地压缩）；system prompt 不存入任何 memory，由 react_loop 请求时 prepend；检索经 `search_memory` 工具暴露给模型自主调用。

**对应设计文档：** `docs/superpowers/specs/2026-10-03-agent-phase3-design.md`

**技术栈：** tokio 增加 `fs` feature（唯一依赖变更）。

**关键前置事实（写代码时直接照用）：**
- `ChatCompletionRequestMessage` 已 derive `Serialize + Deserialize + Clone + PartialEq`（`#[serde(tag = "role")]`），可直接 `serde_json::to_string` / `from_str`
- 消息构造辅助在 `src/memory/memory.rs`：`user_msg` / `assistant_msg` / `system_msg` / `tool_msg`
- `Agent` 字段均为 `pub(crate)`，子模块/测试可直接读写（如 `agent.max_reflections = 2`）
- MockLlm 在 `src/agent/mock.rs`（`#[cfg(test)]`），其他模块测试可 `use crate::agent::mock::{...}`
- 现有 `Agent::chat_once` 在 `src/agent.rs`，verbose 时带前缀逐 chunk 打印
- 现有 `react_loop` 在 `src/agent/react.rs`，签名 `(&self, memory: &Arc<dyn Memory>, quiet: bool)`

---

## 文件结构总览

```
src/
├── memory/
│   ├── memory.rs        # Task 1：+replace 默认实现、+render_message
│   └── file.rs          # Task 2：FileMemory
├── memory.rs            # Task 2：+pub mod file;
├── llm.rs               # Task 3：+collect_chat 自由函数
├── context.rs           # Task 4-5：ContextStrategy + Window + Summarize + ContextKind
├── retrieval.rs         # Task 6：Retriever + KeywordRetriever + tokenize
├── tools/search_memory.rs # Task 7
├── tools.rs             # Task 7：+pub mod search_memory;
├── agent.rs             # Task 4-5：+context/window/summarize 字段、set_context/set_strategy/compact
├── agent/react.rs       # Task 4：prepend system + 应用 context 策略
├── agent/reflect.rs     # Task 4：删掉 system 播种行
├── agent/plan.rs        # Task 4：删掉 system 播种行
├── agent/mock.rs        # Task 4：+received 请求记录
├── repl.rs              # Task 8：+/strategy /compact
├── main.rs              # Task 4 删 system add；Task 8：FileMemory + 注册 search_memory
└── .gitignore           # Task 8：+.agent_history.jsonl
```

---

## Task 1: Memory::replace 默认实现 + render_message

**Files:**
- Modify: `src/memory/memory.rs`

- [ ] **Step 1: 写测试（memory.rs 测试模块追加）**

```rust
#[tokio::test]
async fn replace_swaps_all_messages() {
    let m = InMemoryMemory::new();
    m.add(user_msg("旧消息1")).await;
    m.add(user_msg("旧消息2")).await;
    m.replace(vec![system_msg("摘要"), user_msg("新消息")]).await;
    let msgs = m.messages().await;
    assert_eq!(msgs.len(), 2);
    assert!(matches!(msgs[0], ChatCompletionRequestMessage::System(_)));
    assert!(matches!(msgs[1], ChatCompletionRequestMessage::User(_)));
}

#[test]
fn render_message_extracts_text() {
    assert_eq!(render_message(&user_msg("你好")), Some("user: 你好".to_string()));
    assert_eq!(render_message(&assistant_msg("回答", None)), Some("assistant: 回答".to_string()));
    assert_eq!(render_message(&tool_msg("3", "call_1")), Some("tool: 3".to_string()));
}
```

测试模块缺的 use 自己补（`ChatCompletionRequestMessage` 等）。

- [ ] **Step 2: 跑测试确认失败** — `cargo test memory` 编译错误（方法不存在）

- [ ] **Step 3: 实现**

`Memory` trait 内追加（默认实现，FileMemory 靠它自动正确）：

```rust
async fn replace(&self, messages: Vec<ChatCompletionRequestMessage>) {
    self.clear().await;
    for m in messages {
        self.add(m).await;
    }
}
```

自由函数（与 user_msg 等辅助同级）：

```rust
/// 提取消息的可读文本，如 "user: 你好"；无正文（如纯 tool_calls 消息）返回 None
pub fn render_message(msg: &ChatCompletionRequestMessage) -> Option<String> { /* 自己实现 */ }
```

提示：
- match 四个变体 `System/User/Assistant/Tool`，前缀分别为 `"system: "` / `"user: "` / `"assistant: "` / `"tool: "`
- 各变体的 content 枚举都有 `Text(String)` 和 `Array(Vec<...>)` 两种形态；Text 直接用，Array 拼接其中的文本部分（全空则返回 None）
- assistant 消息的 content 是 `Option<...>`，`None`（纯 tool_calls）→ 返回 None

- [ ] **Step 4: `cargo test memory` 全绿 → 提交** — `feat: Memory::replace 与 render_message`

---

## Task 2: FileMemory — JSONL 持久化

**Files:**
- Modify: `Cargo.toml`（tokio 加 `fs` feature）
- Modify: `src/memory.rs`（+`pub mod file;`）
- Create: `src/memory/file.rs`

- [ ] **Step 1: 加依赖**

`Cargo.toml`：`tokio = { version = "1", features = ["macros", "rt-multi-thread", "sync", "fs"] }`

- [ ] **Step 2: 写测试（file.rs 底部）**

```rust
use super::*;
use crate::memory::memory::{assistant_msg, user_msg, Memory};

fn temp_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("me-test-{}-{}.jsonl", name, std::process::id()))
}

#[tokio::test]
async fn save_and_load_roundtrip() {
    let path = temp_path("roundtrip");
    {
        let m = FileMemory::load(path.clone()).await.unwrap();
        m.add(user_msg("你好")).await;
        m.add(assistant_msg("你好呀", None)).await;
    } // 离开作用域，模拟退出
    let m2 = FileMemory::load(path.clone()).await.unwrap();
    let msgs = m2.messages().await;
    assert_eq!(msgs.len(), 2);
    assert!(matches!(msgs[0], ChatCompletionRequestMessage::User(_)));
    std::fs::remove_file(&path).ok();
}

#[tokio::test]
async fn load_missing_file_is_empty() {
    let m = FileMemory::load(temp_path("not-exist")).await.unwrap();
    assert!(m.messages().await.is_empty());
}

#[tokio::test]
async fn bad_lines_are_skipped() {
    let path = temp_path("badline");
    let good = serde_json::to_string(&user_msg("有效")).unwrap();
    std::fs::write(&path, format!("{good}\n这不是JSON\n{good}\n")).unwrap();
    let m = FileMemory::load(path.clone()).await.unwrap();
    assert_eq!(m.messages().await.len(), 2);
    std::fs::remove_file(&path).ok();
}

#[tokio::test]
async fn clear_truncates_file() {
    let path = temp_path("clear");
    let m = FileMemory::load(path.clone()).await.unwrap();
    m.add(user_msg("x")).await;
    m.clear().await;
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
    std::fs::remove_file(&path).ok();
}
```

缺 `ChatCompletionRequestMessage` 的 use 自己补。

- [ ] **Step 3: 确认失败** — `cargo test file` 编译错误

- [ ] **Step 4: 实现**

```rust
use std::path::PathBuf;
use async_openai::types::chat::ChatCompletionRequestMessage;
use async_trait::async_trait;
use tokio::sync::Mutex;

pub struct FileMemory {
    messages: Mutex<Vec<ChatCompletionRequestMessage>>,
    path: PathBuf,
}

impl FileMemory {
    /// 加载 JSONL；文件不存在 → 空历史；坏行跳过并 eprintln 警告
    pub async fn load(path: PathBuf) -> std::io::Result<Self> { /* 自己实现 */ }
}

#[async_trait]
impl Memory for FileMemory {
    async fn messages(&self) -> Vec<ChatCompletionRequestMessage> { /* 同 InMemory */ }
    async fn add(&self, msg: ChatCompletionRequestMessage) { /* 压 Vec + 追加一行 JSON */ }
    async fn clear(&self) { /* 清 Vec + 截断文件 */ }
    // replace 用 trait 默认实现（clear + 逐条 add，文件自动正确）
}
```

提示：
- `tokio::fs::read_to_string(&path).await`，用 `Err(e) if e.kind() == ErrorKind::NotFound => 空` 区分「不存在」和「真错误」
- 追加写：`OpenOptions::new().create(true).append(true).open(&path).await` + `AsyncWriteExt::write_all`
- **trait 方法返回 `()`，IO 错误无法传播**——`add`/`clear` 里写文件失败就 `eprintln!` 警告后继续（内存态仍正确）
- 每行一条：`serde_json::to_string(&msg)` + `"\n"`

- [ ] **Step 5: `cargo test file` 全绿 → 提交** — `feat: FileMemory JSONL 持久化`

---

## Task 3: llm::collect_chat — 抽取共享收集函数

**Files:**
- Modify: `src/llm.rs`
- Modify: `src/agent.rs`（chat_once 重构）

纯重构：不新增行为，现有测试保持绿。

- [ ] **Step 1: llm.rs 追加自由函数（照抄）**

```rust
use std::sync::Arc;
use async_openai::types::chat::ChatCompletionRequestMessage;
use futures::StreamExt;
use crate::llm::client::LlmClient;

/// 一次性调用：不带工具，消费流收集正文为 String（不打印）
pub async fn collect_chat(
    llm: &Arc<dyn LlmClient>,
    messages: Vec<ChatCompletionRequestMessage>,
) -> Result<String, LlmError> {
    let mut stream = llm.chat(messages, vec![]).await?;
    let mut text = String::new();
    while let Some(event) = stream.next().await {
        if let ChatStreamEvent::Content(d) = event? {
            text.push_str(&d);
        }
    }
    Ok(text)
}
```

- [ ] **Step 2: agent.rs 的 chat_once 重构为基于 collect_chat**

```rust
async fn chat_once(
    &self,
    messages: Vec<ChatCompletionRequestMessage>,
    prefix: &str,
) -> Result<String, LlmError> {
    if !self.verbose {
        return crate::llm::collect_chat(&self.llm, messages).await;
    }
    // …保留现有 verbose 逐 chunk 打印逻辑（不动）
}
```

- [ ] **Step 3: `cargo test` 全绿（现有 chat_once 测试恰好覆盖静默路径）→ 提交** — `refactor: 抽取 llm::collect_chat`

---

## Task 4: ContextStrategy + WindowStrategy + system 前置统一

**Files:**
- Create: `src/context.rs`
- Modify: `src/main.rs`（+`mod context;`，删 `memory.add(system_msg...)` 行）
- Modify: `src/agent.rs`（+context 字段、set_context）
- Modify: `src/agent/react.rs`（prepend system + 应用策略）
- Modify: `src/agent/reflect.rs`、`src/agent/plan.rs`（各删一行 system 播种）
- Modify: `src/agent/mock.rs`（+received 记录）

**本任务各文件改动对照表（先看清楚再动手）：**

| 文件 | 改动 |
|---|---|
| `context.rs` | 新建：trait + WindowStrategy |
| `agent.rs` | +`pub(crate) context` 字段、new 默认 Window(30)、+`set_context` |
| `react.rs` | 请求消息改为 `[system] + context.apply(memory)` |
| `reflect.rs` | 删 `scratch.add(system_msg(...))` 行 |
| `plan.rs` | 删 `scratch.add(system_msg(...))` 行 |
| `main.rs` | `mod context;`；删 `memory.add(system_msg(&system_prompt)).await;` 行 |
| `mock.rs` | +`received` 字段记录每次 chat 收到的消息 |

⚠️ 从此规则统一：**任何 memory 都不存 system 消息**，system 由 react_loop 请求时 prepend。

- [ ] **Step 1: 写 context.rs 测试（文件底部）**

```rust
use super::*;
use crate::memory::memory::{assistant_msg, user_msg};

fn msgs(n: usize) -> Vec<ChatCompletionRequestMessage> {
    (0..n).map(|i| user_msg(&format!("消息{i}"))).collect()
}

#[test]
fn window_under_limit_unchanged() {
    let m = msgs(5);
    assert_eq!(window(&m, 10).len(), 5);
}

#[test]
fn window_over_limit_keeps_recent() {
    let m = msgs(10);
    let out = window(&m, 3);
    assert_eq!(out.len(), 3);
    assert_eq!(out, m[7..].to_vec());
}
```

- [ ] **Step 2: 实现 context.rs**

```rust
use std::sync::Arc;
use async_openai::types::chat::ChatCompletionRequestMessage;
use async_trait::async_trait;
use crate::llm::LlmError;
use crate::memory::memory::Memory;

#[async_trait]
pub trait ContextStrategy: Send + Sync {
    /// 返回本次请求实际发送的消息列表
    async fn apply(
        &self,
        memory: &Arc<dyn Memory>,
    ) -> Result<Vec<ChatCompletionRequestMessage>, LlmError>;
}

pub struct WindowStrategy {
    pub max_messages: usize,
}

impl WindowStrategy {
    pub fn new(max_messages: usize) -> Self { Self { max_messages } }
}

/// 纯函数：取末尾 max 条（视图变换，不碰 memory）
fn window(messages: &[ChatCompletionRequestMessage], max: usize) -> Vec<ChatCompletionRequestMessage> {
    /* 自己实现（提示：saturating_sub + 切片 + to_vec） */
}

#[async_trait]
impl ContextStrategy for WindowStrategy { /* apply = window(&memory.messages().await, self.max_messages) */ }
```

- [ ] **Step 3: mock.rs 增加请求记录**

```rust
pub(crate) struct MockLlm {
    scripts: Mutex<VecDeque<Vec<ChatStreamEvent>>>,
    /// 每次 chat 收到的消息列表（供测试断言上下文策略生效）
    pub(crate) received: Mutex<Vec<Vec<ChatCompletionRequestMessage>>>,
}
```

`new` 里初始化 `received: Mutex::new(Vec::new())`；`chat` 开头 `self.received.lock().unwrap().push(messages);`（参数名的下划线去掉）。

- [ ] **Step 4: Agent 接线**

`agent.rs`：`Agent` 加字段 `pub(crate) context: Arc<dyn ContextStrategy>`；`new` 里 `context: Arc::new(WindowStrategy::new(30))`；加方法：

```rust
pub fn set_context(&mut self, strategy: Arc<dyn ContextStrategy>) {
    self.context = strategy;
}
```

`react.rs` 的循环内，把原来的 `self.llm.chat(memory.messages().await, ...)` 改为：

```rust
let mut msgs = vec![system_msg(&self.system_prompt)];
msgs.extend(self.context.apply(memory).await?);
let mut stream = self.llm.chat(msgs, schema.clone()).await?;
```

（`system_msg` 需 import。）`reflect.rs` / `plan.rs` / `main.rs` 按对照表删行。

- [ ] **Step 5: 写集成测试（agent.rs 测试模块追加）**

```rust
#[tokio::test]
async fn window_strategy_truncates_request() {
    let mock = Arc::new(MockLlm::new(vec![content_events("ok")]));
    let llm: Arc<dyn LlmClient> = mock.clone();
    let memory: Arc<dyn Memory> = Arc::new(InMemoryMemory::new());
    let mut agent = Agent::new(llm, memory.clone(), ToolRegistry::new(), "sys".to_string());
    agent.set_context(Arc::new(crate::context::WindowStrategy::new(3)));

    for i in 0..5 {
        memory.add(user_msg(&format!("问题{i}"))).await;
        memory.add(assistant_msg(&format!("回答{i}"), None)).await;
    }
    agent.ask("新问题").await.unwrap();

    let received = mock.received.lock().unwrap();
    // 第一条是 react_loop prepend 的 system，其后最多 3 条窗口内历史
    assert!(matches!(received[0][0], ChatCompletionRequestMessage::System(_)));
    assert_eq!(received[0].len(), 4);
}
```

- [ ] **Step 6: `cargo test` 全绿（现有 47 个测试不受影响——system 不进 memory，断言条数不变）→ 提交** — `feat: ContextStrategy 与 WindowStrategy，system 请求时前置`

---

## Task 5: SummarizeStrategy + compact

**Files:**
- Modify: `src/context.rs`
- Modify: `src/agent.rs`（+window/summarize/strategy_kind 字段、set_strategy/strategy/compact）

- [ ] **Step 1: 写测试（context.rs 测试模块追加）**

```rust
use crate::agent::mock::{MockLlm, content_events};
use crate::memory::memory::InMemoryMemory;
use crate::llm::client::LlmClient;

#[tokio::test]
async fn summarize_under_threshold_returns_as_is() {
    let llm: Arc<dyn LlmClient> = Arc::new(MockLlm::new(vec![])); // 不该被调用，调了就 panic
    let s = SummarizeStrategy::new(llm, 8000, 3);
    let memory: Arc<dyn Memory> = Arc::new(InMemoryMemory::new());
    memory.add(user_msg("短对话")).await;
    let out = s.apply(&memory).await.unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(memory.messages().await.len(), 1);
}

#[tokio::test]
async fn summarize_over_threshold_compacts_in_place() {
    let llm: Arc<dyn LlmClient> = Arc::new(MockLlm::new(vec![content_events("摘要内容")]));
    let s = SummarizeStrategy::new(llm, 10, 2); // 阈值 10 字符，必然触发
    let memory: Arc<dyn Memory> = Arc::new(InMemoryMemory::new());
    for i in 0..5 {
        memory.add(user_msg(&format!("第 {i} 条消息内容")).await;
    }
    let out = s.apply(&memory).await.unwrap();
    assert_eq!(out.len(), 3); // 1 摘要 + 2 近期
    assert_eq!(memory.messages().await.len(), 3);
    assert!(matches!(memory.messages().await[0], ChatCompletionRequestMessage::System(_)));
}

#[tokio::test]
async fn summarize_too_few_messages_skip() {
    // 4 条消息、keep_recent=5：即使字符超阈值也没东西可压
    let llm: Arc<dyn LlmClient> = Arc::new(MockLlm::new(vec![]));
    let s = SummarizeStrategy::new(llm, 1, 5);
    let memory: Arc<dyn Memory> = Arc::new(InMemoryMemory::new());
    for i in 0..4 {
        memory.add(user_msg(&format!("很长很长很长很长很长的消息{i}"))).await;
    }
    let out = s.apply(&memory).await.unwrap();
    assert_eq!(out.len(), 4);
}
```

- [ ] **Step 2: 确认失败**

- [ ] **Step 3: 实现（context.rs 追加）**

```rust
use crate::llm::client::LlmClient;
use crate::llm::collect_chat;
use crate::memory::memory::{render_message, system_msg, user_msg};

pub const SUMMARIZE_PROMPT: &str =
    "你是对话摘要专家。把以下对话历史压缩为一段摘要，保留关键事实、用户偏好和待办事项。直接输出摘要。";

pub struct SummarizeStrategy {
    llm: Arc<dyn LlmClient>,
    pub threshold_chars: usize,
    pub keep_recent: usize,
}

impl SummarizeStrategy {
    pub fn new(llm: Arc<dyn LlmClient>, threshold_chars: usize, keep_recent: usize) -> Self { /* ... */ }

    /// 就地压缩：memory 重写为 [摘要 system 消息] + 最近 keep_recent 条
    pub(crate) async fn compact_memory(&self, memory: &Arc<dyn Memory>) -> Result<(), LlmError> { /* 自己实现 */ }
}

#[async_trait]
impl ContextStrategy for SummarizeStrategy { /* 自己实现 apply */ }
```

提示：
- `compact_memory`：`msgs.len() <= keep_recent` → 直接返回（没的可压）；否则 `split = len - keep_recent`，旧段 `msgs[..split]` 用 `render_message` 逐条渲染 join 成文本，`collect_chat` 发给 LLM（system=SUMMARIZE_PROMPT，user=旧段文本），新列表 = `[system_msg("以下是之前对话的摘要：\n{summary}")] + msgs[split..]`，`memory.replace(new).await`
- `apply`：总字符数（`render_message` 长度求和）≤ 阈值 → 原样返回；否则先 `compact_memory` 再返回 `memory.messages().await`

- [ ] **Step 4: Agent 增加策略切换与 compact（agent.rs）**

`context.rs` 加：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextKind { Window, Summarize }
```

`agent.rs`：`Agent` 加三个字段 `window: Arc<WindowStrategy>`、`summarize: Arc<SummarizeStrategy>`、`pub(crate) strategy_kind: ContextKind`；`new` 里：

```rust
let window = Arc::new(WindowStrategy::new(30));
let summarize = Arc::new(SummarizeStrategy::new(llm.clone(), 8000, 10));
// Self { ..., context: window.clone(), window, summarize, strategy_kind: ContextKind::Window }
```

方法：

```rust
pub fn set_strategy(&mut self, kind: ContextKind) {
    self.context = match kind {
        ContextKind::Window => self.window.clone(),
        ContextKind::Summarize => self.summarize.clone(),
    };
    self.strategy_kind = kind;
}
pub fn strategy(&self) -> ContextKind { self.strategy_kind }

/// /compact：无门槛立即压缩，返回 (压缩前条数, 压缩后条数)
pub async fn compact(&self) -> Result<(usize, usize), LlmError> { /* 自己实现 */ }
```

- [ ] **Step 5: 写 compact 测试（agent.rs 测试模块追加）**

```rust
#[tokio::test]
async fn compact_reduces_history() {
    let llm: Arc<dyn LlmClient> = Arc::new(MockLlm::new(vec![content_events("摘要")]));
    let memory: Arc<dyn Memory> = Arc::new(InMemoryMemory::new());
    let agent = Agent::new(llm, memory.clone(), ToolRegistry::new(), "sys".to_string());
    for i in 0..12 {
        memory.add(user_msg(&format!("消息{i}"))).await;
    }
    let (before, after) = agent.compact().await.unwrap();
    assert_eq!(before, 12);
    assert_eq!(after, 11); // 1 摘要 + keep_recent=10
}
```

- [ ] **Step 6: 测试绿 → 提交** — `feat: SummarizeStrategy 与 /compact 支撑`

---

## Task 6: retrieval.rs — 分词与关键词检索

**Files:**
- Create: `src/retrieval.rs`
- Modify: `src/main.rs`（+`mod retrieval;`）

- [ ] **Step 1: 写测试（retrieval.rs 底部）**

```rust
use super::*;

#[test]
fn tokenize_ascii_lowercase() {
    assert_eq!(tokenize("Hello Rust2024"), vec!["hello", "rust2024"]);
}

#[test]
fn tokenize_cjk_unigram_and_bigram() {
    assert_eq!(tokenize("ab 计算"), vec!["ab", "计", "算", "计算"]);
}

#[test]
fn tokenize_ignores_punctuation() {
    assert_eq!(tokenize("你好，世界！"), vec!["你", "好", "你好", "世", "界", "世界"]);
}

#[test]
fn search_ranks_by_term_matches() {
    let docs = vec![
        "今天天气很好".to_string(),
        "Rust 是一门编程语言".to_string(),
        "Rust 的宏很强大".to_string(),
    ];
    let hits = KeywordRetriever.search("Rust 语言", &docs, 3);
    // docs[1] 同时命中 rust/语/言/语言，docs[2] 只命中 rust，docs[0] 零分被过滤
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].index, 1);
    assert_eq!(hits[1].index, 2);
}

#[test]
fn search_top_k_and_zero_score() {
    let docs = vec!["Rust 语言".to_string(), "Rust 宏".to_string()];
    assert_eq!(KeywordRetriever.search("rust", &docs, 1).len(), 1);
    assert!(KeywordRetriever.search("xyz不存在", &docs, 3).is_empty());
}
```

- [ ] **Step 2: 确认失败**

- [ ] **Step 3: 实现（本任务的 tokenize/search 是管道代码，照抄即可；重点理解 trait 设计）**

```rust
pub struct SearchHit {
    pub index: usize,
    pub score: f64,
}

pub trait Retriever: Send + Sync {
    /// 按相关度降序返回 top_k 个命中（score > 0）
    fn search(&self, query: &str, docs: &[String], top_k: usize) -> Vec<SearchHit>;
}

pub struct KeywordRetriever;

/// ASCII 字母数字转小写词；CJK 逐字单字 + 相邻二元组
pub(crate) fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut word = String::new();
    let mut prev_cjk: Option<char> = None;
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            word.push(c.to_ascii_lowercase());
            prev_cjk = None;
        } else if ('\u{4e00}'..='\u{9fff}').contains(&c) {
            if !word.is_empty() {
                tokens.push(std::mem::take(&mut word));
            }
            tokens.push(c.to_string());
            if let Some(p) = prev_cjk {
                tokens.push(format!("{p}{c}"));
            }
            prev_cjk = Some(c);
        } else {
            if !word.is_empty() {
                tokens.push(std::mem::take(&mut word));
            }
            prev_cjk = None;
        }
    }
    if !word.is_empty() {
        tokens.push(word);
    }
    tokens
}

impl Retriever for KeywordRetriever {
    fn search(&self, query: &str, docs: &[String], top_k: usize) -> Vec<SearchHit> {
        let terms = tokenize(query);
        let mut hits: Vec<SearchHit> = docs
            .iter()
            .enumerate()
            .filter_map(|(i, d)| {
                let doc_tokens = tokenize(d);
                let score = terms.iter().filter(|t| doc_tokens.contains(t)).count() as f64;
                (score > 0.0).then_some(SearchHit { index: i, score })
            })
            .collect();
        hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
        hits.truncate(top_k);
        hits
    }
}
```

- [ ] **Step 4: 测试绿 → 提交** — `feat: 关键词检索器（CJK bigram 分词）`

---

## Task 7: search_memory 工具

**Files:**
- Create: `src/tools/search_memory.rs`
- Modify: `src/tools.rs`（+`pub mod search_memory;`）

- [ ] **Step 1: 写测试**

```rust
use super::*;
use crate::memory::memory::{assistant_msg, user_msg, InMemoryMemory, Memory};
use crate::retrieval::KeywordRetriever;
use crate::tools::Tool;

#[tokio::test]
async fn finds_relevant_memory() {
    let memory: Arc<dyn Memory> = Arc::new(InMemoryMemory::new());
    memory.add(user_msg("我喜欢吃火锅")).await;
    memory.add(assistant_msg("好的，记住了", None)).await;
    let tool = SearchMemoryTool::new(memory, Arc::new(KeywordRetriever));
    let out = tool.call(r#"{"query":"火锅"}"#).await.unwrap();
    assert!(out.contains("火锅"));
}

#[tokio::test]
async fn no_hit_returns_hint() {
    let memory: Arc<dyn Memory> = Arc::new(InMemoryMemory::new());
    let tool = SearchMemoryTool::new(memory, Arc::new(KeywordRetriever));
    let out = tool.call(r#"{"query":"不存在的东西"}"#).await.unwrap();
    assert!(out.contains("没有找到"));
}
```

缺 `Arc` 的 use 自己补。

- [ ] **Step 2: 确认失败**

- [ ] **Step 3: 实现**

```rust
pub struct SearchMemoryTool {
    memory: Arc<dyn Memory>,
    retriever: Arc<dyn Retriever>,
}

impl SearchMemoryTool {
    pub fn new(memory: Arc<dyn Memory>, retriever: Arc<dyn Retriever>) -> Self
}

#[async_trait]
impl Tool for SearchMemoryTool {
    fn name(&self) -> &str { "search_memory" }
    fn description(&self) -> &str {
        "搜索历史对话记忆。当需要回忆用户之前说过的信息时使用"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": { "query": { "type": "string", "description": "搜索关键词" } },
            "required": ["query"]
        })
    }
    async fn call(&self, arguments: &str) -> Result<String, String> { /* 自己实现 */ }
}
```

`call` 提示：
1. `serde_json` 解析 `query` 字段（缺字段返回 `Err("缺少 query 参数".into())`）
2. `memory.messages().await` → `filter_map(render_message)` 收集为 `Vec<String>`
3. `retriever.search(query, &docs, 3)`
4. 空结果 → `Ok("没有找到相关记忆".into())`；否则逐条 `"【记忆 {i}】{doc}"` 拼接返回

- [ ] **Step 4: 测试绿 → 提交** — `feat: search_memory 记忆检索工具`

---

## Task 8: REPL 新命令 + main 组装

**Files:**
- Modify: `src/repl.rs`
- Modify: `src/main.rs`
- Modify: `.gitignore`（+`.agent_history.jsonl`）

- [ ] **Step 1: 写测试（repl.rs 测试模块追加）**

```rust
#[test]
fn parses_strategy() {
    assert_eq!(
        parse_command("/strategy window"),
        Some(Ok(ReplCommand::Strategy(Some(ContextKind::Window))))
    );
    assert_eq!(
        parse_command("/strategy summarize"),
        Some(Ok(ReplCommand::Strategy(Some(ContextKind::Summarize))))
    );
    assert_eq!(parse_command("/strategy"), Some(Ok(ReplCommand::Strategy(None))));
    assert!(matches!(parse_command("/strategy x"), Some(Err(_))));
}

#[test]
fn parses_compact() {
    assert_eq!(parse_command("/compact"), Some(Ok(ReplCommand::Compact)));
}
```

（`ContextKind` 需 `use crate::context::ContextKind;`，它已 derive Debug + PartialEq。）

- [ ] **Step 2: 确认失败**

- [ ] **Step 3: parse_command 加分支 + ReplCommand 加变体**

```rust
pub enum ReplCommand {
    Mode(Option<Mode>),
    Verbose(Option<bool>),
    Strategy(Option<ContextKind>),
    Compact,
    Help,
}
```

`"strategy"` 分支：`[]` → `Strategy(None)`；`["window"]` / `["summarize"]` → 对应变体；其余 `Err("用法: /strategy [window|summarize]")`。`"compact"` → `Compact`。

HELP 更新为：

```rust
pub const HELP: &str = "可用命令：
  /mode [react|reflect|plan]     切换/查询 Agent 范式
  /verbose [on|off]              开关/切换中间过程输出
  /strategy [window|summarize]   切换/查询上下文策略
  /compact                       立即压缩历史对话
  /help                          显示本帮助
  exit                           退出";
```

- [ ] **Step 4: repl::run 加命令分支**

```rust
ReplCommand::Strategy(Some(k)) => {
    agent.set_strategy(k);
    println!("已切换到 {k:?} 策略");
}
ReplCommand::Strategy(None) => {
    println!("当前策略: {:?}", agent.strategy());
}
ReplCommand::Compact => match agent.compact().await {
    Ok((before, after)) => println!("压缩完成: {before} 条 → {after} 条"),
    Err(e) => eprintln!("压缩失败: {e}"),
},
```

- [ ] **Step 5: main.rs 切换 FileMemory + 注册工具 + .gitignore**

```rust
// 替换 InMemoryMemory 那行：
let memory: Arc<dyn Memory> =
    Arc::new(FileMemory::load(Path::new(".agent_history.jsonl")).await?);

// 注册第四个工具（在 LoadSkillTool 之后）：
tool_registry.register(Arc::new(SearchMemoryTool::new(
    memory.clone(),
    Arc::new(KeywordRetriever),
)));

// Agent::new 的 memory 参数改为直接传 memory（它已经是 Arc 了）
```

`.gitignore` 追加一行：`.agent_history.jsonl`

缺的 import 自己补（`FileMemory`、`SearchMemoryTool`、`KeywordRetriever`）。

- [ ] **Step 6: `cargo test` 全绿、`cargo build` 通过 → 提交** — `feat: /strategy /compact 命令与 FileMemory 上线`

---

## Task 9: 端到端手动验证

- [ ] **Step 1: 持久化验证**

```bash
cargo run
> 我喜欢吃火锅，记住了吗
> exit
cargo run          # 重启
> 我喜欢吃什么？    # 观察模型是否调用 search_memory 并答出「火锅」
cat .agent_history.jsonl   # 检查 JSONL 内容
```

- [ ] **Step 2: 策略命令验证**

- `/strategy` → 显示 `Window`；`/strategy summarize` → 切换成功；`/strategy x` → 错误提示不崩溃
- `/compact` → 打印 `压缩完成: N 条 → M 条`（对话短时 N==M 或提示没的可压，属正常——compact_memory 对 `len <= keep_recent` 直接返回）

- [ ] **Step 3: 回归验证**

- 三种模式（react/reflect/plan）各问一轮，行为与二期一致
- `现在几点` / `算 (3+4)*5` 工具调用正常

- [ ] **Step 4: 修掉验证中发现的问题 → 提交** — `feat: 第三期端到端验证`

---

## 完成定义（Definition of Done）

- `cargo test` 全绿（二期 47 个 + 本期新增约 15 个）
- 持久化、检索、策略切换、手动压缩全部手动验证通过
- 每任务一次提交，历史清晰
