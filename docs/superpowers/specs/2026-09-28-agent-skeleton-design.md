# Agent 项目第一期：基础 Agent 骨架 — 设计文档

日期：2026-09-28
状态：已确认

## 背景与目标

这是一个以学习 Rust 为目的的 Agent 项目，基于 `async-openai` crate 实现，LLM 后端为 DeepSeek（通过 `OPENAI_API_KEY` / `OPENAI_BASE_URL` 环境变量）。

最终目标是包含记忆、工具、MCP、ReAct、Skills 的完整智能体。该目标拆分为 5 期，本设计只覆盖第 1 期。后续每期在前一期之上叠加，各自走独立的设计 → 计划 → 实现循环。

## 总体路线图（仅规划，本设计不展开）

1. **第 1 期（本期）**：最小骨架 — REPL 对话循环 + 消息历史 + 一个内置工具（如 get_time）
2. **第 2 期**：ReAct 循环 + 工具系统（Tool trait、注册表、function calling）
3. **第 3 期**：记忆系统（短期：截断/摘要；长期：可选）
4. **第 4 期**：MCP 客户端
5. **第 5 期**：Skills

## 本期需求

- 程序启动后进入 REPL：循环读取用户输入，发送给 LLM，打印回复
- 输入 `exit`（或 EOF）退出
- 多轮对话：对话历史保存在内存中，每轮请求携带完整历史
- 在 `agent.rs` 中留一个 `get_time` 工具定义的常量/函数占位（本期最简形态：只描述工具，不发送 `tools` 字段、不做 function calling——那属于第 2 期）
- 请求使用强类型 `CreateChatCompletionRequest`，不再使用 `json!` 宏

## 非目标（本期明确不做）

- function calling / 工具自动调用
- 流式输出（stream）
- 持久化存储（历史仅存内存，退出即失）
- 任何 trait 抽象（如 `LlmClient` / `Memory` trait）——第 2 期再引入
- 配置文件（`.env` / toml）——密钥与 base_url 用环境变量，模型名用代码常量

## 架构

方案 A：简单模块化。纯 struct + 方法，无 trait、无泛型、无动态分发。

### 文件结构

```
src/
├── main.rs      # 入口：tokio main，REPL 循环，读 stdin，输入 exit 退出
├── agent.rs     # Agent 结构体：持有 Client + Vec<Message> 历史
└── config.rs    # 常量：模型名 MODEL 等
```

### 组件说明

**config.rs**
- `pub const MODEL: &str = "deepseek-chat";`
- 放置其他模型相关常量（如 max_tokens），本期保持最小。

**agent.rs**
- `pub struct Agent` 字段：
  - `client: Client<OpenAIConfig>`（async-openai 客户端）
  - `history: Vec<ChatCompletionRequestMessage>`（短期记忆，内存中）
- `impl Agent`：
  - `pub fn new() -> Self`：创建 client（自动读取环境变量），空 history；可注入 system prompt
  - `pub async fn ask(&mut self, input: &str) -> Result<String, OpenAIError>`：
    1. 把用户消息 push 进 history
    2. 用 `CreateChatCompletionRequestArgs` 构建强类型请求（model 取自 `config::MODEL`）
    3. `client.chat().create(request).await?`
    4. 从 `choices[0].message.content` 取出回复（Option 处理）
    5. 把助手消息 push 进 history，返回回复字符串
- 工具占位：`fn tool_definitions()` 或常量的 `get_time` 描述，本期不参与请求。

**main.rs**
- `#[tokio::main]`，创建 Agent
- 循环：`println!` 提示符 → `stdin().read_line()` 读取 → trim
  - `"exit"` 或 EOF（`read_line` 返回 0）→ break
  - 空输入 → continue
  - 其余 → `agent.ask(&input).await`，打印回复或错误
- 错误打印到 stderr，不中断循环（单轮失败不应退出程序）。

### 数据流

```
用户输入 (stdin)
  → main.rs REPL 循环
  → Agent.ask(input)
      → history.push(user msg)
      → CreateChatCompletionRequest { model, messages: history }
      → DeepSeek API
      → 提取 choices[0].message.content
      → history.push(assistant msg)
  → 打印回复
```

## 错误处理

- `ask` 返回 `Result<String, OpenAIError>`，main 中 `match`：Err 时 `eprintln!` 错误并继续循环
- 退出条件：用户输入 `exit` 或 Ctrl-D（EOF）

## 依赖

- `async-openai = { version = "0.42", features = ["chat-completion"] }`（不再需要 `byot`，改用强类型 API）
- `tokio = { version = "1", features = ["macros", "rt-multi-thread"] }`
- `serde_json`（本期可能不再需要，视实现而定）

## 测试策略

本期以手动验证为主（REPL 交互），不强制单元测试。可选：`ask` 中对 history 的 push 逻辑写一个纯数据结构的单元测试（不发起网络请求）。

## 教学映射（本期接触的 Rust 概念）

- 结构体与所有权：`Agent` 持有 `history`，`ask` 需要 `&mut self`
- 异步：async/await、tokio runtime
- 错误处理：`Result` / `?` / `match`
- 标准 I/O：`stdin` / `stdout`
- 借用与克隆：请求构建时从 history 借消息
