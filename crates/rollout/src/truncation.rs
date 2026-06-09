//! Helpers for truncating rollouts based on effective turn boundaries.

use runtime_protocol::models::ResponseItem;
use runtime_protocol::protocol::EventMsg;
use runtime_protocol::protocol::InterAgentCommunication;
use runtime_protocol::protocol::RolloutItem;

pub fn initial_history_has_prior_user_turns(items: &[RolloutItem]) -> bool {
    items.iter().any(rollout_item_is_user_turn_boundary)
}

fn rollout_item_is_user_turn_boundary(item: &RolloutItem) -> bool {
    matches!(item, RolloutItem::ResponseItem(item) if is_user_turn_boundary(item))
}

pub fn is_user_turn_boundary(item: &ResponseItem) -> bool {
    item.is_user_message() || is_trigger_turn_boundary(item)
}

pub fn user_message_positions_in_rollout(items: &[RolloutItem]) -> Vec<usize> {
    let mut user_positions = Vec::new();
    for (idx, item) in items.iter().enumerate() {
        match item {
            RolloutItem::ResponseItem(item) if is_real_user_message_boundary(item) => {
                user_positions.push(idx);
            }
            RolloutItem::EventMsg(EventMsg::ThreadRolledBack(rollback)) => {
                let num_turns = usize::try_from(rollback.num_turns).unwrap_or(usize::MAX);
                let new_len = user_positions.len().saturating_sub(num_turns);
                user_positions.truncate(new_len);
            }
            _ => {}
        }
    }
    user_positions
}

pub fn fork_turn_positions_in_rollout(items: &[RolloutItem]) -> Vec<usize> {
    let mut rollback_turn_positions = Vec::new();
    let mut fork_turn_positions = Vec::new();
    for (idx, item) in items.iter().enumerate() {
        match item {
            RolloutItem::ResponseItem(item) => {
                if is_user_turn_boundary(item) {
                    rollback_turn_positions.push(idx);
                }
                if is_real_user_message_boundary(item) || is_trigger_turn_boundary(item) {
                    fork_turn_positions.push(idx);
                }
            }
            RolloutItem::EventMsg(EventMsg::ThreadRolledBack(rollback)) => {
                let num_turns = usize::try_from(rollback.num_turns).unwrap_or(usize::MAX);
                if num_turns == 0 {
                    continue;
                }
                let Some(rollback_start_idx) = rollback_turn_positions
                    .len()
                    .checked_sub(num_turns)
                    .map(|rollback_start| rollback_turn_positions[rollback_start])
                    .or_else(|| rollback_turn_positions.first().copied())
                else {
                    continue;
                };
                let new_rollback_len = rollback_turn_positions.len().saturating_sub(num_turns);
                rollback_turn_positions.truncate(new_rollback_len);
                fork_turn_positions.retain(|position| *position < rollback_start_idx);
            }
            _ => {}
        }
    }
    fork_turn_positions
}

pub fn truncate_rollout_before_nth_user_message_from_start(
    items: &[RolloutItem],
    n_from_start: usize,
) -> Vec<RolloutItem> {
    if n_from_start == usize::MAX {
        return items.to_vec();
    }

    let user_positions = user_message_positions_in_rollout(items);
    if user_positions.len() <= n_from_start {
        return items.to_vec();
    }

    items[..user_positions[n_from_start]].to_vec()
}

pub fn truncate_rollout_to_last_n_fork_turns(
    items: &[RolloutItem],
    n_from_end: usize,
) -> Vec<RolloutItem> {
    if n_from_end == 0 {
        return Vec::new();
    }

    let fork_turn_positions = fork_turn_positions_in_rollout(items);
    let Some(keep_idx) = fork_turn_positions
        .len()
        .checked_sub(n_from_end)
        .map(|position| fork_turn_positions[position])
        .or_else(|| fork_turn_positions.first().copied())
    else {
        return Vec::new();
    };
    items[keep_idx..].to_vec()
}

fn is_real_user_message_boundary(item: &ResponseItem) -> bool {
    match item {
        ResponseItem::Message { role, .. } => role == "user",
        ResponseItem::Other
        | ResponseItem::AgentMessage { .. }
        | ResponseItem::Reasoning { .. }
        | ResponseItem::LocalShellCall { .. }
        | ResponseItem::FunctionCall { .. }
        | ResponseItem::ToolSearchCall { .. }
        | ResponseItem::FunctionCallOutput { .. }
        | ResponseItem::CustomToolCall { .. }
        | ResponseItem::CustomToolCallOutput { .. }
        | ResponseItem::ToolSearchOutput { .. }
        | ResponseItem::WebSearchCall { .. }
        | ResponseItem::ImageGenerationCall { .. }
        | ResponseItem::Compaction { .. }
        | ResponseItem::CompactionTrigger
        | ResponseItem::ContextCompaction { .. } => false,
    }
}

fn is_trigger_turn_boundary(item: &ResponseItem) -> bool {
    match item {
        ResponseItem::Message { role, content, .. } => {
            role == "assistant"
                && InterAgentCommunication::from_message_content(content)
                    .is_some_and(|communication| communication.trigger_turn)
        }
        ResponseItem::AgentMessage { .. } => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use runtime_protocol::AgentPath;
    use runtime_protocol::models::ContentItem;
    use runtime_protocol::models::ReasoningItemReasoningSummary;
    use runtime_protocol::protocol::InterAgentCommunication;
    use runtime_protocol::protocol::ThreadRolledBackEvent;

    fn assert_json_eq<T: serde::Serialize, U: serde::Serialize>(actual: T, expected: U) {
        assert_eq!(
            serde_json::to_value(actual).expect("serialize actual"),
            serde_json::to_value(expected).expect("serialize expected")
        );
    }

    fn user_msg(text: &str) -> ResponseItem {
        ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::OutputText {
                text: text.to_string(),
            }],
            phase: None,
        }
    }

    fn assistant_msg(text: &str) -> ResponseItem {
        ResponseItem::Message {
            id: None,
            role: "assistant".to_string(),
            content: vec![ContentItem::OutputText {
                text: text.to_string(),
            }],
            phase: None,
        }
    }

    fn developer_msg(text: &str) -> ResponseItem {
        ResponseItem::Message {
            id: None,
            role: "developer".to_string(),
            content: vec![ContentItem::InputText {
                text: text.to_string(),
            }],
            phase: None,
        }
    }

    fn inter_agent_msg(text: &str, trigger_turn: bool) -> ResponseItem {
        let communication = InterAgentCommunication::new(
            AgentPath::root(),
            AgentPath::try_from("/root/worker").expect("agent path"),
            Vec::new(),
            text.to_string(),
            trigger_turn,
        );
        communication.to_response_input_item().into()
    }

    #[test]
    fn truncates_rollout_from_start_before_nth_user_only() {
        let items = [
            user_msg("u1"),
            assistant_msg("a1"),
            assistant_msg("a2"),
            user_msg("u2"),
            assistant_msg("a3"),
            ResponseItem::Reasoning {
                id: "r1".to_string(),
                summary: vec![ReasoningItemReasoningSummary::SummaryText {
                    text: "s".to_string(),
                }],
                content: None,
                encrypted_content: None,
            },
            ResponseItem::FunctionCall {
                id: None,
                call_id: "c1".to_string(),
                name: "tool".to_string(),
                namespace: None,
                arguments: "{}".to_string(),
            },
            assistant_msg("a4"),
        ];

        let rollout: Vec<RolloutItem> = items
            .iter()
            .cloned()
            .map(RolloutItem::ResponseItem)
            .collect();

        let truncated =
            truncate_rollout_before_nth_user_message_from_start(&rollout, /*n_from_start*/ 1);
        let expected = vec![
            RolloutItem::ResponseItem(items[0].clone()),
            RolloutItem::ResponseItem(items[1].clone()),
            RolloutItem::ResponseItem(items[2].clone()),
        ];
        assert_json_eq(&truncated, &expected);

        let truncated =
            truncate_rollout_before_nth_user_message_from_start(&rollout, /*n_from_start*/ 2);
        assert_json_eq(&truncated, &rollout);
    }

    #[test]
    fn truncation_max_keeps_full_rollout() {
        let rollout = vec![
            RolloutItem::ResponseItem(user_msg("u1")),
            RolloutItem::ResponseItem(assistant_msg("a1")),
            RolloutItem::ResponseItem(user_msg("u2")),
        ];

        let truncated = truncate_rollout_before_nth_user_message_from_start(&rollout, usize::MAX);

        assert_json_eq(&truncated, &rollout);
    }

    #[test]
    fn truncates_rollout_from_start_applies_thread_rollback_markers() {
        let rollout_items = vec![
            RolloutItem::ResponseItem(user_msg("u1")),
            RolloutItem::ResponseItem(assistant_msg("a1")),
            RolloutItem::ResponseItem(user_msg("u2")),
            RolloutItem::ResponseItem(assistant_msg("a2")),
            RolloutItem::EventMsg(EventMsg::ThreadRolledBack(ThreadRolledBackEvent {
                num_turns: 1,
            })),
            RolloutItem::ResponseItem(user_msg("u3")),
            RolloutItem::ResponseItem(assistant_msg("a3")),
            RolloutItem::ResponseItem(user_msg("u4")),
            RolloutItem::ResponseItem(assistant_msg("a4")),
        ];

        let truncated = truncate_rollout_before_nth_user_message_from_start(
            &rollout_items,
            /*n_from_start*/ 2,
        );
        let expected = rollout_items[..7].to_vec();
        assert_json_eq(&truncated, &expected);
    }

    #[test]
    fn truncates_rollout_to_last_n_fork_turns_counts_trigger_turn_messages() {
        let rollout = vec![
            RolloutItem::ResponseItem(user_msg("u1")),
            RolloutItem::ResponseItem(assistant_msg("a1")),
            RolloutItem::ResponseItem(inter_agent_msg("queued message", false)),
            RolloutItem::ResponseItem(assistant_msg("a2")),
            RolloutItem::ResponseItem(inter_agent_msg("triggered task", true)),
            RolloutItem::ResponseItem(assistant_msg("a3")),
            RolloutItem::ResponseItem(user_msg("u2")),
            RolloutItem::ResponseItem(assistant_msg("a4")),
        ];

        let truncated = truncate_rollout_to_last_n_fork_turns(&rollout, /*n_from_end*/ 2);
        let expected = rollout[4..].to_vec();

        assert_json_eq(&truncated, &expected);
    }

    #[test]
    fn truncates_rollout_to_last_n_fork_turns_drops_startup_prefix_even_when_under_limit() {
        let rollout = vec![
            RolloutItem::ResponseItem(developer_msg("startup developer context")),
            RolloutItem::ResponseItem(user_msg("current task")),
            RolloutItem::ResponseItem(assistant_msg("answer")),
        ];

        let truncated = truncate_rollout_to_last_n_fork_turns(&rollout, /*n_from_end*/ 2);
        let expected = rollout[1..].to_vec();

        assert_json_eq(&truncated, &expected);
    }

    #[test]
    fn truncates_rollout_to_last_n_fork_turns_discards_trigger_boundaries_in_rolled_back_suffix() {
        let rollout = vec![
            RolloutItem::ResponseItem(user_msg("u1")),
            RolloutItem::ResponseItem(user_msg("u2")),
            RolloutItem::ResponseItem(inter_agent_msg("triggered task", true)),
            RolloutItem::ResponseItem(assistant_msg("a1")),
            RolloutItem::EventMsg(EventMsg::ThreadRolledBack(ThreadRolledBackEvent {
                num_turns: 1,
            })),
            RolloutItem::ResponseItem(user_msg("u3")),
            RolloutItem::ResponseItem(assistant_msg("a2")),
        ];

        let truncated = truncate_rollout_to_last_n_fork_turns(&rollout, /*n_from_end*/ 2);
        let expected = rollout[1..].to_vec();

        assert_json_eq(&truncated, &expected);
    }
}
