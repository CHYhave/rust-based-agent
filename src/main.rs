pub(crate) mod llm;
pub(crate) mod memory;
pub(crate) mod tools;
pub(crate) mod skill;


use async_openai::types::chat::{CreateChatCompletionRequest, CreateChatCompletionResponse};
use async_openai::Client;
use serde_json::json;
use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let client = Client::new();
    let request = json!({
        "messages":[
            {
                "role": "user",
                "content": "Hello, how are you?"
            }
        ],
        "model": "deepseek-flash",
        "store": false
    });

    // json! 构造的 Value 先反序列化成强类型请求
    let request: CreateChatCompletionRequest = serde_json::from_value(request)?;
    let response: CreateChatCompletionResponse = client.chat().create(request).await?;

    // 字段都是 pub 的，直接点出来即可
    let content = response.choices[0]
        .message
        .content
        .as_deref()
        .unwrap_or("（模型没有返回文本）");

    println!("id: {}", response.id);
    println!("model: {}", response.model);
    println!("finish_reason: {:?}", response.choices[0].finish_reason);
    println!("content: {content}");
    Ok(())
}
