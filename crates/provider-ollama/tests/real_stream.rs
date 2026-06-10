use futures::StreamExt;
use model_runtime::ModelRuntime;
use model_runtime::ModelStreamEvent;
use model_runtime::ModelTurnRequest;
use provider_ollama::OllamaRuntime;
use runtime_protocol::models::ContentItem;
use runtime_protocol::models::ResponseInputItem;

fn env_var(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

#[tokio::test]
#[ignore = "requires a live Ollama server"]
async fn real_ollama_streams_text() {
    let base_url = env_var("SPRITE_OLLAMA_BASE_URL")
        .unwrap_or_else(|| "http://localhost:11434/v1".to_string());
    let model = env_var("SPRITE_OLLAMA_MODEL")
        .unwrap_or_else(|| model_provider_info::DEFAULT_OLLAMA_MODEL.to_string());

    let provider = model_provider_info::create_local_provider_with_base_url(
        "Ollama",
        &base_url,
        model_provider_info::WireApi::Responses,
    );
    let runtime = OllamaRuntime::new(provider.clone()).expect("runtime");
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
