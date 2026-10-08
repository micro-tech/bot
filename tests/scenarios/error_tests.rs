//! Error path & recovery scenarios
use crate::TestContext;

#[tokio::test]
async fn test_bus_invalid_json_handling() {
    let mut ctx = TestContext::new();
    ctx.bootstrap().await.unwrap();

    ctx.publish_and_wait("test_harness", "not valid json {", 30)
        .await
        .unwrap();

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
    assert!(matches!(result, helix::hy_evo::node::NodeResult::Error(_)));

    ctx.shutdown().await;
}
