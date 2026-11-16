//! Thin WebDriver BiDi seam (task 351).
//!
//! **ALL** thirtyfour BiDi types stay behind this module. The rest of the
//! browser code talks to [`BidiSeam`], never to `thirtyfour::bidi::*` — so
//! spec and Firefox updates are cheap to absorb (the evolution plan in
//! task 359: pin Firefox, keep this seam thin, track the spec).
//!
//! BiDi is the W3C standard and Firefox-native (CDP is gone in Fx 141+).
//! Its event stream (navigation, dialogs, console, network pushed to the
//! client) fits an agent driving a browser better than polling.
//!
//! Fallback rule (task 352): if a BiDi command fails because geckodriver
//! does not implement it yet, the caller falls back to the classic
//! WebDriver equivalent and logs it — the seam itself only reports the
//! failure, it never silently degrades.
//!
//! Single-session rule: Firefox allows exactly ONE BiDi session and it
//! outlives its WebSocket — always call [`BidiSeam::end`] on shutdown; a
//! refused session on startup means restart Firefox, don't retry the
//! same session.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use log::{debug, warn};
use thirtyfour::bidi::BiDi;
use thirtyfour::bidi::events::{Load, NavigationCommitted, NavigationFailed};
use thirtyfour::bidi::modules::browsing_context::ReadinessState;
use thirtyfour::prelude::*;

/// A navigation/load event pushed by the browser over BiDi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavEventKind {
    Committed,
    Load,
    Failed,
}

/// One recorded navigation event: its kind, the URL, and when it arrived.
#[derive(Debug, Clone)]
pub struct NavEvent {
    pub kind: NavEventKind,
    pub url: String,
    pub at: Instant,
}

/// Shared last-navigation-event state, fed by the BiDi event-drain task.
///
/// The record/wait logic is pure and unit-testable without a browser: a
/// test feeds events and asserts `wait_for_load` returns (or times out).
/// The real drain task only calls [`NavEvents::record`].
pub struct NavEvents {
    state: Mutex<Option<NavEvent>>,
    notify: tokio::sync::Notify,
}

impl Default for NavEvents {
    fn default() -> Self {
        Self {
            state: Mutex::new(None),
            notify: tokio::sync::Notify::new(),
        }
    }
}

impl NavEvents {
    /// Record an event (from the drain task, or a test) and wake waiters.
    pub fn record(&self, kind: NavEventKind, url: String) {
        let event = NavEvent {
            kind,
            url,
            at: Instant::now(),
        };
        if let Ok(mut guard) = self.state.lock() {
            *guard = Some(event);
        }
        self.notify.notify_waiters();
    }

    /// The most recent navigation event, if any.
    pub fn last(&self) -> Option<NavEvent> {
        self.state.lock().ok().and_then(|g| g.clone())
    }

    /// Wait for a `Load` event at or after `since`, up to `timeout`.
    /// Returns true when one arrived, false on timeout.
    pub async fn wait_for_load(&self, since: Instant, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(ev) = self.last() {
                if ev.kind == NavEventKind::Load && ev.at >= since {
                    return true;
                }
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            // Wait for the next event (or the deadline), then re-check.
            let notified = self.notify.notified();
            tokio::pin!(notified);
            match tokio::time::timeout(remaining, notified).await {
                Ok(()) => continue,
                Err(_) => return false,
            }
        }
    }
}

/// The BiDi seam: one connected BiDi handle plus the event-drain task.
///
/// Created by [`BidiSeam::connect`]; the drain task is aborted by
/// [`BidiSeam::end`]. Never construct the thirtyfour `BiDi` handle
/// outside this module.
pub struct BidiSeam {
    bidi: BiDi,
    events: Arc<NavEvents>,
    drain: Option<tokio::task::JoinHandle<()>>,
}

impl BidiSeam {
    /// Connect the BiDi handle for `driver` and subscribe to navigation
    /// events. Requires `webSocketUrl: true` on the capabilities (set by
    /// `caps.enable_bidi()` before the session started); without it the
    /// driver returns no BiDi URL and this fails.
    pub async fn connect(driver: &WebDriver) -> Result<Self, String> {
        let bidi = driver
            .bidi()
            .await
            .map_err(|e| format!("BiDi connect failed: {e}"))?;
        let ready = bidi
            .session()
            .status()
            .await
            .map_err(|e| format!("BiDi session status failed: {e}"))?;
        if !ready.ready {
            return Err("BiDi session reports not ready".to_string());
        }
        debug!("browser: BiDi session ready (build {})", ready.message);
        let events = Arc::new(NavEvents::default());
        let drain = Self::spawn_drain(bidi.clone(), Arc::clone(&events)).await?;
        Ok(Self {
            bidi,
            events,
            drain: Some(drain),
        })
    }

    /// Subscribe to the navigation event streams and spawn the task that
    /// drains them into `events`. A subscription failure is fatal to the
    /// seam (the session is BiDi-first; the caller falls back to classic
    /// WebDriver without a seam at all).
    async fn spawn_drain(
        bidi: BiDi,
        events: Arc<NavEvents>,
    ) -> Result<tokio::task::JoinHandle<()>, String> {
        let mut committed = bidi
            .subscribe::<NavigationCommitted>()
            .await
            .map_err(|e| format!("BiDi subscribe navigationCommitted failed: {e}"))?;
        let mut loaded = bidi
            .subscribe::<Load>()
            .await
            .map_err(|e| format!("BiDi subscribe load failed: {e}"))?;
        let mut failed = bidi
            .subscribe::<NavigationFailed>()
            .await
            .map_err(|e| format!("BiDi subscribe navigationFailed failed: {e}"))?;
        Ok(tokio::spawn(async move {
            loop {
                tokio::select! {
                    ev = committed.next() => {
                        match ev {
                            Some(e) => events.record(NavEventKind::Committed, e.url),
                            None => break,
                        }
                    }
                    ev = loaded.next() => {
                        match ev {
                            Some(e) => events.record(NavEventKind::Load, e.url),
                            None => break,
                        }
                    }
                    ev = failed.next() => {
                        match ev {
                            Some(e) => {
                                warn!("browser: BiDi navigationFailed: {}", e.url);
                                events.record(NavEventKind::Failed, e.url);
                            }
                            None => break,
                        }
                    }
                }
            }
            debug!("browser: BiDi event-drain task ended");
        }))
    }

    /// The shared navigation-event state (for status/diagnostics).
    pub fn events(&self) -> &Arc<NavEvents> {
        &self.events
    }

    /// True when the BiDi session reports ready.
    pub async fn status(&self) -> Result<bool, String> {
        let status = self
            .bidi
            .session()
            .status()
            .await
            .map_err(|e| format!("BiDi session status failed: {e}"))?;
        Ok(status.ready)
    }

    /// Navigate the active tab to `url`, waiting for the `load` event
    /// (BiDi `wait=complete`). The load-wait is the BiDi event, not a
    /// poll — the call resolves when the browser reports the load.
    pub async fn navigate(&self, url: &str, timeout: Duration) -> Result<(), String> {
        let context = self
            .bidi
            .browsing_context()
            .top_level()
            .await
            .map_err(|e| format!("BiDi top_level failed: {e}"))?;
        let since = Instant::now();
        // Bind the browsing-context handle first: `navigate` returns a
        // future borrowing it, so it must outlive the `timeout` await.
        let browsing_context = self.bidi.browsing_context();
        let nav = browsing_context.navigate(
            context,
            url,
            Some(ReadinessState::Complete),
        );
        match tokio::time::timeout(timeout, nav).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(format!("BiDi navigate failed: {e}")),
            Err(_) => {
                // The wait=complete call timed out; the load event may
                // still arrive — give the event stream a short grace window
                // before declaring failure.
                if self.events.wait_for_load(since, Duration::from_secs(5)).await {
                    Ok(())
                } else {
                    Err(format!("BiDi navigate to {url} timed out after {timeout:?}"))
                }
            }
        }
    }

    /// Click at viewport coordinates (x, y) via `input.performActions`.
    /// Coordinates are the screenshot's pixel coordinates (the viewport).
    pub async fn click_at(&self, x: i64, y: i64) -> Result<(), String> {
        let context = self.top_level().await?;
        let actions = serde_json::json!([{
            "type": "pointer",
            "id": "mouse",
            "actions": [
                { "type": "pointerMove", "x": x, "y": y },
                { "type": "pointerDown", "button": 0 },
                { "type": "pointerUp", "button": 0 }
            ]
        }]);
        self.perform_actions(context, actions).await
    }

    /// Type `text` into the currently focused element via key actions.
    /// One keyDown/keyUp pair per character (BiDi has no "type string"
    /// action). Capped at 1000 chars by the caller.
    pub async fn type_text(&self, text: &str) -> Result<(), String> {
        let context = self.top_level().await?;
        let mut key_actions = Vec::with_capacity(text.chars().count() * 2);
        for ch in text.chars() {
            let value = ch.to_string();
            key_actions.push(serde_json::json!({ "type": "keyDown", "value": value }));
            key_actions.push(serde_json::json!({ "type": "keyUp", "value": value }));
        }
        let actions = serde_json::json!([{
            "type": "key",
            "id": "keyboard",
            "actions": key_actions
        }]);
        self.perform_actions(context, actions).await
    }

    /// Press one named key (Enter, Tab, Escape, Backspace, ArrowLeft, ...)
    /// or a single character, via keyDown/keyUp.
    pub async fn press_key(&self, key: &str) -> Result<(), String> {
        let value = map_key_value(key);
        let context = self.top_level().await?;
        let actions = serde_json::json!([{
            "type": "key",
            "id": "keyboard",
            "actions": [
                { "type": "keyDown", "value": value },
                { "type": "keyUp", "value": value }
            ]
        }]);
        self.perform_actions(context, actions).await
    }

    /// End the BiDi session (single-session rule: always session.end on
    /// shutdown) and abort the event-drain task.
    pub async fn end(&mut self) -> Result<(), String> {
        if let Some(drain) = self.drain.take() {
            drain.abort();
        }
        self.bidi
            .session()
            .end()
            .await
            .map_err(|e| format!("BiDi session end failed: {e}"))?;
        Ok(())
    }

    async fn top_level(&self) -> Result<thirtyfour::bidi::BrowsingContextId, String> {
        self.bidi
            .browsing_context()
            .top_level()
            .await
            .map_err(|e| format!("BiDi top_level failed: {e}"))
    }

    async fn perform_actions(
        &self,
        context: thirtyfour::bidi::BrowsingContextId,
        actions: serde_json::Value,
    ) -> Result<(), String> {
        let actions = actions
            .as_array()
            .cloned()
            .unwrap_or_default();
        self.bidi
            .input()
            .perform_actions(context, actions)
            .await
            .map_err(|e| format!("BiDi input performActions failed: {e}"))?;
        Ok(())
    }
}

/// Map a key name to its WebDriver/BiDi key value. Named keys use the
/// private-use values (Enter = U+E007, ...); a single character passes
/// through as itself. Unknown multi-char names are rejected by the caller.
pub fn map_key_value(key: &str) -> String {
    match key {
        "Enter" => "\u{E007}".to_string(),
        "Tab" => "\u{E004}".to_string(),
        "Escape" | "Esc" => "\u{E00C}".to_string(),
        "Backspace" => "\u{E003}".to_string(),
        "Delete" => "\u{E017}".to_string(),
        "ArrowLeft" => "\u{E012}".to_string(),
        "ArrowRight" => "\u{E014}".to_string(),
        "ArrowUp" => "\u{E013}".to_string(),
        "ArrowDown" => "\u{E015}".to_string(),
        "Home" => "\u{E011}".to_string(),
        "End" => "\u{E010}".to_string(),
        "PageUp" => "\u{E00E}".to_string(),
        "PageDown" => "\u{E00F}".to_string(),
        "Space" => " ".to_string(),
        other => other.to_string(),
    }
}

/// The named keys the 2FA input endpoint accepts (plus any single char).
pub const NAMED_KEYS: &[&str] = &[
    "Enter",
    "Tab",
    "Escape",
    "Esc",
    "Backspace",
    "Delete",
    "ArrowLeft",
    "ArrowRight",
    "ArrowUp",
    "ArrowDown",
    "Home",
    "End",
    "PageUp",
    "PageDown",
    "Space",
];

/// True when `key` is an accepted key value: a named key or a single char.
pub fn is_valid_key(key: &str) -> bool {
    NAMED_KEYS.contains(&key) || key.chars().count() == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn nav_events_wait_for_load_returns_on_event() {
        let events = NavEvents::default();
        let since = Instant::now();
        // Feed a load event from a (test) drain task.
        let ev = Arc::new(events);
        let feeder = Arc::clone(&ev);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            feeder.record(NavEventKind::Load, "https://example.com/".to_string());
        });
        assert!(
            ev.wait_for_load(since, Duration::from_secs(5)).await,
            "wait_for_load must return true when a load event arrives"
        );
        let last = ev.last().expect("last event");
        assert_eq!(last.kind, NavEventKind::Load);
        assert_eq!(last.url, "https://example.com/");
    }

    #[tokio::test]
    async fn nav_events_wait_for_load_times_out() {
        let events = NavEvents::default();
        let since = Instant::now();
        // A committed event is not a load event — must not satisfy the wait.
        events.record(
            NavEventKind::Committed,
            "https://example.com/".to_string(),
        );
        assert!(
            !events.wait_for_load(since, Duration::from_millis(100)).await,
            "wait_for_load must time out when only a commit arrived"
        );
    }

    #[tokio::test]
    async fn nav_events_ignores_stale_load() {
        let events = NavEvents::default();
        // Load arrived BEFORE `since` — a later wait must not see it.
        events.record(NavEventKind::Load, "https://old.example/".to_string());
        tokio::time::sleep(Duration::from_millis(5)).await;
        let since = Instant::now();
        assert!(
            !events.wait_for_load(since, Duration::from_millis(100)).await,
            "a load older than `since` must not satisfy the wait"
        );
    }

    #[test]
    fn key_mapping_covers_named_keys() {
        assert_eq!(map_key_value("Enter"), "\u{E007}");
        assert_eq!(map_key_value("Tab"), "\u{E004}");
        assert_eq!(map_key_value("Escape"), "\u{E00C}");
        assert_eq!(map_key_value("a"), "a");
        assert!(is_valid_key("Enter"));
        assert!(is_valid_key("x"));
        assert!(!is_valid_key("F13"));
        assert!(!is_valid_key(""));
    }
}
