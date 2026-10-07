use std::io::{self, BufRead, Write};

use crate::agent::{Agent, Mode};

pub const HELP: &str = "可用命令：
  /mode [react|reflect|plan]  切换/查询 Agent 范式
  /verbose [on|off]           开关/切换中间过程输出（反思细节、计划步骤）
  /help                       显示本帮助
  exit                        退出";

#[derive(Debug, PartialEq)]
pub enum ReplCommand {
    /// None = 查询当前模式
    Mode(Option<Mode>),
    /// None = 切换（取反）
    Verbose(Option<bool>),
    Help,
}

/// None = 非命令（普通对话）；Some(Err) = 未知命令/参数错误
pub fn parse_command(input: &str) -> Option<Result<ReplCommand, String>> {
    let body = input.strip_prefix('/')?;
    let mut parts = body.split_whitespace();
    // 输入仅为 "/" 时视为普通对话
    let cmd = parts.next()?;
    let args: Vec<&str> = parts.collect();

    Some(match cmd {
        "mode" => match args.as_slice() {
            [] => Ok(ReplCommand::Mode(None)),
            ["react"] => Ok(ReplCommand::Mode(Some(Mode::ReAct))),
            ["reflect"] => Ok(ReplCommand::Mode(Some(Mode::Reflect))),
            ["plan"] => Ok(ReplCommand::Mode(Some(Mode::PlanSolve))),
            _ => Err("用法: /mode [react|reflect|plan]".to_string()),
        },
        "verbose" => match args.as_slice() {
            [] => Ok(ReplCommand::Verbose(None)),
            ["on"] => Ok(ReplCommand::Verbose(Some(true))),
            ["off"] => Ok(ReplCommand::Verbose(Some(false))),
            _ => Err("用法: /verbose [on|off]".to_string()),
        },
        "help" => Ok(ReplCommand::Help),
        _ => Err(format!("未知命令: /{cmd}，输入 /help 查看可用命令")),
    })
}

pub async fn run(mut agent: Agent) -> io::Result<()> {
    let stdin = io::stdin();
    let mut handle = stdin.lock();
    loop {
        print!("> ");
        io::stdout().flush()?;

        let mut input = String::new();
        if handle.read_line(&mut input)? == 0 {
            break; // EOF
        }
        let input = input.trim();
        if input == "exit" {
            break;
        }
        if input.is_empty() {
            continue;
        }

        match parse_command(input) {
            // 普通对话：Ok 时回复已在流式输出中打印；Err 打印到 stderr 后继续
            None => {
                if let Err(e) = agent.ask(input).await {
                    eprintln!("出错: {e}");
                }
            }
            Some(Err(e)) => eprintln!("{e}"),
            Some(Ok(cmd)) => match cmd {
                ReplCommand::Mode(Some(m)) => {
                    agent.set_mode(m);
                    println!("已切换到 {m:?} 模式");
                }
                ReplCommand::Mode(None) => {
                    println!("当前模式: {:?}", agent.mode());
                }
                ReplCommand::Verbose(Some(on)) => {
                    agent.set_verbose(on);
                    println!("verbose: {on}");
                }
                ReplCommand::Verbose(None) => {
                    let v = !agent.verbose();
                    agent.set_verbose(v);
                    println!("verbose: {v}");
                }
                ReplCommand::Help => println!("{HELP}"),
            },
        }
    }
    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn parses_mode_with_arg() {
        assert_eq!(
            parse_command("/mode reflect"),
            Some(Ok(ReplCommand::Mode(Some(Mode::Reflect))))
        );
        assert_eq!(
            parse_command("/mode plan"),
            Some(Ok(ReplCommand::Mode(Some(Mode::PlanSolve))))
        );
        assert_eq!(
            parse_command("/mode react"),
            Some(Ok(ReplCommand::Mode(Some(Mode::ReAct))))
        );
    }

    #[test]
    fn parses_mode_query() {
        assert_eq!(parse_command("/mode"), Some(Ok(ReplCommand::Mode(None))));
    }

    #[test]
    fn parses_verbose() {
        assert_eq!(
            parse_command("/verbose on"),
            Some(Ok(ReplCommand::Verbose(Some(true))))
        );
        assert_eq!(
            parse_command("/verbose off"),
            Some(Ok(ReplCommand::Verbose(Some(false))))
        );
        assert_eq!(
            parse_command("/verbose"),
            Some(Ok(ReplCommand::Verbose(None)))
        );
    }

    #[test]
    fn parses_help() {
        assert_eq!(parse_command("/help"), Some(Ok(ReplCommand::Help)));
    }

    #[test]
    fn rejects_unknown_and_bad_args() {
        assert!(matches!(parse_command("/foo"), Some(Err(_))));
        assert!(matches!(parse_command("/mode fly"), Some(Err(_))));
        assert!(matches!(parse_command("/verbose maybe"), Some(Err(_))));
    }

    #[test]
    fn non_command_returns_none() {
        assert_eq!(parse_command("你好"), None);
        assert_eq!(parse_command("exit"), None);
    }
}
