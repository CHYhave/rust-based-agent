use std::{collections::HashMap, fs, path::Path};

use serde::Deserialize;

#[derive(Deserialize, Debug)]
pub struct SkillMeta {
    pub name: String,
    pub description: String,
}

pub fn discover_skills(dir: &Path) -> (Vec<SkillMeta>, HashMap<String, String>) {
    let mut skill_metas: Vec<SkillMeta> = Vec::new();
    let mut skill_body_map: HashMap<String, String> = HashMap::new();
    match fs::read_dir(dir) {
        Ok(entries) => {
            for entry in entries {
                match entry {
                    Ok(e) => {
                        let path = e.path();
                        let skill_md_path = path.join("SKILL.md");
                        match resolve_skill(&skill_md_path) {
                            Ok((meta, content)) => {
                                skill_body_map.insert(meta.name.clone(), content);
                                skill_metas.push(meta);
                            }
                            Err(e) => println!("解析SKILL.md失败: {e}")
                        }
                    }
                    Err(e) => println!("目录遍历失败 {e}")
                }
            }
        }
        Err(e) => println!("打开目录失败: {e}")
    }
    (skill_metas, skill_body_map)
}

pub fn resolve_skill(path: &Path) -> Result<(SkillMeta, String), Box<dyn std::error::Error>> {
    let content = fs::read_to_string(path)?;
    // 1. 必须以 --- 开头
    let content = content.trim_start();
    let rest = content.strip_prefix("---")
        .ok_or("文件不以 --- 开头，没有 frontmatter")?;

    // 2. 找到结尾的 ---
    let end = rest.find("\n---")
        .ok_or("找不到 frontmatter 的结束 ---")?;

    let yaml = &rest[..end];
    let body = &rest[end+4..].trim_start();

    let meta: SkillMeta = serde_yaml::from_str(yaml)?;
    Ok((SkillMeta { 
        name: meta.name,
        description: meta.description,
    },  body.to_string()))
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    pub fn discovery_skill() -> std::io::Result<()> {
        let skill_root_path = std::env::temp_dir().join(format!("me-skill-test-{}", std::process::id()));
        fs::create_dir_all(&skill_root_path)?;
        let my_skill_path = skill_root_path.join("my-skill");
        fs::create_dir_all(&my_skill_path)?;
        let skill_md_path = my_skill_path.join("SKILL.md");
        let text = concat!(
            "---\n",
            "name: my-skill\n",
            "description: 演示技能\n",
            "---\n",
            "测试正文",
        );
        fs::write(skill_md_path, text)?;
        let (metas, body_map) = discover_skills(&skill_root_path);
        assert_eq!(metas.len(), 1);
        assert_eq!(body_map.len(), 1);
        let skill_meta = metas.get(0).unwrap();
        assert_eq!(skill_meta.name, "my-skill");
        assert_eq!(skill_meta.description, "演示技能");
        assert!(body_map.contains_key("my-skill"));
        let body = body_map.get("my-skill").unwrap();
        assert_eq!(body, "测试正文");
        fs::remove_dir_all(&skill_root_path)?;
        Ok(())
    }
}