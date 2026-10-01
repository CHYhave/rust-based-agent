
use crate::tools::Tool;
use async_trait::async_trait;
use serde_json::json;
pub struct Calculator {
}

#[async_trait]
impl Tool for Calculator {
    fn name(&self) ->  &str {
        "calculator"
    }

    fn description(&self) ->  &str {
        "A simple calculator tool that can perform basic arithmetic operations, include addition (+), subtraction (-), multiplication (*), and division (/)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "expression": {
                    "type": "string",
                    "description": "The arithmetic expression to evaluate."
                }
            },
        })
    }

    async fn call(&self, arguments: &str) -> Result<String, String> {
        let v: serde_json::Value = 
            serde_json::from_str(arguments).map_err(|e| e.to_string())?;
        let expr = v["expression"]
        .as_str()
        .ok_or("缺少 expression 参数".to_string())?;
        
        let result = eval(expr)?;
        Ok(result.to_string())
    }
}

#[derive(Debug, Clone)]
enum Token {
    Num(f64),
    Plus, Minus, Star, Slash,
    LParen, RParen,
}

fn lex(input: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();

    while let Some(&c) = chars.peek() {
        match c {
            ' ' | '\t' | '\n' => {
                chars.next();
            }
            '0'..='9' | '.' => {
                let mut s = String::new();
                while let Some(&d) = chars.peek() {
                    if d.is_ascii_digit() || d == '.' {
                        s.push(d);
                        chars.next();
                    } else {
                        break;
                    }
                }
                let n = s.parse::<f64>().map_err(|_| format!("非法数字: {s}"))?;
                tokens.push(Token::Num(n));
            }
            '+' => { tokens.push(Token::Plus); chars.next(); }
            '-' => { tokens.push(Token::Minus); chars.next(); }
            '*' => { tokens.push(Token::Star); chars.next(); }
            '/' => { tokens.push(Token::Slash); chars.next(); }
            '(' => { tokens.push(Token::LParen); chars.next(); }
            ')' => { tokens.push(Token::RParen); chars.next(); }
            other => return Err(format!("无法识别的字符: {other}"))
        }
    }
    Ok(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens: tokens,
            pos: 0
        }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn bump(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.pos).cloned();
        if t.is_some() { self.pos += 1; }
        t
    }

    fn expr(&mut self) -> Result<f64, String> {
        let mut left = self.term()?;
        while matches!(self.peek(), Some(Token::Plus) | Some(Token::Minus)) {
            let op = self.bump().unwrap();
            let right = self.term()?;
            left = match op {
                Token::Plus => left + right,
                Token::Minus => left - right,
                _ => unreachable!(),
            };
        }
        Ok(left)
    }

    fn term(&mut self) -> Result<f64, String> {
        let mut left = self.factor()?;
        while matches!(self.peek(), Some(Token::Star) | Some(Token::Slash)) {
            let op = self.bump().unwrap();
            let right = self.term()?;
            left = match op {
                Token::Star => left * right,
                Token::Slash => {
                    if right == 0.0 {
                        return Err("除数不能为0".to_string());
                    }
                    left / right
                }
                _ => unreachable!(),
            };
        }
        Ok(left)
    }

    fn factor(&mut self) -> Result<f64, String> {
        match self.bump() {
            Some(Token::Num(n)) => Ok(n),
            Some(Token::LParen) => {
                let result = self.expr()?;
                match self.bump() {
                    Some(Token::RParen) => Ok(result),
                    _ => Err("缺少右括号".into()),
                }
            }
            Some(Token::Minus) => Ok(-self.factor()?),
            _ => Err("Unexpected token".to_string()),
        }
    }
}


fn eval(expression: &str) -> Result<f64, String> {
    let tokens = lex(expression)?;
    let mut p = Parser { tokens, pos: 0 };
    let v = p.expr()?;
    if p.pos != p.tokens.len() {
        return Err("表达式末尾有多余内容".to_string());
    }
    Ok(v)
}

#[test]
fn basic() {
    assert_eq!(eval("1+2*3").unwrap(), 7.0);
}


#[test]
fn parens() {
    assert_eq!(eval("(1+2)*3").unwrap(), 9.0);
}

#[test]
fn unary() {
    assert_eq!(eval("-3+5").unwrap(), 2.0);
}

#[test]
fn div_zero() {
    assert!(eval("1/0").is_err());
}

#[test]
fn syntax_err() {
    assert!(eval("1+").is_err());
    assert!(eval("(1").is_err());
}

#[test]
fn spaces() {
    assert_eq!(eval(" 2 * ( 3 + 4)").unwrap(), 14.0);
}

#[test]
fn decimals() {
    assert_eq!(eval("0.5+0.25").unwrap(), 0.75);
    assert_eq!(eval(".5*2").unwrap(), 1.0); // 前导小数点：".5" 也是合法数字
}

#[test]
fn nested_parens() {
    assert_eq!(eval("((1+2)*(3+4))/7").unwrap(), 3.0);
}

#[test]
fn left_assoc() {
    // 加减左结合：(10-4)-3 = 3，不是 10-(4-3) = 9
    assert_eq!(eval("10-4-3").unwrap(), 3.0);
}

#[test]
fn division_is_float() {
    assert_eq!(eval("7/2").unwrap(), 3.5);
}

#[test]
fn double_unary() {
    assert_eq!(eval("--5").unwrap(), 5.0);
}

#[test]
fn empty_input() {
    assert!(eval("").is_err());
    assert!(eval("   ").is_err());
}

#[test]
fn bad_char() {
    assert!(eval("1&2").is_err());
    assert!(eval("3,14").is_err()); // 逗号不是小数点
}

#[tokio::test]
async fn tool_call_with_json_args() {
    let c = Calculator {};
    let out = c.call(r#"{"expression":"(1+2)*3"}"#).await.unwrap();
    assert_eq!(out, "9");
}

#[tokio::test]
async fn tool_call_missing_expression() {
    let c = Calculator {};
    assert!(c.call("{}").await.is_err());
}