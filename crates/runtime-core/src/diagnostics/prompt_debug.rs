//! Prompt-debug helpers for inspecting model-visible input assembly.

use std::io;
use std::path::Path;

use runtime_protocol::dynamic_tools::DynamicToolSpec;
use runtime_protocol::models::BaseInstructions;
use runtime_protocol::models::ResponseInputItem;
use runtime_protocol::models::ResponseItem;
use runtime_protocol::user_input::UserInput;
use serde::Serialize;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PromptDebugInput {
    pub existing_items: Vec<ResponseItem>,
    pub user_input: Vec<UserInput>,
    pub base_instructions: Option<BaseInstructions>,
    pub dynamic_tools: Vec<DynamicToolSpec>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PromptDebugDump {
    pub base_instructions: BaseInstructions,
    pub input: Vec<ResponseItem>,
    pub dynamic_tools: Vec<DynamicToolSpec>,
}

impl PromptDebugDump {
    pub fn formatted_input(&self) -> Vec<ResponseItem> {
        self.input.clone()
    }

    pub fn to_json_pretty(&self) -> io::Result<String> {
        serde_json::to_string_pretty(self)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

pub fn build_prompt_input(input: PromptDebugInput) -> Vec<ResponseItem> {
    build_prompt_debug_dump(input).formatted_input()
}

pub fn build_prompt_debug_dump(input: PromptDebugInput) -> PromptDebugDump {
    let PromptDebugInput {
        existing_items,
        user_input,
        base_instructions,
        dynamic_tools,
    } = input;

    let mut items = existing_items;
    if !user_input.is_empty() {
        items.push(ResponseItem::from(ResponseInputItem::from(user_input)));
    }
    PromptDebugDump {
        base_instructions: base_instructions.unwrap_or_default(),
        input: items,
        dynamic_tools,
    }
}

pub async fn write_prompt_debug_dump(
    path: impl AsRef<Path>,
    dump: &PromptDebugDump,
) -> io::Result<()> {
    tokio::fs::write(path, dump.to_json_pretty()?).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use runtime_protocol::models::ContentItem;
    use serde_json::json;

    #[test]
    fn builds_prompt_input_from_user_text() {
        let input = PromptDebugInput {
            existing_items: Vec::new(),
            user_input: vec![UserInput::Text {
                text: "inspect this".to_string(),
                text_elements: Vec::new(),
            }],
            base_instructions: None,
            dynamic_tools: Vec::new(),
        };

        let items = build_prompt_input(input);
        assert_eq!(
            items,
            vec![ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "inspect this".to_string()
                }],
                phase: None,
            }]
        );
    }

    #[test]
    fn debug_dump_contains_base_instructions_history_tools_and_user_input() {
        let dump = build_prompt_debug_dump(PromptDebugInput {
            existing_items: vec![ResponseItem::Message {
                id: None,
                role: "assistant".to_string(),
                content: vec![ContentItem::OutputText {
                    text: "previous answer".to_string(),
                }],
                phase: None,
            }],
            user_input: vec![UserInput::Text {
                text: "next question".to_string(),
                text_elements: Vec::new(),
            }],
            base_instructions: Some(BaseInstructions {
                text: "base rules".to_string(),
            }),
            dynamic_tools: vec![DynamicToolSpec {
                namespace: Some("docs".to_string()),
                name: "lookup".to_string(),
                description: "Look up docs".to_string(),
                input_schema: json!({"type": "object"}),
                defer_loading: false,
            }],
        });

        assert_eq!(dump.base_instructions.text, "base rules");
        assert_eq!(dump.input.len(), 2);
        let json = dump.to_json_pretty().unwrap();
        assert!(json.contains("lookup"));
        assert!(json.contains("Look up docs"));
    }

    #[tokio::test]
    async fn writes_prompt_debug_dump_to_local_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("prompt-debug.json");
        let dump = build_prompt_debug_dump(PromptDebugInput {
            existing_items: Vec::new(),
            user_input: vec![UserInput::Text {
                text: "inspect this".to_string(),
                text_elements: Vec::new(),
            }],
            base_instructions: Some(BaseInstructions {
                text: "debug base".to_string(),
            }),
            dynamic_tools: Vec::new(),
        });

        write_prompt_debug_dump(&path, &dump).await.unwrap();

        let written = tokio::fs::read_to_string(path).await.unwrap();
        assert!(written.contains("debug base"));
        assert!(written.contains("inspect this"));
    }
}
