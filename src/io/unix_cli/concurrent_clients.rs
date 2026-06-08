//! Concurrent client handling (Task 81)
use std::sync::atomic::{AtomicUsize, Ordering};

static ACTIVE_CLIENTS: AtomicUsize = AtomicUsize::new(0);

pub fn client_connected() {
    ACTIVE_CLIENTS.fetch_add(1, Ordering::SeqCst);
}

pub fn client_disconnected() {
    ACTIVE_CLIENTS.fetch_sub(1, Ordering::SeqCst);
}

pub fn active_count() -> usize {
    ACTIVE_CLIENTS.load(Ordering::SeqCst)
}
