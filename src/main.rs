pub(crate) mod llm;
pub(crate) mod memory;
pub(crate) mod tools;
pub(crate) mod skill;
pub(crate) mod config;
pub mod agent;
mod repl;

use std::error::Error;
use std::path::Path;
use std::sync::Arc;

use crate::agent::Agent;
use crate::llm::client::OpenAiClient;
use crate::memory::memory::{InMemoryMemory, Memory};
use crate::skill::discovery::{SkillMeta, discover_skills};
use crate::tools::registry::ToolRegistry;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let (skill_metas, skill_detail) = discover_skills(Path::new("skills"));
    let system_prompt = build_system_prompt(&skill_metas);
   
    let memory = InMemoryMemory::new();
    memory.add(crate::memory::memory::system_msg(&system_prompt)).await;
   
    let mut tool_registry = ToolRegistry::new();
    tool_registry.register(Arc::new(crate::tools::calculator::Calculator{}));
    tool_registry.register(Arc::new(crate::tools::get_time::GetTimeTool{}));
    tool_registry.register(Arc::new(crate::tools::load_skill::LoadSkillTool::new(Arc::new(skill_detail))));
    
    let llm = OpenAiClient::new();

    let agent = Agent::new(Arc::new(llm), Arc::new(memory), tool_registry, system_prompt);
    repl::run(agent).await?;
    Ok(())
}

fn build_system_prompt(skill_metas: &Vec<SkillMeta>) -> String {
    let mut skill_description = String::new();
    for skill_meta in skill_metas {
        skill_description.push_str(&format!("- {}: {}\n", skill_meta.name, skill_meta.description));
    }
    format!("
    你是终端智能体。可用技能：
    {skill_description}
    当任务适配某个技能时，先用 load_skill 工具加载完整指令并严格遵循。
    ")
}