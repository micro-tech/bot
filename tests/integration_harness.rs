//! Integration Test Harness for Bot (Task 122)
//!
//! Fully automated end-to-end test suite that exercises all major subsystems.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use bot::bus::{Bus, Message};
use bot::memory::MemoryManager;
use bot::skills::SkillRegistry;

pub mod scenarios;

/// Test context holding all running components and handles.
pub struct TestContext {
    pub bus: Arc<Bus>,
    pub memory: MemoryManager,
    pub skills: SkillRegistry,
    pub handles: Vec<tokio::task::JoinHandle<()>>,
    pub metrics: std::sync::Arc<std::sync::Mutex<TestMetrics>>,
}

#[derive(Default, Clone, Debug)]
pub struct TestMetrics {
    pub messages_published: u64,
    pub messages_received: u64,
    pub errors: u64,
    pub workflows_executed: u64,
    pub llm_calls: u64,
}

impl TestContext {
    /// Create a fresh test context with a new Bus.
    pub fn new() -> Self {
        let bus = Arc::new(Bus::new());
        Self {
            bus,
            memory: MemoryManager::new(100, 1000),
            skills: SkillRegistry::new(),
            handles: Vec::new(),
            metrics: Arc::new(std::sync::Mutex::new(TestMetrics::default())),
        }
    }

    /// Bootstrap core modules (bus already created).
    pub async fn bootstrap(&mut self) -> anyhow::Result<()> {
        println!("[Harness] Bootstrapping test environment...");

        // Spawn a simple message counter on the bus
        let bus_clone = self.bus.clone();
        let metrics_clone = self.metrics.clone();
        let handle = tokio::spawn(async move {
            let rx = bus_clone.subscribe("test_harness");
            while let Ok(msg) = rx.recv() {
                let mut m = metrics_clone.lock().unwrap();
                m.messages_received += 1;
                if msg.data.contains("error") {
                    m.errors += 1;
                }
            }
        });
        self.handles.push(handle);

        // TODO: Start CPU executor, memory background tasks, HyEvo, etc.
        // For now we keep the skeleton minimal.

        println!("[Harness] Bootstrap complete.");
        Ok(())
    }

    /// Publish a test message and wait for delivery.
    pub async fn publish_and_wait(&self, to: &str, data: &str, wait_ms: u64) -> anyhow::Result<()> {
        let msg = Message {
            to: to.to_string(),
            from: "test_harness".to_string(),
            data: data.to_string(),
            timestamp: bot::utils::now_ms(),
        };

        self.bus.publish(msg).map_err(|e| anyhow::anyhow!(e))?;
        {
            let mut m = self.metrics.lock().unwrap();
            m.messages_published += 1;
        }

        // Give router time to deliver
        tokio::time::sleep(Duration::from_millis(wait_ms)).await;
        Ok(())
    }

    /// Shutdown all background tasks.
    pub async fn shutdown(self) {
        println!("[Harness] Shutting down...");
        for handle in self.handles {
            handle.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_bootstrap_and_basic_messaging() {
        let mut ctx = TestContext::new();
        ctx.bootstrap().await.expect("bootstrap failed");

        // Publish a message to a non-existent topic (should be tolerated)
        ctx.publish_and_wait("nonexistent", r#"{"type":"ping"}"#, 50)
            .await
            .expect("publish failed");

        // Publish to the harness listener
        ctx.publish_and_wait("test_harness", r#"{"type":"test"}"#, 50)
            .await
            .expect("publish failed");

        let metrics = ctx.metrics.lock().unwrap().clone();
        assert!(metrics.messages_published >= 2);
        assert!(metrics.messages_received >= 1);

        ctx.shutdown().await;
    }
}
