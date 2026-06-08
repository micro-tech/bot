//! Distributed Tracing (Task 47)

use std::collections::HashMap;

pub struct TraceSpan {
    pub trace_id: String,
    pub span_id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub attributes: HashMap<String, String>,
}

pub struct Tracer {
    spans: Vec<TraceSpan>,
}

impl Tracer {
    pub fn new() -> Self {
        Self { spans: Vec::new() }
    }

    pub fn start_span(&mut self, name: &str, parent: Option<String>) -> String {
        let span_id = uuid::Uuid::new_v4().to_string();
        let trace_id = parent.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        self.spans.push(TraceSpan {
            trace_id: trace_id.clone(),
            span_id: span_id.clone(),
            parent_id: parent,
            name: name.to_string(),
            attributes: HashMap::new(),
        });

        span_id
    }

    pub fn add_attribute(&mut self, span_id: &str, key: &str, value: &str) {
        if let Some(span) = self.spans.iter_mut().find(|s| s.span_id == span_id) {
            span.attributes.insert(key.to_string(), value.to_string());
        }
    }
}
