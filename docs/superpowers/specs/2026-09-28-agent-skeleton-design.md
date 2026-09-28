# Agent 项目第一期：Agent 骨架 + 工具调用 + ReAct 循环 — 设计文档

日期：2026-09-28
状态：已确认（v2，含工具调用与 trait 抽象）

## 背景与目标

这是一个以学习 Rust 为目的的 Agent 项目，基于 `async-openai` crate 实现，LLM 后端为 DeepSeek（通过 `OPENAI_API_KEY` / `OPENAI_BASE_URL` 环境变量）。

最终目标是包含记忆、工具、MCP、ReAct、Skills 的完整智能体。拆分为 5 期，本设计覆盖第 1 期（已按用户要求升级为含工具调用与 trait 抽象）。

## 总体路线图（仅规划，本设计不展开）

1. **第 1 期（本期）**：REPL 对话循环 + Memory trait（内存实现）+ Tool trait + Registry + LlmClient trait（OpenAI 实现）+ function calling ReAct 循环
2. **第 2 期**：记忆增强（历史截断/摘要策略）、更多内置工具、循环细节打磨
3. **第 3 期**：长期记忆（持久化/检索）
4. **第 4 期**：MCP 客户端（远端工具接入 Registry）
5. **第 5 期**：Skills

## 本期需求

- REPL：循环读用户输入，`exit` 或 EOF 退出，单轮失败打印错误后继续
- 多轮对话：历史保存在内存中，每轮请求携带完整历史
- **function calling 工具调用**：请求携带 `tools` schema；模型返回 `tool_calls` 时执行工具、把结果以 `role=tool` 消息回传，循环直到模型不再请求工具
- **全面 trait 抽象**：`Tool` / `Memory` / `LlmClient` 三个 trait，Agent 通过 `Arc<dyn Trait>` 组合依赖
- 内置工具：`get_time`（无参数）、`calculator`（一个表达式字符串参数，简单四则运算）
- 循环保护：最大迭代次数，防止模型无限请求工具

## 非目标（本期明确不做）

- 流式输出（stream）
- 历史持久化（退出即失）、截断/摘要策略
- MCP、Skills
- 配置文件（`.env` / toml）：密钥与 base_url 用环境变量，模型名用代码常量

## 架构

核心结构：`Agent` 不依赖任何具体类型，只持有三个 `Arc<dyn Trait>`。trait 定义在本 crate 内，async-openai 只出现在 `llm/client.rs` 的 OpenAI 实现中。

### 文件结构

```
src/
├── main.rs            # 入口：tokio main，REPL 循环
├── config.rs          # 常量：MODEL 等
├── agent.rs           # Agent 结构体 + ReAct(function calling) 循环
├── llm/
│   ├── mod.rs
│   └── client.rs      # LlmClient trait + OpenAiClient 实现（唯一依赖 async-openai 类型的地方）
├── memory/
│   ├── mod.rs
│   └── memory.rs      # Memory trait + InMemoryMemory（Vec<Message>）
└── tools/
    ├── mod.rs
    ├── registry.rs    # ToolRegistry：name → Arc<dyn Tool>
    ├── get_time.rs
    └── calculator.rs
```

### trait 定义（均为 async-trait）

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> Value;      // JSON Schema，发给模型
    async fn call(&self, arguments: &str) -> Result<String, String>; // 参数为模型返回的 JSON 字符串
}

#[async_trait]
pub trait Memory: Send + Sync {
    async fn messages(&self) -> Vec<ChatCompletionRequestMessage>;
    async fn add(&self, msg: ChatCompletionRequestMessage);
    async fn clear(&self);
}

#[async_trait]
pub trait LlmClient: Send + Sync {
    async fn chat(
        &self,
        messages: Vec<ChatCompletionRequestMessage>,
        tools: Vec<ChatCompletionTool>,
    ) -> Result<ChatCompletionResponseMessage, LlmError>;
}
```

说明：
- 消息/工具类型直接复用 `async_openai::types::chat` 中的强类型（`ChatCompletionRequestMessage`、`ChatCompletionTool` 等），不自己造轮子；本期接受 memory/llm trait 与 SDK 类型耦合，第 3 期做长期记忆时再评估是否引入自有消息类型
- `LlmError` 为本 crate 定义的错误枚举，包装 `OpenAIError`，让 Agent 层不直接依赖 SDK 错误类型
- 异步 trait 方法需要 `async-trait` crate（`dyn` 兼容性）；`Send + Sync` 保证可跨 await 持有

### 组件说明

**config.rs**
- `pub const MODEL: &str = "deepseek-chat";`

**memory/memory.rs**
- `InMemoryMemory`：`Mutex<Vec<ChatCompletionRequestMessage>>`（需要内部可变性，因为 trait 方法用 `&self`），`clear` 清空

**tools/registry.rs**
- `ToolRegistry`：`HashMap<String, Arc<dyn Tool>>`
- `register(tool)`、`get(name) -> Option<Arc<dyn Tool>>`、`schemas() -> Vec<ChatCompletionTool>`（把每个 Tool 的 name/description/parameters 组装成 SDK 的 tool schema）

**tools/get_time.rs / calculator.rs**
- `get_time`：无参数，`parameters_schema` 为空 object（`{"type":"object","properties":{}}`），返回当前本地时间字符串
- `calculator`：一个 `expression: string` 参数；用自写的递归下降解析器求值（支持 `+ - * /`、括号、一元负号，约百行，顺便练习 Rust），失败返回 `Err(String)`，Err 内容会作为工具结果回传给模型

**agent.rs**
- `pub struct Agent`：`llm: Arc<dyn LlmClient>`、`memory: Arc<dyn Memory>`、`tools: ToolRegistry`、`max_iterations: usize`（默认 8）
- `pub async fn ask(&self, input: &str) -> Result<String, LlmError>` 执行 ReAct 循环：
  1. `memory.add(user 消息)`
  2. 循环（最多 max_iterations 次）：
     - `resp = llm.chat(memory.messages(), tools.schemas()).await?`
     - `memory.add(assistant 消息)`（保留 tool_calls 的原始消息）
     - 若 `resp.tool_calls` 为空 → 返回 `resp.content`
     - 否则对每个 tool_call：按 id 记录，registry 查找工具：
       - 找到 → `tool.call(arguments)`，`Ok/Err` 都序列化为 `role=tool` 消息（`tool_call_id` 对应），push 进 memory
       - 未找到 → 以 `Err` 文本作为工具结果回传，让模型自行纠正
  3. 超过 max_iterations → 返回 `LlmError::MaxIterations`

**main.rs**
- `#[tokio::main]`：组装 `OpenAiClient` / `InMemoryMemory` / Registry（注册两个内置工具），创建 Agent
- REPL 循环：提示符 → `read_line` → trim → `exit`/EOF 退出，空输入 continue，其余 `agent.ask()`，打印回复；错误 `eprintln!` 后继续

### 数据流

```
用户输入 (stdin)
  → main.rs REPL
  → Agent.ask(input)
      → memory.add(user)
      → ┌ ReAct 循环 ─────────────────────────────┐
        │ llm.chat(memory.messages(), tool schemas) │
        │ ├─ 无 tool_calls → 返回 content           │
        │ └─ 有 tool_calls → registry.call()        │
        │      → memory.add(role=tool) → 继续循环    │
        └───────────────────────────────────────────┘
  → 打印回复
```

## 错误处理

- 三层错误：工具执行 `Result<String, String>`（Err 回传模型）→ `LlmError`（Agent/LLM 层，包装 SDK 错误）→ main 捕获打印继续 REPL
- 防御点：未知工具名、工具参数解析失败、模型返回空 content、达到最大迭代次数

## 依赖

- `async-openai = { version = "0.42", features = ["chat-completion"] }`（不再需要 `byot`）
- `async-trait = "0.1"`
- `tokio = { version = "1", features = ["macros", "rt-multi-thread", "sync"] }`（`sync` 用于 Mutex）
- `serde_json = "1"`（工具 schema、参数解析）
- `chrono = "0.4"`（get_time；如不想加依赖可改用 `std::time`，实现时定）

## 测试策略

- 单元测试（不联网）：
  - calculator：正常表达式、非法表达式、除零
  - registry：注册/查找/未知工具名
  - memory：add/messages/clear
  - agent 循环：用 MockLlmClient（测试里实现 `LlmClient` trait 返回固定 tool_calls）验证 ReAct 循环走满"调用工具→回传→再请求"的完整路径——这也是 trait 抽象的直接收益
- 手动验证：REPL 多轮对话、触发计算器工具

## 教学映射（本期接触的 Rust 概念）

- trait 与 `async-trait`、`Arc<dyn Trait>` 依赖注入
- `Mutex` 与内部可变性
- 所有权/借用：`ask(&self)` 与共享状态
- 错误处理：自定义错误枚举、`thiserror`（可选）、`?` 传播
- 异步：多轮 await、tokio
- 泛型与集合：HashMap 注册表
- 单元测试与 mock
