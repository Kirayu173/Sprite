use futures::StreamExt;
use model_runtime::ModelRuntime;
use model_runtime::ModelStreamEvent;
use model_runtime::ModelTurnRequest;
use provider_lmstudio::LMStudioRuntime;
use runtime_protocol::models::ContentItem;
use runtime_protocol::models::ResponseInputItem;

fn env_var(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

#[tokio::test]
#[ignore = "requires a live LM Studio server"]
async fn real_lmstudio_streams_text() {
    let base_url = env_var("SPRITE_LMSTUDIO_BASE_URL")
        .unwrap_or_else(|| "http://localhost:1234/v1".to_string());
    let model = env_var("SPRITE_LMSTUDIO_MODEL")
        .unwrap_or_else(|| model_provider_info::DEFAULT_LMSTUDIO_MODEL.to_string());

    let provider = model_provider_info::create_local_provider_with_base_url(
        "LM Studio",
        &base_url,
        model_provider_info::WireApi::Responses,
    );
    let runtime = LMStudioRuntime::new(provider.clone()).expect("runtime");
    let mut request = ModelTurnRequest::new(provider, model);
    request.messages.push(ResponseInputItem::Message {
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "Reply with the single word: pong".to_string(),
        }],
        phase: None,
    });

    let mut stream = runtime.stream_turn(request).await.expect("stream");
    let mut text = String::new();
    while let Some(event) = stream.next().await {
        match event.expect("event") {
            ModelStreamEvent::TextDelta { text: delta } => text.push_str(&delta),
            ModelStreamEvent::Completed { .. } => break,
            _ => {}
        }
    }

    assert!(!text.trim().is_empty(), "stream produced no assistant text");
}
