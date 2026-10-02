# Agent 项目第二期：经典 Agent 范式（Reflection + Plan-and-Solve）— 设计文档

日期：2026-10-02
状态：已确认
参考：datawhalechina/hello-agents 第 4 章「智能体经典范式构建」

## 背景与目标

第一期已完成：REPL + trait 抽象（`Tool` / `Memory` / `LlmClient`）+ function calling ReAct 循环 + 流式输出 + Claude Code 式 Skills。

第二期目标：在现有架构上新增两个经典 Agent 范式——**Reflection**（执行→反思→优化）与 **Plan-and-Solve**（规划→逐步执行），与现有 ReAct 并列，通过 REPL 斜杠命令切换，中间过程可用 `/verbose` 控制可见性。

## 本期需求

- `Mode` 枚举三范式：`ReAct`（现状）、`Reflect`、`PlanSolve`，`Agent::ask` 按模式分发
- REPL 斜杠命令：`/mode [react|reflect|plan]`（无参数查询当前模式）、`/verbose [on|off]`（无参数切换）、`/help`
- Reflection：初始执行（完整 ReAct，可用工具）→ 评审 → 优化，循环至评审回复「无需改进」或达到 `max_reflections`（默认 3）
- Plan-and-Solve：规划（编号步骤列表）→ 逐步执行（每步走完整 ReAct，可用工具）→ 最后一步输出即最终答案；计划解析失败时降级为 ReAct
- verbose=off 时只输出最终答案；verbose=on 时中间过程带前缀打印（`[反思]` / `[计划]`）

## 非目标（本期明确不做）

- Plan-and-Solve 的重规划（replanning）与步骤失败重试
- Reflection 的轨迹记忆结构（hello-agents 的 `Memory.records`）：`draft`/`critique` 局部变量直接拼提示词，语义等价
- Plan 的额外 synthesis 步骤：遵循 hello-agents，最后一步输出即答案
- 策略 trait 抽象（`trait Paradigm`）：范式固定 3 个，枚举 + match 足够（YAGNI）
- 历史截断/摘要、长期记忆、MCP（沿用第一期 spec 路线图，属后续期）

## 架构

### 关键约定：共享 memory 只存「干净记录」

- **共享 memory**（`Agent` 持有）：只追加 `user 消息` + `最终 assistant 答复`。Reflection 的多轮自我批评、Plan 的分步执行对用户是不可见的黑盒内部过程，不进共享历史——长对话不会被中间过程撑爆
- **草稿 memory**（scratch）：Reflect/Plan 模式内部各建临时 `InMemoryMemory`（播种 system prompt + 用户输入/执行上下文），中间 ReAct 工具调用、批评、改写全写进草稿，用完即弃

### 文件结构

```
src/
├── agent.rs          # 模块根：Agent 结构体、Mode 枚举、ask() 分发、chat_once() 辅助
├── agent/
│   ├── react.rs      # ReAct 循环（从现 agent.rs 搬移，参数化 memory 与 quiet）
│   ├── reflect.rs    # Reflection 范式
│   └── plan.rs       # Plan-and-Solve 范式（含 parse_plan 纯函数）
├── repl.rs           # REPL 循环 + 斜杠命令解析（从 main.rs 抽出）
├── main.rs           # 只剩组装：扫技能 → 建组件 → repl::run
└── （config / llm / memory / skill / tools 不动）
```

（采用 Rust 2018+ 无 mod.rs 模块风格：`agent.rs` 作模块根 + `agent/` 存子模块，全仓统一。）

### 核心类型（agent.rs，模块根）

```rust
pub enum Mode { ReAct, Reflect, PlanSolve }

pub struct Agent {
    llm: Arc<dyn LlmClient>,
    memory: Arc<dyn Memory>,
    tools: ToolRegistry,
    max_iterations: usize,    // ReAct 循环上限（现状，默认 8）
    max_reflections: usize,   // Reflection 轮数上限（默认 3）
    mode: Mode,
    verbose: bool,
}

impl Agent {
    pub async fn ask(&self, input: &str) -> Result<String, LlmError>; // match self.mode 分发
    pub fn set_mode(&mut self, mode: Mode);
    pub fn mode(&self) -> Mode;
    pub fn set_verbose(&mut self, on: bool);
    pub fn verbose(&self) -> bool;
}
```

REPL 持有 `mut agent: Agent`，斜杠命令通过 `&mut` setter 改状态；`ask` 保持 `&self`。

### 共享辅助函数

```rust
// 一次性静默调用：消费流收集为字符串；verbose 时逐 chunk 带前缀打印
async fn chat_once(&self, messages: Vec<ChatCompletionRequestMessage>, prefix: &str)
    -> Result<String, LlmError>

// ReAct 循环重构（三模式复用）：
// quiet=true 时不打印正文流（Plan 中间步骤在非 verbose 下静默）
async fn react_loop(&self, memory: &Arc<dyn Memory>, quiet: bool) -> Result<String, LlmError>
```

## Reflection 流程（reflect.rs）

```
scratch = 新 InMemoryMemory（播种 system prompt + user 输入）
draft = react_loop(scratch, quiet=!verbose)              # 初始执行，可用工具
loop 最多 max_reflections(3) 次:
    critique = chat_once(CRITIC_PROMPT + draft, "[反思] ")
    if critique 含 "无需改进" → break                     # 停止条件①
    draft = chat_once(REFINE_PROMPT(draft, critique), "[反思] ")
# 停止条件②：轮数耗尽
if !verbose → println!(draft)    # 静默模式下中间过程都没打印，最后一次性输出最终稿
共享 memory.add(user 消息 + assistant(最终 draft))
```

打印约定：verbose=on 时 draft/批评/优化稿在产生过程中已带前缀逐 chunk 打印，结尾不重复打印；verbose=off 时全程静默，循环结束后一次性 `println!` 最终稿。

提示词（改写自 hello-agents 第 4 章，去掉其 Python 代码特化，适配通用对话助手）：

- **CRITIC_PROMPT**：`你是一位极其严格的评审专家。检查以下回答的事实错误、逻辑漏洞、遗漏信息。如果回答已经足够好，只回复"无需改进"四个字，否则给出具体改进意见。`
- **REFINE_PROMPT**：`你正在根据评审专家的反馈优化你的回答。原回答：{draft}。评审意见：{critique}。请直接输出优化后的完整回答，不要输出解释。`

## Plan-and-Solve 流程（plan.rs）

```
plan_text = chat_once(PLANNER_PROMPT + input, "[计划] ")   # 规划，静默
steps = parse_plan(&plan_text)                             # 纯函数
if steps 为空 → 打印 "[计划] 无法解析，降级为直接回答" → 走 ReAct → 返回
scratch = 新 InMemoryMemory（播种 system prompt）
history = ""
for (i, step) in steps.enumerate():
    context = EXECUTOR_PROMPT(原始问题, 完整计划, history, 当前步骤)
    quiet = !verbose 且非最后一步
    result = react_loop(scratch 上追加 context 后, quiet)  # 每步可用工具
    history += result
# 最后一步输出即最终答案
共享 memory.add(user 消息 + assistant(最终答案))
```

提示词：

- **PLANNER_PROMPT**：`你是一个顶级的规划专家。把用户问题分解为有序步骤，每步一行，格式为"1. xxx"。只输出步骤列表，不要输出其他内容。`
- **EXECUTOR_PROMPT** 四要素：原始问题、完整计划、历史步骤结果、当前步骤；末尾要求`严格按照计划执行当前步骤，只输出该步骤的答案`

与 hello-agents 的差异：它用 ```` ```python ```` 列表 + `ast.literal_eval` 解析计划；Rust 无对应物，改为**编号行解析**（更宽容，模型不易跑偏）：

```rust
fn parse_plan(text: &str) -> Vec<String>  // 提取 "1. xxx" / "2、xxx" 等编号行，跳过杂行
```

## 斜杠命令层（repl.rs）

```rust
enum ReplCommand {
    Mode(Option<Mode>),   // None = 查询当前模式
    Verbose(bool),        // /verbose 无参数时由 REPL 取反后构造
    Help,
}

// None = 非命令（普通对话）；Some(Err) = 未知命令/参数错误（不发往 LLM）
fn parse_command(input: &str) -> Option<Result<ReplCommand, String>>
```

命令集：`/mode [react|reflect|plan]`、`/verbose [on|off]`、`/help`。普通 `exit`/EOF 退出行为不变。

## 数据流

```
用户输入 → REPL: parse_command
  ├ 是命令 → 改 Agent 状态 / 打印帮助，不进入 LLM
  └ 普通输入 → agent.ask(input) → match mode:
       ├ ReAct    → react_loop(共享 memory)          # 现状
       ├ Reflect  → react_loop(scratch) → 反思循环 → 共享 memory 只存 user+最终稿
       └ PlanSolve → chat_once(规划) → 逐步 react_loop(scratch) → 共享 memory 只存 user+最终答案
```

## 错误处理

- `chat_once` / `react_loop` 失败：直接 `?` 传播，REPL `eprintln!` 后继续（现状）
- 计划解析为空：打印降级提示，fallback 到 ReAct（不视为错误）
- 不新增 `LlmError` 变体

## 依赖

无新增依赖。

## 测试策略（均不联网，复用第一期 MockLlm 脚本队列）

- `parse_plan`：标准编号行、混入杂行/空行、空文本、全角编号
- `parse_command`：各命令正常/缺参/参数错误/未知命令/非命令输入
- Reflection（Mock 脚本序列）：
  - draft → 评审回复「无需改进」→ 一轮收敛，返回 draft
  - draft → 批评 → 优化稿 → 收敛 → 返回优化稿
  - 评审永远不收敛 → 跑满 max_reflections 后返回最后一稿
- Plan-and-Solve：
  - 两步计划 → 逐步执行 → 返回最后一步结果；共享 memory 仅 user+assistant 两条
  - 计划为空 → 降级 ReAct
- 模式分发：`set_mode` 后 `ask` 消费的 Mock 脚本数量与顺序符合对应范式
- 手动验证：`/mode`、`/verbose` 切换后行为正确，`/help` 输出完整

## 教学映射（本期接触的 Rust 概念）

- 枚举状态机 + 穷举 match、模式分发
- 模块拆分重构（agent.rs → agent/ 目录、REPL 抽出 main.rs）
- `&mut self` 与所有权（REPL 持有 agent）
- 提取参数重构（react_loop 参数化 memory/quiet）
- 字符串解析（编号行、命令参数）
- 循环控制（break 提前收敛、迭代上限）
- 多脚本 Mock 的异步测试编排

## 与 hello-agents 第 4 章的对应与偏离

| 项 | hello-agents | 本设计 | 理由 |
|---|---|---|---|
| Reflection 停止条件 | 含「无需改进」/ 达上限 | 相同 | — |
| Reflection 轨迹 | `Memory` 类存完整 records | 局部变量拼提示词 | 语义等价，YAGNI |
| Plan 解析 | ```python 列表 + ast.literal_eval | 编号行纯函数解析 | Rust 无 literal_eval，更宽容 |
| Plan 最终答案 | 最后一步输出 | 相同 | — |
| 执行器可用工具 | 纯 LLM 文本 | 每步走完整 ReAct（可用工具） | 复用第一期成果，实用性更强 |
