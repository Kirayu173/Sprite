//! Prompt-debug helpers for inspecting model-visible input assembly.

use runtime_protocol::models::ResponseInputItem;
use runtime_protocol::models::ResponseItem;
use runtime_protocol::user_input::UserInput;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PromptDebugInput {
    pub existing_items: Vec<ResponseItem>,
    pub user_input: Vec<UserInput>,
}

pub fn build_prompt_input(input: PromptDebugInput) -> Vec<ResponseItem> {
    let mut items = input.existing_items;
    if !input.user_input.is_empty() {
        items.push(ResponseItem::from(ResponseInputItem::from(
            input.user_input,
        )));
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use runtime_protocol::models::ContentItem;

    #[test]
    fn builds_prompt_input_from_user_text() {
        let input = PromptDebugInput {
            existing_items: Vec::new(),
            user_input: vec![UserInput::Text {
                text: "inspect this".to_string(),
                text_elements: Vec::new(),
            }],
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
}
