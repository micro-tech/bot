//! Memory subsystem test scenarios
use crate::TestContext;
use bot::cpu::interfaces::MemoryInterface;
use serde_json::json;

#[tokio::test]
async fn test_memory_write_read_context() {
    let mut ctx = TestContext::new();
    ctx.bootstrap().await.unwrap();

    {
        let _ = ctx.memory.write("context", json!("hello world"));
        let read = ctx.memory.read("context");
        assert!(matches!(read, bot::hy_evo::node::NodeResult::Value(_)));
    }

    ctx.shutdown().await;
}

#[tokio::test]
async fn test_memory_belief_set_get() {
    let mut ctx = TestContext::new();
    ctx.bootstrap().await.unwrap();

    ctx.memory.set_belief("test_key", json!("value123"));
    let val = ctx.memory.get_belief("test_key");
    assert!(val.is_some());

    ctx.shutdown().await;
}
