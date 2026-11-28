//! Bus subsystem test scenarios
use crate::TestContext;
use helix::bus::Message;
use std::time::Duration;

#[tokio::test]
async fn test_bus_publish_subscribe_roundtrip() {
    let mut ctx = TestContext::new();
    ctx.bootstrap().await.unwrap();

    // M25: subscribe and wait for delivery with a timeout, instead of
    // sleeping a fixed 30ms and hoping the router delivered.
    let mut rx = ctx.bus.subscribe("test_topic");

    // Publish several messages
    for i in 0..5 {
        let msg = Message {
            to: "test_topic".into(),
            from: "harness".into(),
            data: format!(r#"{{"seq":{}}}"#, i),
            timestamp: helix::utils::now_ms(),
        };
        let _ = ctx.bus.publish(msg);
        {
            let mut m = ctx.metrics.lock().unwrap();
            m.messages_published += 1;
        }
    }

    // All five must arrive, in order, within the deadline.
    for i in 0..5 {
        let msg = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("timed out waiting for bus delivery")
            .expect("bus closed");
        assert!(
            msg.data.contains(&format!(r#""seq":{}"#, i)),
            "expected seq {} in order, got: {}",
            i,
            msg.data
        );
    }

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
        timestamp: helix::utils::now_ms(),
    };
    let _ = ctx.bus.publish(msg);

    // M25: poll until the listener counts the error, with a deadline,
    // instead of sleeping a fixed 30ms and hoping the listener ran.
    let mut errors = 0;
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        errors = ctx.metrics.lock().unwrap().errors;
        if errors >= 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(errors >= 1, "error message was not counted by the listener");

    ctx.shutdown().await;
}
