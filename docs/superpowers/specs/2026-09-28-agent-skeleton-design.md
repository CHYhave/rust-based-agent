# Agent 项目第一期：Agent 骨架 + 工具调用 + ReAct 循环 + 流式输出 + Skills — 设计文档

日期：2026-09-28
状态：已确认（v3）

## 背景与目标

以学习 Rust 为目的的 Agent 项目，基于 `async-openai` crate 实现，LLM 后端为 DeepSeek（通过 `OPENAI_API_KEY` / `OPENAI_BASE_URL` 环境变量）。

最终目标是包含记忆、工具、MCP、ReAct、Skills 的完整智能体。本设计覆盖第 1 期（范围：骨架 + trait 抽象 + 工具调用 ReAct + 流式输出 + Claude Code 式 Skills）。

## 总体路线图（仅规划，本设计不展开）

1. **第 1 期（本期）**：REPL + Memory/Tool/LlmClient trait + function calling ReAct 循环 + 流式输出 + Skills
2. **第 2 期**：记忆增强（历史截断/摘要策略）、更多内置工具、循环细节打磨
3. **第 3 期**：长期记忆（持久化/检索）
4. **第 4 期**：MCP 客户端（远端工具接入 Registry）

## 本期需求

- REPL：循环读用户输入，`exit` 或 EOF 退出，单轮失败打印错误后继续
- 多轮对话：历史保存在内存中，每轮请求携带完整历史
- **trait 全面抽象**：`Tool` / `Memory` / `LlmClient` 三个 trait，Agent 通过 `Arc<dyn Trait>` 组合依赖
- **function calling ReAct 循环**：模型返回 `tool_calls` 时执行工具、把结果以 `role=tool` 消息回传，循环直到模型不再请求工具；最大迭代次数保护
- **流式输出**：助手回复内容边接收边打印；流式响应中的 tool_calls 增量也要正确累加（不能因为有工具就退回非流式）
- **Claude Code 式 Skills**：
  - 项目根目录 `skills/` 下每个技能一个目录，含 `SKILL.md`（YAML frontmatter：`name`、`description`；正文：技能指令）
  - 启动时扫描并解析所有技能，把「名称 + 描述」列表注入 system prompt
  - 模型自主决定使用技能时，通过内置工具 `load_skill(name)` 获取完整指令内容
- 内置工具：`get_time`、`calculator`、`load_skill`

## 非目标（本期明确不做）

- 历史持久化（退出即失）、截断/摘要策略
- REPL 中用户手动触发技能（`/skill_name`）——本期技能完全由模型自主触发
- MCP
- 配置文件（`.env` / toml）：密钥与 base_url 用环境变量，模型名用代码常量
- 子技能/技能内资源文件（progressive disclosure 的进阶形态）

## 架构

核心结构：`Agent` 不依赖任何具体类型，只持有 `Arc<dyn Trait>`。trait 定义在本 crate 内，async-openai 只出现在 `llm/client.rs` 的 OpenAI 实现中。

### 文件结构

```
src/
├── main.rs            # 入口：tokio main，REPL 循环，组装各组件
├── config.rs          # 常量：MODEL 等
├── agent.rs           # Agent 结构体 + ReAct 循环 + system prompt 构建
├── llm/
│   ├── mod.rs         # ChatStreamEvent 等公开类型
│   └── client.rs      # LlmClient trait + OpenAiClient 实现（唯一依赖 async-openai 的地方）
├── memory/
│   ├── mod.rs
│   └── memory.rs      # Memory trait + InMemoryMemory（Mutex<Vec<Message>>）
├── tools/
│   ├── mod.rs
│   ├── registry.rs    # ToolRegistry：name → Arc<dyn Tool>
│   ├── get_time.rs
│   ├── calculator.rs  # 自写递归下降解析器（+ - * / 括号 一元负号）
│   └── load_skill.rs  # 内置工具：加载 SKILL.md 正文
└── skill/
    ├── mod.rs
    ├── discovery.rs   # 扫描 skills/ 目录、解析 frontmatter
    └── accumulator.rs # 流式 tool_calls 增量累加器（纯逻辑，可单测）

skills/                # 技能数据目录（项目根，非 src 下）
└── <skill-name>/
    └── SKILL.md
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
    ) -> Result<Pin<Box<dyn Stream<Item = Result<ChatStreamEvent, LlmError>> + Send>>, LlmError>;
}
```

说明：
- 消息/工具类型复用 `async_openai::types::chat` 强类型；流事件用**自定义** `ChatStreamEvent`，SDK 流类型不出现在 trait 上（Mock 实现友好）
- `ChatStreamEvent`：
  ```rust
  pub enum ChatStreamEvent {
      Content(String),                       // 正文增量
      ToolCallDelta { index: usize, id: Option<String>, name: Option<String>, args: String },
      Done,
  }
  ```
- 异步 trait 需要 `async-trait` crate；`Send + Sync` 保证可跨 await 持有

### 流式 tool_calls 累加

流式响应中一个 tool_call 的 id/name 出现在首个 delta、arguments 是逐段 JSON 字符串。`ToolCallAccumulator`（`HashMap<usize, 累加状态>`）负责把增量还原成完整的 tool_calls 列表——这是本期最绕的逻辑，单独成文件 + 单测覆盖。

### 组件说明

**config.rs**
- `pub const MODEL: &str = "deepseek-chat";`

**memory/memory.rs**
- `InMemoryMemory`：`Mutex<Vec<ChatCompletionRequestMessage>>`，trait 方法用 `&self`（内部可变性）

**tools/registry.rs**
- `ToolRegistry`：`HashMap<String, Arc<dyn Tool>>`；`register` / `get` / `schemas`（组装 SDK tool schema 列表）

**tools/get_time.rs**
- 无参数；返回当前本地时间字符串（`chrono`）

**tools/calculator.rs**
- 参数 `expression: string`；自写递归下降解析器（支持 `+ - * /`、括号、一元负号，约百行）；`Err(String)` 回传模型

**tools/load_skill.rs**
- 持有 `Arc<HashMap<String, String>>`（技能名 → SKILL.md 正文，由 discovery 产出）
- 参数 `name: string`；找到返回正文，未找到返回错误文本（作为工具结果让模型纠正）

**skill/discovery.rs**
- 扫描 `skills/*/SKILL.md`，手写一个极简 frontmatter 解析器（只取 `---` 之间的 `name:` / `description:` 行，不引 YAML 库——避免依赖膨胀，也够本期用）
- 产出：`Vec<SkillMeta { name, description }>` + `HashMap<String, String>`（name → 正文）
- 目录不存在时返回空（不报错）

**agent.rs**
- `pub struct Agent`：`llm: Arc<dyn LlmClient>`、`memory: Arc<dyn Memory>`、`tools: ToolRegistry`、`max_iterations: usize`（默认 8）、`system_prompt: String`
- `ask(&self, input)` 流程：
  1. `memory.add(user 消息)`
  2. ReAct 循环（最多 max_iterations）：
     - 打开流：`llm.chat(memory.messages(), tools.schemas())`
     - 消费流：`Content(delta)` → 立即打印 + 累加；`ToolCallDelta` → 喂给 `ToolCallAccumulator`；`Done` → 结束
     - 由累加结果构建 assistant 消息（正文 + tool_calls），`memory.add`
     - 无 tool_calls → 返回正文
     - 有 tool_calls → 逐个执行：registry 查找，找到执行 `call`，`Ok/Err` 均序列化为 `role=tool` 消息（带 `tool_call_id`）；未找到 → 错误文本回传 → 继续循环
  3. 超过 max_iterations → `LlmError::MaxIterations`

**system prompt 构建**（`agent.rs` 或独立函数）
- 注入技能列表：`可用技能：\n- {name}: {description}\n当任务匹配某个技能时，先用 load_skill 工具加载其完整指令，然后严格遵循。`

**main.rs**
- 组装：discovery 扫描 skills → 创建 registry（注册 3 个工具）→ Agent → REPL
- 错误 `eprintln!` 后继续；`exit`/EOF 退出

### 数据流

```
用户输入 → memory.add(user)
  → ReAct 循环:
      llm.chat(...) → 流
        ├ Content(delta) ──→ stdout（即时打印）
        └ ToolCallDelta ──→ Accumulator
      Done → 构建 assistant 消息 → memory.add
        ├ 无 tool_calls → 返回
        └ 有 tool_calls → registry.call() → memory.add(role=tool) → 继续循环
```

## 错误处理

- 工具执行 `Result<String, String>`（Err 回传模型自我纠正）
- `LlmError`：包装 SDK 错误 + `MaxIterations` 变体
- main 捕获打印，REPL 继续

## 依赖

- `async-openai = { version = "0.42", features = ["chat-completion"] }`（不再需要 `byot`）
- `async-trait = "0.1"`
- `tokio = { version = "1", features = ["macros", "rt-multi-thread", "sync"] }`
- `serde_json = "1"`
- `futures = "0.3"`（消费流）
- `chrono = "0.4"`（get_time）

## 测试策略（均不联网）

- calculator：正常表达式、优先级、括号、非法输入、除零
- registry：注册/查找/未知工具
- memory：add/messages/clear
- skill discovery：临时目录构造 SKILL.md，验证 frontmatter 解析与缺目录容错
- ToolCallAccumulator：跨 chunk 的 id/name/args 拼合、多 tool_call 交错
- agent ReAct 循环：MockLlmClient 用 `tokio_stream` 返回固定事件序列，验证「工具调用→回传→再请求」完整路径及 max_iterations 触发

## 教学映射（本期接触的 Rust 概念）

- trait 与 `async-trait`、`Arc<dyn Trait>` 依赖注入
- `Mutex` 与内部可变性
- 流（Stream）与增量数据处理、状态累加器
- 所有权/借用、`Pin<Box<dyn Stream>>`
- 自定义错误枚举、`?` 传播
- 文件系统遍历（fs::read_dir）、简单文本解析
- 递归下降解析器（经典练习）
- 单元测试、mock、tokio 异步测试
