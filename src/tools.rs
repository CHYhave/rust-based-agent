pub mod calculator;
pub mod get_time;
pub mod load_skill;
pub mod registry;

use async_trait::async_trait;

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> serde_json::Value;
    async fn call(&self, arguments: &str) -> Result<String, String>;
}