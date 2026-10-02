use std::{collections::HashMap, sync::Arc};

use crate::tools::Tool;
use async_trait::async_trait;
use serde_json::json;

pub struct LoadSkillTool {
    skills: Arc<HashMap<String, String>>,
}

impl LoadSkillTool {
    pub fn new(skills: Arc<HashMap<String, String>>) -> Self {
        LoadSkillTool { skills }
    }
}

#[async_trait]
impl Tool for LoadSkillTool {
    fn name(&self) ->  &str {
        "load_skill"
    }

    fn description(&self) ->  &str {
        "Load the full instructions for a named skill. Use when a task matches one of the available skills."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "The skill name to load."
                }
            },
        })
    }

    async fn call(&self, arguments: &str) -> Result<String, String> {
        let v: serde_json::Value = 
            serde_json::from_str(arguments).map_err(|e| e.to_string())?;
        let name = v["name"].as_str().ok_or("缺少 name 参数".to_string())?;
        if !self.skills.contains_key(name) {
            return Err("技能不存在".to_string());
        }
        let body = self.skills.get(name).unwrap();
        Ok(body.to_string())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::{collections::HashMap, sync::Arc};

    #[tokio::test]
    async fn loads_existing() {
        let mut contents = HashMap::new();
        contents.insert("demo".to_string(), "做某事的完整指令".to_string());
        let tool = LoadSkillTool::new(Arc::new(contents));
        assert_eq!(tool.name(), "load_skill");
        let out = tool.call(r#"{"name":"demo"}"#).await.unwrap();
        assert_eq!(out, "做某事的完整指令");
    }

    #[tokio::test]
    async fn missing_skill_reports_error() {
        let tool = LoadSkillTool::new(Arc::new(HashMap::new()));
        let out = tool.call(r#"{"name":"nope"}"#).await;
        assert!(out.is_err());
    }
}