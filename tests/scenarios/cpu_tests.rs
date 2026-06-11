//! CPU / Workflow executor test scenarios (lightweight)
use crate::TestContext;
use bot::cpu::interfaces::MemoryInterface;
use serde_json::json;

#[tokio::test]
async fn test_cpu_instruction_write_memory() {
    let mut ctx = TestContext::new();
    ctx.bootstrap().await.unwrap();

    {
        let _ = ctx.memory.write("context", json!("cpu_test"));
    }

    assert_eq!(ctx.memory.working.context.len(), 1);
    ctx.shutdown().await;
}
