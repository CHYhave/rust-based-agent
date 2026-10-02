pub mod client;

use std::pin::Pin;
use futures::Stream;

// Result<ChatStreamEvent, LlmError> : result的类型要么是ChatStreamEvent，要么是LlmError
// Stream<Item = Result<ChatStreamEvent, LlmError>> : Iterator的类型是Result<ChatStreamEvent, LlmError>，也就是Stream的Item类型
// Stream<Item = Result<ChatStreamEvent, LlmError>> + Send : 线程安全
// Box<dyn Stream<Item = Result<ChatStreamEvent, LlmError>> + Send> : box装堆，由于里面类型是动态的需要加载到堆上
// Pin<Box<dyn Stream<Item = Result<ChatStreamEvent, LlmError>> + Send>> : Pin是为了防止移动，保证指针的稳定性，讨论rust中异步流传递都需要Pin
pub type ChatStream = Pin<Box<dyn Stream<Item = Result<ChatStreamEvent, LlmError>> + Send>>;


#[derive(Debug)]
pub enum ChatStreamEvent {
    Content(String),
    ToolCallDelta {
        index: usize,
        id: Option<String>,
        name: Option<String>,
        args: String,
    },
    Done,
}

#[derive(Debug)]
pub enum LlmError {
    OpenAI(async_openai::error::OpenAIError),
    MaxIterations(usize),
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmError::OpenAI(e) => write!(f, "{e}"),
            LlmError::MaxIterations(n) => write!(f, "max iterations reached: {n}"),
        }
    }
}

impl std::error::Error for LlmError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            LlmError::OpenAI(e) => Some(e),
            LlmError::MaxIterations(_) => None,
        }
    }

    fn cause(&self) -> Option<&dyn std::error::Error> {
        self.source()
    }
}

impl From<async_openai::error::OpenAIError> for LlmError {
    fn from(value: async_openai::error::OpenAIError) -> Self {
        LlmError::OpenAI(value)
    }
}