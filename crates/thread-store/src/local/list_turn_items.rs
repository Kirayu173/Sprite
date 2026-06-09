use runtime_protocol::protocol::EventMsg;
use runtime_protocol::protocol::RolloutItem;

use crate::ItemPage;
use crate::ListItemsParams;
use crate::ListTurnsParams;
use crate::LoadThreadHistoryParams;
use crate::SortDirection;
use crate::StoredTurn;
use crate::StoredTurnError;
use crate::StoredTurnItemsView;
use crate::StoredTurnStatus;
use crate::ThreadStore;
use crate::ThreadStoreError;
use crate::ThreadStoreResult;
use crate::TurnPage;
use crate::local::LocalThreadStore;

pub(super) async fn list_turns(
    store: &LocalThreadStore,
    params: ListTurnsParams,
) -> ThreadStoreResult<TurnPage> {
    let history = store
        .load_history(LoadThreadHistoryParams {
            thread_id: params.thread_id,
            include_archived: params.include_archived,
        })
        .await?;
    let mut turns = build_stored_turns(&history.items, params.items_view);
    if params.sort_direction == SortDirection::Desc {
        turns.reverse();
    }

    let (start, end, next_cursor, backwards_cursor) =
        page_bounds(turns.len(), params.cursor.as_deref(), params.page_size)?;
    Ok(TurnPage {
        turns: turns[start..end].to_vec(),
        next_cursor,
        backwards_cursor,
    })
}

pub(super) async fn list_items(
    store: &LocalThreadStore,
    params: ListItemsParams,
) -> ThreadStoreResult<ItemPage> {
    let history = store
        .load_history(LoadThreadHistoryParams {
            thread_id: params.thread_id,
            include_archived: params.include_archived,
        })
        .await?;
    let mut turns = build_stored_turns(&history.items, StoredTurnItemsView::Full);
    if params.sort_direction == SortDirection::Desc {
        turns.reverse();
    }
    let turn = turns
        .into_iter()
        .find(|turn| turn.turn_id == params.turn_id)
        .ok_or_else(|| ThreadStoreError::InvalidRequest {
            message: format!(
                "turn {} not found in thread {}",
                params.turn_id, params.thread_id
            ),
        })?;

    let (start, end, next_cursor, backwards_cursor) =
        page_bounds(turn.items.len(), params.cursor.as_deref(), params.page_size)?;
    Ok(ItemPage {
        items: turn.items[start..end].to_vec(),
        next_cursor,
        backwards_cursor,
    })
}

fn build_stored_turns(items: &[RolloutItem], items_view: StoredTurnItemsView) -> Vec<StoredTurn> {
    let mut builder = StoredTurnBuilder::new(items_view);
    for (idx, item) in items.iter().enumerate() {
        builder.handle_item(idx, item);
    }
    builder.finish()
}

fn page_bounds(
    len: usize,
    cursor: Option<&str>,
    page_size: usize,
) -> ThreadStoreResult<(usize, usize, Option<String>, Option<String>)> {
    let start = match cursor {
        Some(cursor) => cursor
            .parse::<usize>()
            .map_err(|_| ThreadStoreError::InvalidRequest {
                message: format!("invalid pagination cursor `{cursor}`"),
            })?
            .min(len),
        None => 0,
    };
    let size = page_size.max(1);
    let end = start.saturating_add(size).min(len);
    let next_cursor = (end < len).then(|| end.to_string());
    let backwards_cursor = (start > 0).then(|| start.saturating_sub(size).to_string());
    Ok((start, end, next_cursor, backwards_cursor))
}

struct StoredTurnBuilder {
    turns: Vec<PendingStoredTurn>,
    current_turn: Option<PendingStoredTurn>,
    items_view: StoredTurnItemsView,
}

impl StoredTurnBuilder {
    fn new(items_view: StoredTurnItemsView) -> Self {
        Self {
            turns: Vec::new(),
            current_turn: None,
            items_view,
        }
    }

    fn handle_item(&mut self, idx: usize, item: &RolloutItem) {
        match item {
            RolloutItem::EventMsg(EventMsg::TurnStarted(event)) => {
                self.finish_current_turn();
                self.current_turn = Some(PendingStoredTurn {
                    turn_id: event.turn_id.clone(),
                    items: Vec::new(),
                    items_view: self.items_view,
                    status: StoredTurnStatus::InProgress,
                    error: None,
                    started_at: event.started_at,
                    completed_at: None,
                    duration_ms: None,
                    opened_explicitly: true,
                    saw_compaction: false,
                });
                self.push_current_item(item);
            }
            RolloutItem::EventMsg(EventMsg::TurnComplete(event)) => {
                if let Some(turn) = self
                    .current_turn
                    .as_mut()
                    .filter(|turn| turn.turn_id == event.turn_id)
                {
                    if matches!(
                        turn.status,
                        StoredTurnStatus::Completed | StoredTurnStatus::InProgress
                    ) {
                        turn.status = StoredTurnStatus::Completed;
                    }
                    turn.completed_at = event.completed_at;
                    turn.duration_ms = event.duration_ms;
                    self.push_current_item(item);
                    self.finish_current_turn();
                } else if let Some(turn) = self
                    .turns
                    .iter_mut()
                    .find(|turn| turn.turn_id == event.turn_id)
                {
                    if matches!(
                        turn.status,
                        StoredTurnStatus::Completed | StoredTurnStatus::InProgress
                    ) {
                        turn.status = StoredTurnStatus::Completed;
                    }
                    turn.completed_at = event.completed_at;
                    turn.duration_ms = event.duration_ms;
                    push_item_for_view(turn, self.items_view, item);
                }
            }
            RolloutItem::EventMsg(EventMsg::TurnAborted(event)) => {
                if let Some(turn_id) = event.turn_id.as_deref() {
                    if let Some(turn) = self
                        .current_turn
                        .as_mut()
                        .filter(|turn| turn.turn_id == turn_id)
                    {
                        turn.status = StoredTurnStatus::Interrupted;
                        turn.completed_at = event.completed_at;
                        turn.duration_ms = event.duration_ms;
                        self.push_current_item(item);
                        return;
                    }
                    if let Some(turn) = self.turns.iter_mut().find(|turn| turn.turn_id == turn_id) {
                        turn.status = StoredTurnStatus::Interrupted;
                        turn.completed_at = event.completed_at;
                        turn.duration_ms = event.duration_ms;
                        push_item_for_view(turn, self.items_view, item);
                        return;
                    }
                }
                let turn = self.ensure_turn(idx);
                turn.status = StoredTurnStatus::Interrupted;
                turn.completed_at = event.completed_at;
                turn.duration_ms = event.duration_ms;
                self.push_current_item(item);
            }
            RolloutItem::EventMsg(EventMsg::Error(event)) if event.affects_turn_status() => {
                let turn = self.ensure_turn(idx);
                turn.status = StoredTurnStatus::Failed;
                turn.error = Some(StoredTurnError {
                    message: event.message.clone(),
                    additional_details: None,
                });
                self.push_current_item(item);
            }
            RolloutItem::EventMsg(EventMsg::ThreadRolledBack(event)) => {
                self.finish_current_turn();
                let n = usize::try_from(event.num_turns).unwrap_or(usize::MAX);
                if n >= self.turns.len() {
                    self.turns.clear();
                } else {
                    self.turns.truncate(self.turns.len().saturating_sub(n));
                }
            }
            RolloutItem::EventMsg(EventMsg::UserMessage(_)) => {
                self.ensure_turn(idx);
                self.push_current_item(item);
            }
            RolloutItem::Compacted(_) => {
                self.ensure_turn(idx).saw_compaction = true;
                self.push_current_item(item);
            }
            RolloutItem::ResponseItem(response_item) => {
                if response_item.is_user_message()
                    && self
                        .current_turn
                        .as_ref()
                        .is_some_and(|turn| !turn.opened_explicitly)
                {
                    self.finish_current_turn();
                }
                self.ensure_turn(idx);
                self.push_current_item(item);
            }
            RolloutItem::EventMsg(_)
            | RolloutItem::TurnContext(_)
            | RolloutItem::SessionMeta(_) => {
                if self.current_turn.is_some() {
                    self.push_current_item(item);
                }
            }
        }
    }

    fn finish(mut self) -> Vec<StoredTurn> {
        self.finish_current_turn();
        self.turns.into_iter().map(StoredTurn::from).collect()
    }

    fn finish_current_turn(&mut self) {
        let Some(turn) = self.current_turn.take() else {
            return;
        };
        if turn.items.is_empty() && !turn.opened_explicitly && !turn.saw_compaction {
            return;
        }
        self.turns.push(turn);
    }

    fn ensure_turn(&mut self, idx: usize) -> &mut PendingStoredTurn {
        if self.current_turn.is_none() {
            self.current_turn = Some(PendingStoredTurn {
                turn_id: format!("rollout-{idx}"),
                items: Vec::new(),
                items_view: self.items_view,
                status: StoredTurnStatus::Completed,
                error: None,
                started_at: None,
                completed_at: None,
                duration_ms: None,
                opened_explicitly: false,
                saw_compaction: false,
            });
        }
        self.current_turn.as_mut().expect("current turn exists")
    }

    fn push_current_item(&mut self, item: &RolloutItem) {
        let items_view = self.items_view;
        if let Some(turn) = self.current_turn.as_mut() {
            push_item_for_view(turn, items_view, item);
        }
    }
}

fn push_item_for_view(
    turn: &mut PendingStoredTurn,
    items_view: StoredTurnItemsView,
    item: &RolloutItem,
) {
    match items_view {
        StoredTurnItemsView::NotLoaded => {}
        StoredTurnItemsView::Summary | StoredTurnItemsView::Full => turn.items.push(item.clone()),
    }
}

struct PendingStoredTurn {
    turn_id: String,
    items: Vec<RolloutItem>,
    items_view: StoredTurnItemsView,
    status: StoredTurnStatus,
    error: Option<StoredTurnError>,
    started_at: Option<i64>,
    completed_at: Option<i64>,
    duration_ms: Option<i64>,
    opened_explicitly: bool,
    saw_compaction: bool,
}

impl From<PendingStoredTurn> for StoredTurn {
    fn from(value: PendingStoredTurn) -> Self {
        Self {
            turn_id: value.turn_id,
            items: value.items,
            items_view: value.items_view,
            status: value.status,
            error: value.error,
            started_at: value.started_at,
            completed_at: value.completed_at,
            duration_ms: value.duration_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    use runtime_protocol::protocol::ThreadRolledBackEvent;
    use runtime_protocol::protocol::TurnCompleteEvent;
    use runtime_protocol::protocol::TurnStartedEvent;
    use runtime_protocol::protocol::UserMessageEvent;

    use super::*;

    fn started(turn_id: &str) -> RolloutItem {
        RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
            turn_id: turn_id.to_string(),
            trace_id: None,
            started_at: Some(10),
            model_context_window: None,
            collaboration_mode_kind: Default::default(),
        }))
    }

    fn completed(turn_id: &str) -> RolloutItem {
        RolloutItem::EventMsg(EventMsg::TurnComplete(TurnCompleteEvent {
            turn_id: turn_id.to_string(),
            last_agent_message: None,
            completed_at: Some(20),
            duration_ms: Some(10_000),
            time_to_first_token_ms: None,
        }))
    }

    fn user(message: &str) -> RolloutItem {
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
    fn groups_explicit_turn_items_and_applies_rollback() {
        let items = vec![
            started("a"),
            user("one"),
            completed("a"),
            started("b"),
            user("two"),
            completed("b"),
            RolloutItem::EventMsg(EventMsg::ThreadRolledBack(ThreadRolledBackEvent {
                num_turns: 1,
            })),
        ];

        let turns = build_stored_turns(&items, StoredTurnItemsView::Full);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].turn_id, "a");
        assert_eq!(turns[0].status, StoredTurnStatus::Completed);
        assert_eq!(turns[0].items.len(), 3);
    }

    #[test]
    fn not_loaded_omits_items() {
        let items = vec![started("a"), user("one"), completed("a")];

        let turns = build_stored_turns(&items, StoredTurnItemsView::NotLoaded);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items_view, StoredTurnItemsView::NotLoaded);
        assert!(turns[0].items.is_empty());
    }
}
