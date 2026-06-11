//! Skills subsystem test scenarios
use crate::TestContext;
use serde_json::json;

#[tokio::test]
async fn test_skill_noop() {
    let mut ctx = TestContext::new();
    ctx.bootstrap().await.unwrap();

    let result = ctx.skills.call("noop", &json!({}));
    assert!(matches!(result, bot::hy_evo::node::NodeResult::Value(_)));

    ctx.shutdown().await;
}

#[tokio::test]
async fn test_skill_list_tools() {
    let mut ctx = TestContext::new();
    ctx.bootstrap().await.unwrap();

    let result = ctx.skills.call("list_tools", &json!({}));
    // Should return text (even if empty)
    assert!(matches!(result, bot::hy_evo::node::NodeResult::Text(_)) ||
            matches!(result, bot::hy_evo::node::NodeResult::Value(_)));

    ctx.shutdown().await;
}
