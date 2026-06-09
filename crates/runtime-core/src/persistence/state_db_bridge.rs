pub use rollout::state_db::StateDbHandle;
pub use thread_store::LocalThreadStore;
pub use thread_store::LocalThreadStoreConfig;

pub async fn init_state_db(config: &config::RuntimeConfig) -> Option<StateDbHandle> {
    rollout::state_db::init(config).await
}

pub async fn init_local_thread_store(config: &config::RuntimeConfig) -> LocalThreadStore {
    let state_db = init_state_db(config).await;
    LocalThreadStore::new(LocalThreadStoreConfig::from_config(config), state_db)
}
