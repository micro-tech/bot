//! Error path & recovery scenarios
use crate::TestContext;
use bot::bus::Message;

#[tokio::test]
async fn test_bus_invalid_json_handling() {
    let mut ctx = TestContext::new();
    ctx.bootstrap().await.unwrap();

    let bad_msg = Message {
        to: "test_harness".into(),
        from: "harness".into(),
        data: "not valid json {".into(),
        timestamp: bot::utils::now_ms(),
    };
    let _ = ctx.bus.publish(bad_msg);

    tokio::time::sleep(std::time::Duration::from_millis(30)).await;

    // Harness should not panic; we just check it kept running
    let m = ctx.metrics.lock().unwrap().clone();
    assert!(m.messages_published >= 1);

    ctx.shutdown().await;
}

#[tokio::test]
async fn test_skill_unknown_name() {
    let mut ctx = TestContext::new();
    ctx.bootstrap().await.unwrap();

    let result = ctx.skills.call("nonexistent_skill_xyz", &serde_json::json!({}));
    assert!(matches!(result, bot::hy_evo::node::NodeResult::Error(_)));

    ctx.shutdown().await;
}
