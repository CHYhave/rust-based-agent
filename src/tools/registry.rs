use std::{collections::HashMap, sync::Arc};
use async_openai::types::chat::{ChatCompletionTool, FunctionObject};

use crate::tools::Tool;

pub struct ToolRegistry {
    tools: std::collections::HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        ToolRegistry { tools: HashMap::new() }
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    pub fn schemas(&self) -> Vec<ChatCompletionTool> {
        self.tools.values().map(|tool| {
            // let mut schema = tool.parameters_schema();
            // schema["name"] = serde_json::Value::String(tool.name().to_string());
            // schema["description"] = serde_json::Value::String(tool.description().to_string());
            // schema
            let function = FunctionObject {
                name: tool.name().to_string(),
                description: Some(tool.description().to_string()),
                parameters: tool.parameters_schema().into(),
                strict: Some(true),
            };
            ChatCompletionTool { function: function }
        }).collect()
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use serde_json::json;
    use async_trait::async_trait;
    struct FakeTool;
    #[async_trait]
    impl Tool for FakeTool {
        fn name(&self) ->  &str {
            "fake"
        }
    
        fn description(&self) ->  &str {
            "这是一个测试工具"
        }
    
        fn parameters_schema(&self) -> serde_json::Value {
            json!({
                "type": "object",
                "properties": {}
            })
        }
    
        async fn call(&self, arguments: &str) -> Result<String, String> {
            Ok("oke".into())
        }
    }

    #[test]
    fn register_and_get() {
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(FakeTool));
        assert!(reg.get("fake").is_some());
        assert!(reg.get("nope").is_none());
    }

    #[test]
    fn schemas_match_tools() {
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(FakeTool));
        let schemas = reg.schemas();
        assert_eq!(schemas.len(), 1);
        assert_eq!(schemas[0].function.name, "fake");
    }
}