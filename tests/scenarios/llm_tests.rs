//! LLM / Ollama / Gemini integration scenarios (mocked)
use crate::TestContext;

#[tokio::test]
async fn test_llm_chat_request_flow() {
    let mut ctx = TestContext::new();
    ctx.bootstrap().await.unwrap();

    // Simulate publishing a chat_request that would normally go to Ollama/Gemini
    let msg = bot::bus::Message {
        to: "ollama_local".into(),
        from: "harness".into(),
        data: r#"{"type":"chat_request","prompt":"hello"}"#.into(),
        timestamp: bot::utils::now_ms(),
    };
    let _ = ctx.bus.publish(msg);

    tokio::time::sleep(std::time::Duration::from_millis(30)).await;

    // In real harness we would have a mock LLM listener that replies.
    // Here we just verify the message was published.
    let m = ctx.metrics.lock().unwrap().clone();
    assert!(m.messages_published >= 1);

    ctx.shutdown().await;
}
