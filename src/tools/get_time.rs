use crate::tools::Tool;
use async_trait::async_trait;
use serde_json::json;

pub struct GetTime {}



#[async_trait]
impl Tool for GetTime {
    fn name(&self) -> &str {
        "get_time"
    }
    fn description(&self) -> &str {
        "A tool that returns the current local date and time."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
            },
        })
    }
    
    async fn call(&self, arguments: &str) -> Result<String, String> {
        Ok(chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string())
    }
}


#[tokio::test]
async fn return_time() {
    let t = GetTime{};
    let out = t.call("{}").await.unwrap();
    assert!(!out.is_empty());
}