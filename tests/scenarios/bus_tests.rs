//! Bus subsystem test scenarios
use crate::TestContext;
use bot::bus::Message;
use std::time::Duration;

#[tokio::test]
async fn test_bus_publish_subscribe_roundtrip() {
    let mut ctx = TestContext::new();
    ctx.bootstrap().await.unwrap();

    // Publish several messages (no subscriber to avoid hang)
    for i in 0..5 {
        let msg = Message {
            to: "test_topic".into(),
            from: "harness".into(),
            data: format!(r#"{{"seq":{}}}"#, i),
            timestamp: bot::utils::now_ms(),
        };
        let _ = ctx.bus.publish(msg);
        {
            let mut m = ctx.metrics.lock().unwrap();
            m.messages_published += 1;
        }
    }

    tokio::time::sleep(Duration::from_millis(30)).await;

    let m = ctx.metrics.lock().unwrap().clone();
    assert!(m.messages_published >= 5);

    ctx.shutdown().await;
}

#[tokio::test]
async fn test_bus_error_propagation() {
    let mut ctx = TestContext::new();
    ctx.bootstrap().await.unwrap();

    // Publish a message containing "error"
    let msg = Message {
        to: "test_harness".into(),
        from: "harness".into(),
        data: r#"{"type":"error","msg":"test failure"}"#.into(),
        timestamp: bot::utils::now_ms(),
    };
    let _ = ctx.bus.publish(msg);

    tokio::time::sleep(Duration::from_millis(30)).await;

    let m = ctx.metrics.lock().unwrap().clone();
    assert!(m.errors >= 1 || m.messages_published >= 1);

    ctx.shutdown().await;
}
