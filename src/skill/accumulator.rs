use std::collections::BTreeMap;
use async_openai::types::chat::{ChatCompletionMessageToolCall, ChatCompletionMessageToolCalls, FunctionCall};

pub struct ToolCallAccumulator {
    slots: BTreeMap<usize, Slot>,
}

pub struct Slot {
    id: Option<String>,
    name: Option<String>,
    args: String,
}

impl Slot {
    pub fn new() -> Self {
        Slot { id: None, name: None, args: String::new() }
    }
}

impl ToolCallAccumulator {
    fn new() -> Self {
        ToolCallAccumulator { slots: BTreeMap::new()}
    }

    pub fn feed(&mut self, index: usize, id: Option<String>, name: Option<String>, args: Option<&str>) {
        let slot = self.slots.entry(index).or_insert_with(Slot::new);
        if let Some(id) = id { slot.id = Some(id); }      // None 直接跳过
        if let Some(name) = name { slot.name = Some(name); }
        if let Some(args) = args { slot.args.push_str(args); }
    }

    pub fn into_tool_calls(self) ->  Vec<ChatCompletionMessageToolCalls> {
        self.slots
            .into_iter()
            .filter_map(|(_, slot)| {
                let id = slot.id?;
                Some(ChatCompletionMessageToolCalls::Function(
                    ChatCompletionMessageToolCall {
                        id,
                        function: FunctionCall {
                            name: slot.name.unwrap_or_default(),
                            arguments: slot.args,
                        },
                        ..Default::default() 
                    }
                ))
            })
            .collect()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn accmulates_across_chunks() {
        let mut acc = ToolCallAccumulator::new();
        acc.feed(0, Some("call_1".into()), Some("calculator".into()), None);
        acc.feed(0, None, None, Some(r#"{"expres"#));
        acc.feed(0, None, None, Some(r#"sion":"1+2"}"#));
        let calls = acc.into_tool_calls();
        assert_eq!(calls.len(), 1);
        match &calls[0] {
            ChatCompletionMessageToolCalls::Function(c) => {
                assert_eq!(c.id, "call_1");
                assert_eq!(c.function.name, "calculator");
                assert_eq!(c.function.arguments, r#"{"expression":"1+2"}"#);
            } 
            _ => panic!("unexpected variant"),
        }
    }

    #[test]
    fn interleaved_calls() {
        let mut acc = ToolCallAccumulator::new();
        acc.feed(0, Some("a".into()), Some("t1".into()), None);
        acc.feed(1, Some("b".into()), Some("t2".into()), None);
        acc.feed(0, None, None, Some("{}".into()));
        acc.feed(1, None, None, Some("{}".into()));
        let calls = acc.into_tool_calls();
        assert_eq!(calls.len(), 2);
    }
}