use futures::StreamExt;
use model_provider_info::ModelProviderInfo;
use model_runtime::ModelRuntime;
use model_runtime::ModelStreamEvent;
use model_runtime::ModelTurnRequest;
use provider_openai_compatible::OpenAiCompatibleRuntime;
use runtime_protocol::models::ContentItem;
use runtime_protocol::models::ResponseInputItem;

fn env_var(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

#[tokio::test]
#[ignore = "requires live openai-compatible credentials"]
async fn real_openai_compatible_streams_text() {
    let base_url = env_var("SPRITE_OAI_BASE_URL").expect("SPRITE_OAI_BASE_URL");
    let model = env_var("SPRITE_OAI_MODEL").expect("SPRITE_OAI_MODEL");
    let api_key = env_var("SPRITE_OAI_API_KEY");

    let mut provider = ModelProviderInfo::create_openai_compatible_provider(Some(base_url));
    provider.experimental_bearer_token = api_key;

    let runtime = OpenAiCompatibleRuntime::new(provider.clone()).expect("runtime");
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
