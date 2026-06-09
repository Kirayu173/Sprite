//! Prompt-debug helpers for inspecting model-visible input assembly.

use std::io;
use std::path::Path;

use runtime_protocol::dynamic_tools::DynamicToolSpec;
use runtime_protocol::models::BaseInstructions;
use runtime_protocol::models::ResponseInputItem;
use runtime_protocol::models::ResponseItem;
use runtime_protocol::protocol::InitialHistory;
use runtime_protocol::protocol::RolloutItem;
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

pub fn build_prompt_debug_dump_from_history(
    initial_history: &InitialHistory,
    user_input: Vec<UserInput>,
) -> PromptDebugDump {
    build_prompt_debug_dump(PromptDebugInput {
        existing_items: model_visible_items_from_history(initial_history),
        user_input,
        base_instructions: initial_history.get_base_instructions(),
        dynamic_tools: initial_history.get_dynamic_tools().unwrap_or_default(),
    })
}

fn model_visible_items_from_history(initial_history: &InitialHistory) -> Vec<ResponseItem> {
    initial_history
        .get_rollout_items()
        .into_iter()
        .flat_map(|item| match item {
            RolloutItem::ResponseItem(item) => vec![item],
            RolloutItem::Compacted(compacted) => {
                if let Some(replacement_history) = compacted.replacement_history {
                    replacement_history
                } else {
                    vec![ResponseItem::from(compacted)]
                }
            }
            RolloutItem::SessionMeta(_)
            | RolloutItem::TurnContext(_)
            | RolloutItem::EventMsg(_) => Vec::new(),
        })
        .collect()
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
    use runtime_protocol::ThreadId;
    use runtime_protocol::models::ContentItem;
    use runtime_protocol::protocol::CompactedItem;
    use runtime_protocol::protocol::InitialHistory;
    use runtime_protocol::protocol::ResumedHistory;
    use runtime_protocol::protocol::RolloutItem;
    use runtime_protocol::protocol::SessionMeta;
    use runtime_protocol::protocol::SessionMetaLine;
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

    #[test]
    fn debug_dump_from_history_uses_session_metadata_and_model_visible_items() {
        let history = InitialHistory::Forked(vec![
            RolloutItem::SessionMeta(SessionMetaLine {
                meta: SessionMeta {
                    id: ThreadId::default(),
                    base_instructions: Some(BaseInstructions {
                        text: "session base".to_string(),
                    }),
                    dynamic_tools: Some(vec![DynamicToolSpec {
                        namespace: Some("docs".to_string()),
                        name: "lookup".to_string(),
                        description: "Look up docs".to_string(),
                        input_schema: json!({"type": "object"}),
                        defer_loading: false,
                    }]),
                    ..SessionMeta::default()
                },
                git: None,
            }),
            RolloutItem::ResponseItem(ResponseItem::Message {
                id: None,
                role: "assistant".to_string(),
                content: vec![ContentItem::OutputText {
                    text: "previous answer".to_string(),
                }],
                phase: None,
            }),
        ]);

        let dump = build_prompt_debug_dump_from_history(
            &history,
            vec![UserInput::Text {
                text: "next question".to_string(),
                text_elements: Vec::new(),
            }],
        );

        assert_eq!(dump.base_instructions.text, "session base");
        assert_eq!(dump.dynamic_tools[0].name, "lookup");
        assert_eq!(dump.input.len(), 2);
        assert_eq!(
            dump.input.last(),
            Some(&ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "next question".to_string()
                }],
                phase: None,
            })
        );
    }

    #[test]
    fn debug_dump_from_history_uses_compaction_replacement_history() {
        let replacement = ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "replacement context".to_string(),
            }],
            phase: None,
        };
        let history = InitialHistory::Resumed(ResumedHistory {
            conversation_id: ThreadId::default(),
            history: vec![RolloutItem::Compacted(CompactedItem {
                message: "summary should not be used".to_string(),
                replacement_history: Some(vec![replacement.clone()]),
            })],
            rollout_path: None,
        });

        let dump = build_prompt_debug_dump_from_history(&history, Vec::new());

        assert_eq!(dump.input, vec![replacement]);
    }
}
