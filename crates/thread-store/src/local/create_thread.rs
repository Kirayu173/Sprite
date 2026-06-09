use super::LocalThreadStore;
use crate::CreateThreadParams;
use crate::ThreadStoreError;
use crate::ThreadStoreResult;
use rollout::RolloutConfig;
use rollout::RolloutRecorder;
use rollout::RolloutRecorderParams;
use runtime_protocol::protocol::ThreadMemoryMode;

pub(super) async fn create_thread(
    store: &LocalThreadStore,
    params: CreateThreadParams,
) -> ThreadStoreResult<RolloutRecorder> {
    let cwd = params
        .metadata
        .cwd
        .clone()
        .ok_or_else(|| ThreadStoreError::InvalidRequest {
            message: "local thread store requires a cwd".to_string(),
        })?;
    let config = RolloutConfig {
        sprite_home: store.config.sprite_home.clone(),
        sqlite_home: store.config.sqlite_home.clone(),
        cwd,
        model_provider_id: params.metadata.model_provider.clone(),
        generate_memories: matches!(params.metadata.memory_mode, ThreadMemoryMode::Enabled),
    };
    let recorder = RolloutRecorder::new(
        &config,
        RolloutRecorderParams::new(
            params.thread_id,
            params.forked_from_id,
            params.parent_thread_id,
            params.source,
            params.thread_source,
            params.base_instructions,
            params.dynamic_tools,
        )
        .with_multi_agent_version(params.multi_agent_version),
    )
    .await
    .map_err(|err| ThreadStoreError::Internal {
        message: format!("failed to initialize local thread recorder: {err}"),
    })?;

    Ok(recorder)
}
