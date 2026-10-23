use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{json, Value};

use super::backend::{BackendError, FailureKind};
use super::types::{ContentBlock, Message, Role, StopReason, ToolChatRequest, ToolSpec};

static TOOL_USE_COUNTER: AtomicU64 = AtomicU64::new(0);

pub struct EmulatedPrompt {
    pub system: String,
    pub user: String,
    pub schema: Value,
}

pub fn prompt(request: &ToolChatRequest) -> Result<EmulatedPrompt, BackendError> {
    if request.messages.is_empty() || request.messages.last().map(|m| m.role) != Some(Role::User) {
        return Err(BackendError::new(
            FailureKind::InvalidRequest,
            "last_message_not_user",
        ));
    }
    Ok(EmulatedPrompt {
        system: system_prompt(&request.system, &request.tools)?,
        user: transcript(&request.messages)?,
        schema: schema(&request.tools),
    })
}

fn system_prompt(system: &str, tools: &[ToolSpec]) -> Result<String, BackendError> {
    let mut prompt = system.trim().to_owned();
    if !prompt.is_empty() {
        prompt.push_str("\n\n");
    }
    prompt.push_str(
        "Reply only through the structured output. Put your final reply for the user into `answer`.",
    );
    if tools.is_empty() {
        return Ok(prompt);
    }
    let catalogue = serde_json::to_string(tools)
        .map_err(|_| BackendError::new(FailureKind::InvalidRequest, "tools"))?;
    prompt.push_str(
        " You may call tools instead of answering. To call tools, leave `answer` out and put one or more calls into `tool_calls`, each with the tool `name` and an `input` object that matches the tool's input_schema. Tool results come back in the next turn inside <tool_result> elements. Only call tools from this list:\n",
    );
    prompt.push_str(&catalogue);
    Ok(prompt)
}

fn transcript(messages: &[Message]) -> Result<String, BackendError> {
    if let [Message {
        role: Role::User,
        content,
    }] = messages
    {
        if let [ContentBlock::Text { text }] = content.as_slice() {
            return Ok(text.clone());
        }
    }
    let mut out = String::from("<conversation>\n");
    for message in messages {
        let tag = match message.role {
            Role::User => "user",
            Role::Assistant => "assistant",
        };
        let _ = writeln!(out, "<{tag}>");
        for block in &message.content {
            render_block(&mut out, block)?;
        }
        let _ = writeln!(out, "</{tag}>");
    }
    out.push_str("</conversation>\nContinue the conversation as the assistant.");
    Ok(out)
}

fn render_block(out: &mut String, block: &ContentBlock) -> Result<(), BackendError> {
    match block {
        ContentBlock::Text { text } => {
            let _ = writeln!(out, "{text}");
        }
        ContentBlock::ToolUse { id, name, input } => {
            let input = serde_json::to_string(input)
                .map_err(|_| BackendError::new(FailureKind::InvalidRequest, "tool_input"))?;
            let _ = writeln!(
                out,
                "<tool_call id=\"{}\" name=\"{}\">{input}</tool_call>",
                attribute(id),
                attribute(name)
            );
        }
        ContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => {
            let _ = writeln!(
                out,
                "<tool_result id=\"{}\" error=\"{is_error}\">{content}</tool_result>",
                attribute(tool_use_id)
            );
        }
    }
    Ok(())
}

fn attribute(value: &str) -> String {
    value
        .chars()
        .filter(|character| !matches!(character, '"' | '<' | '>'))
        .collect()
}

pub fn schema(tools: &[ToolSpec]) -> Value {
    if tools.is_empty() {
        return json!({
            "type": "object",
            "properties": {"answer": {"type": "string"}},
            "required": ["answer"],
            "additionalProperties": false
        });
    }
    let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_str()).collect();
    json!({
        "type": "object",
        "properties": {
            "answer": {"type": "string"},
            "tool_calls": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string", "enum": names},
                        "input": {"type": "object"}
                    },
                    "required": ["name", "input"],
                    "additionalProperties": false
                }
            }
        },
        "additionalProperties": false
    })
}

pub fn decision(
    output: &Value,
    tools: &[ToolSpec],
) -> Result<(Vec<ContentBlock>, StopReason), BackendError> {
    let invalid = |reason| BackendError::new(FailureKind::InvalidOutput, reason);
    let object = output.as_object().ok_or_else(|| invalid("not_object"))?;
    let answer = object
        .get("answer")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty());
    let calls = match object.get("tool_calls") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(calls)) => calls.clone(),
        Some(_) => return Err(invalid("tool_calls_not_array")),
    };
    let mut content = Vec::new();
    if let Some(text) = answer {
        content.push(ContentBlock::Text {
            text: text.to_owned(),
        });
    }
    if calls.is_empty() {
        return if content.is_empty() {
            Err(invalid("empty_answer"))
        } else {
            Ok((content, StopReason::EndTurn))
        };
    }
    for call in calls {
        let name = call
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| tools.iter().any(|tool| tool.name == *name))
            .ok_or_else(|| invalid("unknown_tool"))?;
        let input = call
            .get("input")
            .filter(|input| input.is_object())
            .cloned()
            .ok_or_else(|| invalid("tool_input_not_object"))?;
        content.push(ContentBlock::ToolUse {
            id: tool_use_id(),
            name: name.to_owned(),
            input,
        });
    }
    Ok((content, StopReason::ToolUse))
}

fn tool_use_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    let counter = TOOL_USE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("toolu_gw{nanos:x}{counter:x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool() -> ToolSpec {
        ToolSpec {
            name: "hero_lookup".into(),
            description: "Look up a hero".into(),
            input_schema: json!({"type": "object", "properties": {"hero": {"type": "string"}}}),
        }
    }

    #[test]
    fn tool_calls_become_tool_use_blocks_with_unique_ids() {
        let (content, stop) = decision(
            &json!({"tool_calls": [{"name": "hero_lookup", "input": {"hero": "Haze"}}, {"name": "hero_lookup", "input": {}}]}),
            &[tool()],
        )
        .expect("tool round");
        assert_eq!(stop, StopReason::ToolUse);
        let ids: Vec<&str> = content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::ToolUse { id, .. } => Some(id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0], ids[1]);
    }

    #[test]
    fn answers_unknown_tools_and_empty_outputs_are_checked() {
        let (content, stop) =
            decision(&json!({"answer": " Haze "}), &[tool()]).expect("final answer");
        assert_eq!(stop, StopReason::EndTurn);
        assert_eq!(
            content,
            vec![ContentBlock::Text {
                text: "Haze".into()
            }]
        );
        for output in [
            json!({"tool_calls": [{"name": "rm", "input": {}}]}),
            json!({"tool_calls": [{"name": "hero_lookup", "input": "x"}]}),
            json!({"answer": "  "}),
            json!([]),
        ] {
            assert_eq!(
                decision(&output, &[tool()])
                    .map(|_| ())
                    .err()
                    .map(|e| e.kind),
                Some(FailureKind::InvalidOutput)
            );
        }
    }

    #[test]
    fn single_user_text_passes_through_and_history_is_rendered() {
        let single = ToolChatRequest {
            system: "sys".into(),
            messages: vec![Message {
                role: Role::User,
                content: vec![ContentBlock::Text { text: "hi".into() }],
            }],
            tools: Vec::new(),
            max_tokens: None,
        };
        let rendered = prompt(&single).expect("prompt");
        assert_eq!(rendered.user, "hi");
        assert_eq!(rendered.schema["required"], json!(["answer"]));
        let mut round = single.clone();
        round.tools = vec![tool()];
        round.messages.push(Message {
            role: Role::Assistant,
            content: vec![ContentBlock::ToolUse {
                id: "toolu_1".into(),
                name: "hero_lookup".into(),
                input: json!({"hero": "Haze"}),
            }],
        });
        round.messages.push(Message {
            role: Role::User,
            content: vec![ContentBlock::ToolResult {
                tool_use_id: "toolu_1".into(),
                content: "Haze is an assassin".into(),
                is_error: false,
            }],
        });
        let rendered = prompt(&round).expect("prompt");
        assert!(rendered.user.contains(
            "<tool_call id=\"toolu_1\" name=\"hero_lookup\">{\"hero\":\"Haze\"}</tool_call>"
        ));
        assert!(rendered.user.contains(
            "<tool_result id=\"toolu_1\" error=\"false\">Haze is an assassin</tool_result>"
        ));
        assert!(rendered.system.starts_with("sys\n\n"));
        assert!(rendered.system.contains("hero_lookup"));
        assert_eq!(
            rendered.schema["properties"]["tool_calls"]["items"]["properties"]["name"]["enum"],
            json!(["hero_lookup"])
        );
        round.messages.pop();
        assert_eq!(
            prompt(&round).map(|_| ()).err().map(|e| e.kind),
            Some(FailureKind::InvalidRequest)
        );
    }
}
