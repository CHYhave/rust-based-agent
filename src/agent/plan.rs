
pub(crate) fn parse_plan(text: &str) -> Vec<String> {
    let mut plans = Vec::new();
    if text.is_empty() {
        return plans;
    }
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some((prefix, step)) = split_once_any(line, &['.', '、']) {
            let step = step.trim();
            if is_all_ascii_digits(prefix.trim()) && !step.is_empty() {
                plans.push(step.to_string());
            }
        }
    }
    plans
}

fn split_once_any<'a>(s: &'a str, delims: &[char]) -> Option<(&'a str, &'a str)> {
    let idx = s.find(|c| delims.contains(&c))?;
    let d = s[idx..].chars().next().unwrap();
    let rest = &s[idx + d.len_utf8()..];
    Some((&s[..idx], rest))
}

fn is_all_ascii_digits(s: &str) -> bool {
    // 注意：空字符串的 all() 返回 true，必须显式排除
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod test {
    use crate::agent::plan::parse_plan;

    #[test]
    fn parses_numbered_lines() {
        let text = "1. 先查时间\n2. 再计算\n3. 总结结果";
        assert_eq!(parse_plan(text), vec!["先查时间", "再计算", "总结结果"]);
    }

    #[test]
    fn skips_junk_lines() {
        let text = "好的，计划如下：\n1. 步骤一\n\n2、步骤二\n希望对你有帮助";
        assert_eq!(parse_plan(text), vec!["步骤一", "步骤二"]);
    }

    #[test]
    fn empty_when_no_steps() {
        assert!(parse_plan("没有任何编号").is_empty());
        assert!(parse_plan("").is_empty());
    }

    #[test]
    fn rejects_empty_prefix_and_empty_step() {
        // ". 没有编号前缀"：分隔符前为空，不是步骤
        // "1. "：步骤内容为空，不收集
        assert!(parse_plan(". 没有编号前缀").is_empty());
        assert!(parse_plan("1. ").is_empty());
    }
}