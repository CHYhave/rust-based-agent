# Agent 项目第三期：长期记忆 + 上下文工程 — 设计文档

日期：2026-10-03
状态：已确认
参考：datawhalechina/hello-agents 第 8 章（记忆与检索）、第 9 章（上下文工程）

## 背景与目标

第一期：ReAct + 流式 + Skills；第二期：Reflection + Plan-and-Solve 范式。第三期补齐记忆与上下文能力，解决两个真实痛点：① 对话退出即失，无法跨会话记忆；② 长对话撑爆上下文窗口。

## 本期需求

1. **JSONL 持久化**：`FileMemory` 实现 `Memory` trait，每条消息追加一行 JSON（SDK 消息类型原生支持 serde），启动时加载历史
2. **关键词检索**：`Retriever` trait + `KeywordRetriever`（ASCII 词 + CJK 二元组打分），通过 `search_memory` 工具暴露给模型自主调用
3. **上下文策略**：`ContextStrategy` trait + 两实现——`WindowStrategy`（滑动窗口，视图变换）与 `SummarizeStrategy`（超阈值就地压缩）
4. **REPL 新命令**：`/strategy [window|summarize]` 切换策略、`/compact` 手动压缩

## 非目标（本期明确不做）

- Embedding 向量检索（Retriever trait 预留插拔点）
- Token 精确计数（不引 tiktoken，用消息数/字符数作代理）
- 多会话管理 / 会话切换（单一历史文件）
- 压缩消息的归档保留（压缩是有损的，被压掉的消息即丢失）

## 架构

### 文件结构

```
src/
├── memory/
│   ├── memory.rs        # Memory trait（+replace 默认实现）+ InMemoryMemory + 消息构造辅助（不动）
│   └── file.rs          # FileMemory：JSONL 持久化
├── context.rs           # ContextStrategy trait + WindowStrategy + SummarizeStrategy
├── retrieval.rs         # Retriever trait + KeywordRetriever + render_message + tokenize
├── tools/
│   └── search_memory.rs # search_memory 工具
├── agent.rs             # +context 字段、+strategy 切换、+compact()
└── repl.rs              # +/strategy /compact 命令
```

（沿用无 mod.rs 风格；模块小所以 context/retrieval 是单文件。）

### 数据流

```
启动：FileMemory::load(path) → 读 JSONL → 内存 Vec（无文件 = 空历史，不报错）

每轮：ask(input) → memory.add(user)
  → react_loop 每轮请求前：msgs = context.apply(&memory).await?
      ├ Window    → [system] + 最近 N 条（纯视图，memory 不动）
      └ Summarize → 超阈值：LLM 摘要旧段 → memory.replace(就地压缩) → 返回全量
  → llm.chat(msgs, tools)

模型自主：search_memory(query) 工具 → memory.messages() → render 文本 → Retriever 打分 → top-3 回传

/compact → Agent::compact()：无门槛立即压缩（不受当前策略限制）
```

### 关键设计：Window 是视图，Summarize 是就地压缩

两者对 memory 的影响不同，但统一在同一个 trait 接口下：

```rust
#[async_trait]
pub trait ContextStrategy: Send + Sync {
    /// 返回本次请求实际发送的消息列表
    async fn apply(
        &self,
        memory: &Arc<dyn Memory>,
    ) -> Result<Vec<ChatCompletionRequestMessage>, LlmError>;
}
```

- `WindowStrategy { max_messages: usize }`（默认 30）：`[首条 system（若首条是 system 才保留）] + 末尾 N 条`。**不改 memory**——完整历史留在存储里，`search_memory` 仍能搜到被窗口截掉的内容
- `SummarizeStrategy { llm, threshold_chars: usize, keep_recent: usize }`（默认 8000 字符 / 保留近 10 条）：超阈值时把 `[1 .. len-keep_recent]` 的旧消息发给 LLM 压缩成一条摘要消息，`memory.replace()` 就地重写。摘要是有损操作，这符合其语义

`Memory` trait 新增（默认实现，FileMemory 靠 clear+add 自动正确）：

```rust
async fn replace(&self, messages: Vec<ChatCompletionRequestMessage>) {
    self.clear().await;
    for m in messages { self.add(m).await; }
}
```

`Agent::compact()` 供 `/compact` 调用：不看阈值，直接把共享 memory 压到 `keep_recent` 条（复用 SummarizeStrategy 的压缩逻辑——抽成该策略的 `pub(crate) async fn compact_memory(&self, memory)`，`apply` 超阈值时也调它）。

## 组件契约

### memory/file.rs

```rust
pub struct FileMemory { /* Mutex<Vec<Message>> + PathBuf */ }

impl FileMemory {
    /// 加载 JSONL；文件不存在 → 空历史；坏行跳过并 eprintln 警告
    pub async fn load(path: PathBuf) -> std::io::Result<Self>
}

// impl Memory for FileMemory:
//   add  → 压 Vec + tokio::fs 追加一行 JSON
//   clear → 清 Vec + 截断文件
//   messages/replace → 默认/同 InMemory
```

已核实：`ChatCompletionRequestMessage` derive 了 `Serialize + Deserialize`（`#[serde(tag = "role")]`），可直接 `serde_json::to_string` / `from_str` 逐行读写。新依赖：`tokio` 的 `fs` feature。

历史文件位置：项目根目录 `.agent_history.jsonl`（cwd 相对路径，与 `skills/` 同款约定），并加入 `.gitignore`。

### context.rs

```rust
pub struct WindowStrategy { pub max_messages: usize }      // 默认 30
pub struct SummarizeStrategy {
    llm: Arc<dyn LlmClient>,
    pub threshold_chars: usize,                            // 默认 8000
    pub keep_recent: usize,                                // 默认 10
}
```

摘要调用复用 `chat_once` 逻辑——**先做一步重构**：把 `Agent::chat_once` 的「收集流为 String」核心抽成 `llm` 模块的自由函数：

```rust
// llm.rs
pub async fn collect_chat(
    llm: &Arc<dyn LlmClient>,
    messages: Vec<ChatCompletionRequestMessage>,
) -> Result<String, LlmError>
```

`Agent::chat_once`（带 verbose 打印）与 `SummarizeStrategy`（纯收集）都基于它。

摘要提示词：`你是对话摘要专家。把以下对话历史压缩为一段摘要，保留关键事实、用户偏好和待办事项。直接输出摘要。`

### retrieval.rs

```rust
pub struct SearchHit { pub index: usize, pub score: f64 }

pub trait Retriever: Send + Sync {
    /// 按相关度降序返回 top_k 个命中（score > 0）
    fn search(&self, query: &str, docs: &[String], top_k: usize) -> Vec<SearchHit>;
}

pub struct KeywordRetriever;   // 打分 = 查询项在文档中的出现次数（TF）
```

分词器（中文无空格，混合策略）：

```rust
/// ASCII 字母数字序列转小写为词；CJK 字符取二元组（bigram）
/// 例："查 Rust 书籍" → ["rust", "查书", "书籍"] …（"查"+"Rust" 跨界不组 bigram）
pub(crate) fn tokenize(text: &str) -> Vec<String>
```

消息渲染（tool_calls 等无正文消息返回 None）：

```rust
pub(crate) fn render_message(msg: &ChatCompletionRequestMessage) -> Option<String>
// 输出形如 "user: 帮我算 1+2" / "tool: 3"
```

### tools/search_memory.rs

```rust
pub struct SearchMemoryTool {
    memory: Arc<dyn Memory>,        // 持有 FileMemory，检索范围 = 全部历史（含过去会话）
    retriever: Arc<dyn Retriever>,
}
// 参数 {"query": "..."} → top-3 结果按 "score 降序 + 原文" 拼成文本回传模型；无命中返回 "没有找到相关记忆"
```

### agent.rs / repl.rs 变更

```rust
pub struct Agent {
    // …现有字段
    context: Arc<dyn ContextStrategy>,
    window: Arc<WindowStrategy>,          // /strategy 切换用
    summarize: Arc<SummarizeStrategy>,
}
pub fn set_strategy(&mut self, kind: ContextKind)   // enum ContextKind { Window, Summarize }
pub async fn compact(&self) -> Result<(), LlmError>  // 供 /compact
```

- `react_loop` 每轮请求前 `context.apply(memory).await?`（reflect/plan 的草稿 memory 也走同一策略——草稿通常很短，apply 是无害的恒等操作）
- `chat_once` 不套策略（它的输入是构造的提示词，不是对话历史）
- REPL 新增：`/strategy [window|summarize]`（无参查询）、`/compact`（压缩后打印压缩前后消息数）

## 错误处理

- JSONL 读写：`std::io::Error` 向上传播到 main（启动失败直接退出）——坏行不致命，跳过
- 摘要 LLM 调用失败：`LlmError` 传播，REPL 报错继续
- `search_memory` 无命中 / 参数缺 query：错误文本回传模型（与现有工具一致）

## 依赖

- `tokio` 增加 `fs` feature（其余不动）

## 测试策略（均不联网）

- `FileMemory`：`std::env::temp_dir()` 临时文件——save→load 往返一致、坏行跳过、clear 截断文件、无文件不报错
- `WindowStrategy`：窗口内不动 / 超窗保留 system + 末尾 N 条
- `SummarizeStrategy`：MockLlm——未超阈值原样返回；超阈值 → memory 被重写为 system+摘要+近期、返回压缩后列表
- `tokenize`：纯英文、纯中文、中英混合、标点过滤
- `KeywordRetriever`：相关度排序、top_k 截断、零分过滤
- `search_memory`：命中返回原文、无命中返回提示文本
- `parse_command`：`/strategy` 各参数、`/compact`
- Agent 集成：MockLlm 增加**请求记录**（`received: Mutex<Vec<Vec<Message>>>`），断言 Window 策略下发送的消息数被截断

## 教学映射（本期接触的 Rust 概念）

- serde 序列化/反序列化（JSONL 逐行、tagged enum）
- tokio 异步文件 IO（OpenOptions / append / truncate）
- trait 策略模式 ×2（ContextStrategy、Retriever）
- trait 默认实现（Memory::replace）
- 文本处理：字符/字节边界、bigram 分词
- 可测试性设计（Mock 记录请求、纯函数分词器）
