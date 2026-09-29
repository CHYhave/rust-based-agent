use std::pin::Pin;
use futures::Stream;

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