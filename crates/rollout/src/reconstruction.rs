//! Replay helpers for rebuilding model-visible history from rollout items.

use runtime_protocol::models::ContentItem;
use runtime_protocol::models::ResponseItem;
use runtime_protocol::protocol::EventMsg;
use runtime_protocol::protocol::RolloutItem;
use runtime_protocol::protocol::TurnContextItem;

use crate::truncation::is_user_turn_boundary;

const COMPACT_USER_MESSAGE_MAX_TOKENS: usize = 20_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviousTurnSettings {
    pub model: String,
    pub realtime_active: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct RolloutReconstruction {
    pub history: Vec<ResponseItem>,
    pub previous_turn_settings: Option<PreviousTurnSettings>,
    pub reference_context_item: Option<TurnContextItem>,
}

#[derive(Debug, Default)]
enum TurnReferenceContextItem {
    #[default]
    NeverSet,
    Cleared,
    Latest(Box<TurnContextItem>),
}

#[derive(Debug, Default)]
struct ActiveReplaySegment<'a> {
    turn_id: Option<String>,
    counts_as_user_turn: bool,
    previous_turn_settings: Option<PreviousTurnSettings>,
    reference_context_item: TurnReferenceContextItem,
    base_replacement_history: Option<&'a [ResponseItem]>,
}

pub fn reconstruct_history_from_rollout(rollout_items: &[RolloutItem]) -> RolloutReconstruction {
    let mut base_replacement_history: Option<&[ResponseItem]> = None;
    let mut previous_turn_settings = None;
    let mut reference_context_item = TurnReferenceContextItem::NeverSet;
    let mut pending_rollback_turns = 0usize;
    let mut rollout_suffix = rollout_items;
    let mut active_segment: Option<ActiveReplaySegment<'_>> = None;

    for (index, item) in rollout_items.iter().enumerate().rev() {
        match item {
            RolloutItem::Compacted(compacted) => {
                let active_segment =
                    active_segment.get_or_insert_with(ActiveReplaySegment::default);
                if matches!(
                    active_segment.reference_context_item,
                    TurnReferenceContextItem::NeverSet
                ) {
                    active_segment.reference_context_item = TurnReferenceContextItem::Cleared;
                }
                if active_segment.base_replacement_history.is_none()
                    && let Some(replacement_history) = &compacted.replacement_history
                {
                    active_segment.base_replacement_history = Some(replacement_history);
                    rollout_suffix = &rollout_items[index + 1..];
                }
            }
            RolloutItem::EventMsg(EventMsg::ThreadRolledBack(rollback)) => {
                pending_rollback_turns = pending_rollback_turns
                    .saturating_add(usize::try_from(rollback.num_turns).unwrap_or(usize::MAX));
            }
            RolloutItem::EventMsg(EventMsg::TurnComplete(event)) => {
                let active_segment =
                    active_segment.get_or_insert_with(ActiveReplaySegment::default);
                if active_segment.turn_id.is_none() {
                    active_segment.turn_id = Some(event.turn_id.clone());
                }
            }
            RolloutItem::EventMsg(EventMsg::TurnAborted(event)) => {
                if let Some(active_segment) = active_segment.as_mut() {
                    if active_segment.turn_id.is_none()
                        && let Some(turn_id) = &event.turn_id
                    {
                        active_segment.turn_id = Some(turn_id.clone());
                    }
                } else if let Some(turn_id) = &event.turn_id {
                    active_segment = Some(ActiveReplaySegment {
                        turn_id: Some(turn_id.clone()),
                        ..Default::default()
                    });
                }
            }
            RolloutItem::EventMsg(EventMsg::UserMessage(_)) => {
                let active_segment =
                    active_segment.get_or_insert_with(ActiveReplaySegment::default);
                active_segment.counts_as_user_turn = true;
            }
            RolloutItem::TurnContext(ctx) => {
                let active_segment =
                    active_segment.get_or_insert_with(ActiveReplaySegment::default);
                if active_segment.turn_id.is_none() {
                    active_segment.turn_id = ctx.turn_id.clone();
                }
                if turn_ids_are_compatible(
                    active_segment.turn_id.as_deref(),
                    ctx.turn_id.as_deref(),
                ) {
                    active_segment.previous_turn_settings = Some(PreviousTurnSettings {
                        model: ctx.model.clone(),
                        realtime_active: ctx.realtime_active,
                    });
                    if matches!(
                        active_segment.reference_context_item,
                        TurnReferenceContextItem::NeverSet
                    ) {
                        active_segment.reference_context_item =
                            TurnReferenceContextItem::Latest(Box::new(ctx.clone()));
                    }
                }
            }
            RolloutItem::EventMsg(EventMsg::TurnStarted(event)) => {
                if active_segment.as_ref().is_some_and(|active_segment| {
                    turn_ids_are_compatible(
                        active_segment.turn_id.as_deref(),
                        Some(event.turn_id.as_str()),
                    )
                }) && let Some(active_segment) = active_segment.take()
                {
                    finalize_active_segment(
                        active_segment,
                        &mut base_replacement_history,
                        &mut previous_turn_settings,
                        &mut reference_context_item,
                        &mut pending_rollback_turns,
                    );
                }
            }
            RolloutItem::ResponseItem(response_item) => {
                let active_segment =
                    active_segment.get_or_insert_with(ActiveReplaySegment::default);
                active_segment.counts_as_user_turn |= is_user_turn_boundary(response_item);
            }
            RolloutItem::EventMsg(_) | RolloutItem::SessionMeta(_) => {}
        }

        if base_replacement_history.is_some()
            && previous_turn_settings.is_some()
            && !matches!(reference_context_item, TurnReferenceContextItem::NeverSet)
        {
            break;
        }
    }

    if let Some(active_segment) = active_segment.take() {
        finalize_active_segment(
            active_segment,
            &mut base_replacement_history,
            &mut previous_turn_settings,
            &mut reference_context_item,
            &mut pending_rollback_turns,
        );
    }

    let mut history = HistoryReplay::new();
    let mut saw_legacy_compaction_without_replacement_history = false;
    if let Some(base_replacement_history) = base_replacement_history {
        history.replace(base_replacement_history.to_vec());
    }

    for item in rollout_suffix {
        match item {
            RolloutItem::ResponseItem(response_item) => {
                history.record_item(response_item.clone());
            }
            RolloutItem::Compacted(compacted) => {
                if let Some(replacement_history) = &compacted.replacement_history {
                    history.replace(replacement_history.clone());
                } else {
                    saw_legacy_compaction_without_replacement_history = true;
                    let user_messages = collect_user_messages(history.raw_items());
                    history.replace(build_legacy_compacted_history(
                        &user_messages,
                        &compacted.message,
                    ));
                }
            }
            RolloutItem::EventMsg(EventMsg::ThreadRolledBack(rollback)) => {
                history.drop_last_n_user_turns(rollback.num_turns.into());
            }
            RolloutItem::EventMsg(_)
            | RolloutItem::TurnContext(_)
            | RolloutItem::SessionMeta(_) => {}
        }
    }

    let reference_context_item = match reference_context_item {
        TurnReferenceContextItem::NeverSet | TurnReferenceContextItem::Cleared => None,
        TurnReferenceContextItem::Latest(turn_reference_context_item) => {
            Some(*turn_reference_context_item)
        }
    };
    let reference_context_item = if saw_legacy_compaction_without_replacement_history {
        None
    } else {
        reference_context_item
    };

    RolloutReconstruction {
        history: history.into_items(),
        previous_turn_settings,
        reference_context_item,
    }
}

fn turn_ids_are_compatible(active_turn_id: Option<&str>, item_turn_id: Option<&str>) -> bool {
    active_turn_id
        .is_none_or(|turn_id| item_turn_id.is_none_or(|item_turn_id| item_turn_id == turn_id))
}

fn finalize_active_segment<'a>(
    active_segment: ActiveReplaySegment<'a>,
    base_replacement_history: &mut Option<&'a [ResponseItem]>,
    previous_turn_settings: &mut Option<PreviousTurnSettings>,
    reference_context_item: &mut TurnReferenceContextItem,
    pending_rollback_turns: &mut usize,
) {
    if *pending_rollback_turns > 0 {
        if active_segment.counts_as_user_turn {
            *pending_rollback_turns -= 1;
        }
        return;
    }

    if base_replacement_history.is_none()
        && let Some(segment_base_replacement_history) = active_segment.base_replacement_history
    {
        *base_replacement_history = Some(segment_base_replacement_history);
    }

    if previous_turn_settings.is_none() && active_segment.counts_as_user_turn {
        *previous_turn_settings = active_segment.previous_turn_settings;
    }

    if matches!(reference_context_item, TurnReferenceContextItem::NeverSet)
        && (active_segment.counts_as_user_turn
            || matches!(
                active_segment.reference_context_item,
                TurnReferenceContextItem::Cleared
            ))
    {
        *reference_context_item = active_segment.reference_context_item;
    }
}

struct HistoryReplay {
    items: Vec<ResponseItem>,
}

impl HistoryReplay {
    fn new() -> Self {
        Self { items: Vec::new() }
    }

    fn record_item(&mut self, item: ResponseItem) {
        self.items.push(item);
    }

    fn raw_items(&self) -> &[ResponseItem] {
        &self.items
    }

    fn replace(&mut self, items: Vec<ResponseItem>) {
        self.items = items;
    }

    fn drop_last_n_user_turns(&mut self, num_turns: u64) {
        for _ in 0..num_turns {
            let Some(position) = self.items.iter().rposition(history_item_starts_user_turn) else {
                self.items.clear();
                return;
            };
            self.items.truncate(position);
        }
    }

    fn into_items(self) -> Vec<ResponseItem> {
        self.items
    }
}

fn history_item_starts_user_turn(item: &ResponseItem) -> bool {
    match item {
        ResponseItem::Message { role, content, .. } => {
            role == "user" || inter_agent_trigger_turn(content)
        }
        ResponseItem::AgentMessage { .. } => true,
        _ => false,
    }
}

fn inter_agent_trigger_turn(content: &[ContentItem]) -> bool {
    runtime_protocol::protocol::InterAgentCommunication::from_message_content(content)
        .is_some_and(|communication| communication.trigger_turn)
}

fn collect_user_messages(items: &[ResponseItem]) -> Vec<String> {
    items
        .iter()
        .filter_map(|item| match item {
            ResponseItem::Message { role, content, .. } if role == "user" => {
                Some(content_text(content))
            }
            _ => None,
        })
        .filter(|message| !message.trim().is_empty())
        .collect()
}

fn content_text(content: &[ContentItem]) -> String {
    content
        .iter()
        .filter_map(|item| match item {
            ContentItem::InputText { text } | ContentItem::OutputText { text } => {
                Some(text.as_str())
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_legacy_compacted_history(
    user_messages: &[String],
    summary_text: &str,
) -> Vec<ResponseItem> {
    let mut history = Vec::new();
    let mut selected_messages: Vec<String> = Vec::new();
    let mut remaining = COMPACT_USER_MESSAGE_MAX_TOKENS;
    for message in user_messages.iter().rev() {
        if remaining == 0 {
            break;
        }
        let tokens = approx_token_count(message);
        if tokens <= remaining {
            selected_messages.push(message.clone());
            remaining = remaining.saturating_sub(tokens);
        } else {
            selected_messages.push(truncate_text_by_approx_tokens(message, remaining));
            break;
        }
    }
    selected_messages.reverse();

    for message in selected_messages {
        history.push(user_response_message(message));
    }

    let summary_text = if summary_text.is_empty() {
        "(no summary available)".to_string()
    } else {
        summary_text.to_string()
    };
    history.push(user_response_message(summary_text));
    history
}

fn user_response_message(text: String) -> ResponseItem {
    ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText { text }],
        phase: None,
    }
}

fn approx_token_count(text: &str) -> usize {
    text.split_whitespace().count().max(1)
}

fn truncate_text_by_approx_tokens(text: &str, max_tokens: usize) -> String {
    if max_tokens == 0 {
        return String::new();
    }
    text.split_whitespace()
        .take(max_tokens)
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use runtime_protocol::models::ContentItem;
    use runtime_protocol::protocol::CompactedItem;
    use runtime_protocol::protocol::ThreadRolledBackEvent;
    use runtime_protocol::protocol::TurnCompleteEvent;
    use runtime_protocol::protocol::TurnStartedEvent;
    use runtime_protocol::protocol::UserMessageEvent;

    fn user_message(text: &str) -> ResponseItem {
        ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: text.to_string(),
            }],
            phase: None,
        }
    }

    fn assistant_message(text: &str) -> ResponseItem {
        ResponseItem::Message {
            id: None,
            role: "assistant".to_string(),
            content: vec![ContentItem::OutputText {
                text: text.to_string(),
            }],
            phase: None,
        }
    }

    fn turn_context(turn_id: &str, model: &str) -> TurnContextItem {
        TurnContextItem {
            turn_id: Some(turn_id.to_string()),
            cwd: std::path::PathBuf::from("/repo"),
            workspace_roots: None,
            current_date: Some("2026-06-09".to_string()),
            timezone: Some("UTC".to_string()),
            approval_policy: Default::default(),
            sandbox_policy: runtime_protocol::protocol::SandboxPolicy::ReadOnly {
                network_access: false,
            },
            permission_profile: None,
            network: None,
            file_system_sandbox_policy: None,
            model: model.to_string(),
            personality: None,
            collaboration_mode: None,
            multi_agent_version: None,
            realtime_active: None,
            effort: None,
            summary: runtime_protocol::config_types::ReasoningSummary::Auto,
        }
    }

    fn started(turn_id: &str) -> RolloutItem {
        RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
            turn_id: turn_id.to_string(),
            trace_id: None,
            started_at: None,
            model_context_window: Some(128_000),
            collaboration_mode_kind: Default::default(),
        }))
    }

    fn complete(turn_id: &str) -> RolloutItem {
        RolloutItem::EventMsg(EventMsg::TurnComplete(TurnCompleteEvent {
            turn_id: turn_id.to_string(),
            last_agent_message: None,
            completed_at: None,
            duration_ms: None,
            time_to_first_token_ms: None,
        }))
    }

    fn user_event(message: &str) -> RolloutItem {
        RolloutItem::EventMsg(EventMsg::UserMessage(UserMessageEvent {
            client_id: None,
            message: message.to_string(),
            images: None,
            local_images: Vec::new(),
            text_elements: Vec::new(),
            ..Default::default()
        }))
    }

    #[test]
    fn rollback_keeps_history_and_metadata_in_sync_for_completed_turns() {
        let first_context = turn_context("turn-1", "model-1");
        let second_context = turn_context("turn-2", "model-2");
        let turn_one_user = user_message("turn 1 user");
        let turn_one_assistant = assistant_message("turn 1 assistant");
        let turn_two_user = user_message("turn 2 user");
        let turn_two_assistant = assistant_message("turn 2 assistant");

        let rollout_items = vec![
            started("turn-1"),
            user_event("turn 1 user"),
            RolloutItem::TurnContext(first_context.clone()),
            RolloutItem::ResponseItem(turn_one_user.clone()),
            RolloutItem::ResponseItem(turn_one_assistant.clone()),
            complete("turn-1"),
            started("turn-2"),
            user_event("turn 2 user"),
            RolloutItem::TurnContext(second_context),
            RolloutItem::ResponseItem(turn_two_user),
            RolloutItem::ResponseItem(turn_two_assistant),
            complete("turn-2"),
            RolloutItem::EventMsg(EventMsg::ThreadRolledBack(ThreadRolledBackEvent {
                num_turns: 1,
            })),
        ];

        let reconstructed = reconstruct_history_from_rollout(&rollout_items);

        assert_eq!(
            reconstructed.history,
            vec![turn_one_user, turn_one_assistant]
        );
        assert_eq!(
            reconstructed.previous_turn_settings,
            Some(PreviousTurnSettings {
                model: "model-1".to_string(),
                realtime_active: None,
            })
        );
        assert_eq!(
            serde_json::to_value(reconstructed.reference_context_item)
                .expect("serialize reconstructed reference context"),
            serde_json::to_value(Some(first_context))
                .expect("serialize expected reference context")
        );
    }

    #[test]
    fn replacement_history_checkpoint_replays_only_surviving_tail() {
        let seed = user_message("seed");
        let after = assistant_message("after compact");
        let rollout_items = vec![
            started("turn-1"),
            user_event("seed"),
            RolloutItem::TurnContext(turn_context("turn-1", "old-model")),
            RolloutItem::ResponseItem(seed.clone()),
            RolloutItem::Compacted(CompactedItem {
                message: "summary".to_string(),
                replacement_history: Some(vec![assistant_message("summary")]),
            }),
            RolloutItem::ResponseItem(after.clone()),
            complete("turn-1"),
        ];

        let reconstructed = reconstruct_history_from_rollout(&rollout_items);

        assert_eq!(
            reconstructed.history,
            vec![assistant_message("summary"), after]
        );
        assert_eq!(
            reconstructed.previous_turn_settings,
            Some(PreviousTurnSettings {
                model: "old-model".to_string(),
                realtime_active: None,
            })
        );
        assert!(reconstructed.reference_context_item.is_none());
    }

    #[test]
    fn legacy_compaction_without_replacement_history_rebuilds_from_prior_user_messages() {
        let first_user = user_message("first user");
        let first_assistant = assistant_message("first assistant");
        let second_user = user_message("second user");
        let after = assistant_message("after compact");
        let rollout_items = vec![
            started("turn-1"),
            user_event("first user"),
            RolloutItem::TurnContext(turn_context("turn-1", "old-model")),
            RolloutItem::ResponseItem(first_user.clone()),
            RolloutItem::ResponseItem(first_assistant),
            complete("turn-1"),
            started("turn-2"),
            user_event("second user"),
            RolloutItem::ResponseItem(second_user.clone()),
            RolloutItem::Compacted(CompactedItem {
                message: "legacy summary".to_string(),
                replacement_history: None,
            }),
            RolloutItem::ResponseItem(after.clone()),
            complete("turn-2"),
        ];

        let reconstructed = reconstruct_history_from_rollout(&rollout_items);

        assert_eq!(
            reconstructed.history,
            vec![
                first_user,
                second_user,
                user_message("legacy summary"),
                after
            ]
        );
        assert!(reconstructed.reference_context_item.is_none());
    }

    #[test]
    fn bare_turn_context_does_not_hydrate_previous_turn_settings() {
        let rollout_items = vec![RolloutItem::TurnContext(turn_context("turn-1", "model-1"))];

        let reconstructed = reconstruct_history_from_rollout(&rollout_items);

        assert_eq!(reconstructed.previous_turn_settings, None);
        assert!(reconstructed.reference_context_item.is_none());
    }

    #[test]
    fn previous_turn_settings_keep_realtime_active_from_turn_context() {
        let mut context = turn_context("turn-1", "model-1");
        context.realtime_active = Some(true);
        let rollout_items = vec![
            started("turn-1"),
            user_event("turn 1 user"),
            RolloutItem::TurnContext(context),
            complete("turn-1"),
        ];

        let reconstructed = reconstruct_history_from_rollout(&rollout_items);

        assert_eq!(
            reconstructed.previous_turn_settings,
            Some(PreviousTurnSettings {
                model: "model-1".to_string(),
                realtime_active: Some(true),
            })
        );
    }
}
