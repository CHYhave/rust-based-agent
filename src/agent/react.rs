use std::sync::Arc;

use crate::agent::{Agent, flush_stdout};
use crate::skill::accumulator::ToolCallAccumulator;
use crate::memory::memory::{Memory, assistant_msg, tool_msg};
use crate::llm::{ ChatStreamEvent, LlmError};
use async_openai::types::chat::ChatCompletionMessageToolCalls;
use futures::StreamExt;

impl Agent {
    pub async fn react_loop(
        &self,
        memory: &Arc<dyn Memory>,
        quiet: bool,
    ) -> Result<String, LlmError> {
        let schema = self.tools.schemas();
        for _ in 0..self.max_iterations {
            let mut stream = self.llm.chat(memory.messages().await, schema.clone()).await?;
            let mut text = String::new();
            let mut acc: ToolCallAccumulator = ToolCallAccumulator::new();
            while let Some(event) = stream.next().await {
                let event = event?;
                match event {
                    ChatStreamEvent::Content(d) => {
                        if !quiet {
                            print!("{d}");
                            flush_stdout();
                        }
                        text.push_str(&d);
                    }
                    ChatStreamEvent::ToolCallDelta {index, id, name, args}  => {
                        acc.feed(index, id, name, Some(&args));
                    }
                    ChatStreamEvent::Done => break,
                }
            }
            let tool_calls = acc.into_tool_calls();
            if tool_calls.is_empty() {
                // 纯文本回复：assistant 消息入 memory，换行收尾，返回
                memory.add(assistant_msg(&text, None)).await;
                println!();
                return Ok(text);
            }

            // 有工具调用：先执行，收集结果消息
            // 注意 history 顺序必须是 assistant(带 tool_calls) 在前、tool 消息在后，
            // 所以等执行完再一起存
            let mut tool_msgs = Vec::new();
            for call in &tool_calls {
                let ChatCompletionMessageToolCalls::Function(fc) = call else { continue };
                let result = match self.tools.get(&fc.function.name) {
                    Some(tool) => tool.call(&fc.function.arguments).await,
                    None => Err(format!("工具未找到: {}", fc.function.name)),
                };
                // Ok 和 Err 都回传给模型，让它有机会自我纠正
                let content = result.unwrap_or_else(|e| format!("执行出错: {e}"));
                tool_msgs.push(tool_msg(&content, &fc.id));
            }

            memory.add(assistant_msg(&text, Some(tool_calls))).await;
            for m in tool_msgs {
                memory.add(m).await;
            }
            // 工具结果已入 memory，进入下一轮迭代
        }
        Err(LlmError::MaxIterations(self.max_iterations))
    }
}

