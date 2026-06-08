//! Prometheus Metrics (Task 48)

use std::collections::HashMap;

pub struct PrometheusMetrics {
    counters: HashMap<String, u64>,
    gauges: HashMap<String, f64>,
}

impl PrometheusMetrics {
    pub fn new() -> Self {
        Self {
            counters: HashMap::new(),
            gauges: HashMap::new(),
        }
    }

    pub fn inc(&mut self, name: &str) {
        *self.counters.entry(name.to_string()).or_insert(0) += 1;
    }

    pub fn set_gauge(&mut self, name: &str, value: f64) {
        self.gauges.insert(name.to_string(), value);
    }

    pub fn export(&self) -> String {
        let mut output = String::new();
        for (k, v) in &self.counters {
            output.push_str(&format!("# TYPE {} counter\n{} {}\n", k, k, v));
        }
        for (k, v) in &self.gauges {
            output.push_str(&format!("# TYPE {} gauge\n{} {}\n", k, k, v));
        }
        output
    }
}
