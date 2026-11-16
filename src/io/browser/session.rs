//! Browser session manager (task 351).
//!
//! A long-lived service like `mcp_client`, not a one-shot tool function:
//! one shared Firefox session (one page/tab) that the LLM and John share
//! — the "like Cobble jr's browser" model. A browser tool call never
//! launches a fresh browser.
//!
//! Layers (thinnest at the bottom):
//! - [`BrowserDriver`] — the async driver interface. `RealDriver` is the
//!   production impl (thirtyfour over WebDriver BiDi, classic WebDriver
//!   as fallback); `FakeDriver` is the in-memory stub tests use (no
//!   Firefox in the sandbox).
//! - [`BrowserSession`] — the manager: start/stop/restart, the action
//!   dispatch, the navigation allowlist, the password guards, and the
//!   Gmail OAuth flow (task 354).
//!
//! Policy lives in the session, not the driver, so it applies regardless
//! of which driver is behind it:
//! - Navigation is confined to `[browser] allowed_hosts` (default Google
//!   + localhost). Anything else is refused with a message telling the
//!   LLM to ask the user.
//! - The tool never returns password field contents: `get_text` strips
//!   `input[type=password]` before extracting text, and `evaluate`
//!   refuses any script mentioning "password". The LLM can only *type*
//!   what it knows (e.g. a 2FA code John dictates); it can never read a
//!   saved password, because it never sees one.
//! - Single-session rule (BiDi): Firefox allows exactly ONE BiDi session
//!   and it outlives its WebSocket — always `session.end` on shutdown
//!   (see [`RealDriver::shutdown`]); a refused session on startup means
//!   restart Firefox, don't retry the same session (the `restart` action
//!   stops the session, kills leftover firefox/geckodriver processes,
//!   and starts fresh).

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use log::{debug, info, warn};
use serde_json::{json, Value};
use thirtyfour::prelude::*;

use crate::config::browser::BrowserConfig;

use super::bidi::{BidiSeam, NavEvent};

/// A 1x1 transparent PNG, returned by the fake driver's screenshot.
const FAKE_PNG_1X1: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
    0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F,
    0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00,
    0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49,
    0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

/// JS for `get_text`: clone the body, drop password inputs (defense in
/// depth — `textContent` never includes input values anyway) and
/// script/style, then return the visible text. `textContent` (not
/// `innerText`) so it works on the detached clone.
const GET_TEXT_SCRIPT: &str = r#"(() => {
  const clone = document.body.cloneNode(true);
  clone.querySelectorAll('input[type="password"]').forEach(el => el.remove());
  clone.querySelectorAll('script, style, noscript').forEach(el => el.remove());
  return (clone.textContent || '').replace(/\s+/g, ' ').trim();
})()"#;

const READY_STATE_SCRIPT: &str = "return document.readyState";

/// Max chars returned by `get_text` (the LLM works from text, not images).
const GET_TEXT_MAX_CHARS: usize = 8000;
/// Max chars returned by `evaluate`.
const EVALUATE_MAX_CHARS: usize = 4000;
/// Max chars the `type` action / input endpoint accept in one call.
const TYPE_MAX_CHARS: usize = 1000;

/// A browser driver: the async interface the session manager drives.
/// `Send + Sync` so it can live behind the manager mutex on the dedicated
/// browser runtime (see `src/io/browser/mod.rs`).
#[async_trait]
pub trait BrowserDriver: Send + Sync {
    /// Navigate to `url` (BiDi-first, classic fallback) and return
    /// (current URL, page title).
    async fn navigate(&self, url: &str, timeout: Duration) -> Result<(String, String), String>;
    /// Click the element matching `selector` (CSS).
    async fn click(&self, selector: &str) -> Result<String, String>;
    /// Click the element matching `selector`, then type `text` into it.
    async fn type_text(&self, selector: &str, text: &str) -> Result<String, String>;
    /// Current viewport as PNG bytes.
    async fn screenshot_png(&self) -> Result<Vec<u8>, String>;
    /// Visible page text (password inputs stripped; see module docs).
    async fn get_text(&self) -> Result<String, String>;
    /// Current URL.
    async fn current_url(&self) -> Result<String, String>;
    /// Browser back.
    async fn back(&self) -> Result<(), String>;
    /// Run `script` and return its JSON result as a string.
    async fn evaluate(&self, script: &str) -> Result<String, String>;
    /// Click at viewport coordinates (x, y) — the 2FA affordance.
    async fn input_click(&self, x: i64, y: i64) -> Result<String, String>;
    /// Type `text` into the currently focused element — the 2FA affordance.
    async fn input_type(&self, text: &str) -> Result<String, String>;
    /// Press one named key (Enter, Tab, ...) or a single char.
    async fn input_key(&self, key: &str) -> Result<String, String>;
    /// True when the BiDi session is connected and ready.
    async fn bidi_ready(&self) -> bool;
    /// The most recent BiDi navigation event, if any.
    async fn last_nav_event(&self) -> Option<NavEvent>;
    /// End the BiDi session (if any) and quit the driver.
    async fn shutdown(&self) -> Result<(), String>;
}

// ── Real driver (thirtyfour) ─────────────────────────────────────────────

/// Production driver: a thirtyfour `WebDriver` over a managed geckodriver,
/// BiDi-first via [`BidiSeam`], classic WebDriver as fallback (task 352).
pub struct RealDriver {
    driver: WebDriver,
    /// The BiDi seam, if it connected. `None` means classic-only (the seam
    /// failed at startup and the session carries on without it).
    seam: tokio::sync::Mutex<Option<BidiSeam>>,
}

impl RealDriver {
    /// Start a headless Firefox session per `config`.
    /// - `firefox_binary` set: use that binary; otherwise PATH.
    /// - `profile_dir` set: launch with that profile (`-profile`).
    /// - `geckodriver_url` set: connect to that driver; otherwise
    ///   `WebDriver::managed` auto-downloads the matching geckodriver.
    /// - BiDi is enabled on the capabilities (`enable_bidi`); the seam
    ///   connects after the session starts.
    pub async fn start(config: &BrowserConfig) -> Result<Self, String> {
        let mut caps = DesiredCapabilities::firefox();
        if config.headless {
            caps.set_headless()
                .map_err(|e| format!("firefox caps set_headless failed: {e}"))?;
        }
        if !config.firefox_binary.is_empty() {
            caps.set_firefox_binary(&config.firefox_binary)
                .map_err(|e| format!("firefox caps set_firefox_binary failed: {e}"))?;
        }
        if let Some(profile) = config.resolved_profile_dir() {
            if !profile.exists() {
                warn!(
                    "browser: profile dir {} does not exist yet — Firefox will create it (Sync sign-in will be needed)",
                    profile.display()
                );
            }
            caps.add_arg("-profile")
                .map_err(|e| format!("firefox caps add_arg failed: {e}"))?;
            caps.add_arg(profile.to_string_lossy().as_ref())
                .map_err(|e| format!("firefox caps add_arg failed: {e}"))?;
        }
        // Small viewport so one-shot 2FA screenshots stay small (there is
        // no image crate in the tree to downscale with).
        caps.add_arg("--window-size=1024,768")
            .map_err(|e| format!("firefox caps add_arg failed: {e}"))?;
        caps.enable_bidi()
            .map_err(|e| format!("firefox caps enable_bidi failed: {e}"))?;

        let driver = if config.geckodriver_url.is_empty() {
            WebDriver::managed(caps)
                .await
                .map_err(|e| format!("managed geckodriver launch failed: {e}"))?
        } else {
            WebDriver::new(config.geckodriver_url.clone(), caps)
                .await
                .map_err(|e| format!("geckodriver connect failed: {e}"))?
        };
        info!("browser: Firefox session started");

        // BiDi seam (BiDi-first). A seam failure does NOT fail the session:
        // the driver carries on classic-only and the fallback rule in each
        // method logs the downgrade.
        let seam = match BidiSeam::connect(&driver).await {
            Ok(s) => {
                info!("browser: BiDi seam connected");
                Some(s)
            }
            Err(e) => {
                warn!("browser: BiDi seam failed, classic-only session: {e}");
                None
            }
        };
        Ok(Self {
            driver,
            seam: tokio::sync::Mutex::new(seam),
        })
    }

    /// Classic fallback for load-wait: poll `document.readyState` until
    /// `complete` or `timeout`. Best-effort — a JS error just ends the
    /// wait (the `goto` already returned).
    async fn wait_ready_state(&self, timeout: Duration) -> Result<(), String> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            match self.driver.execute(READY_STATE_SCRIPT, vec![]).await {
                Ok(ret) if ret.json().as_str() == Some("complete") => return Ok(()),
                _ => {}
            }
            if std::time::Instant::now() >= deadline {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    async fn current_url_and_title(&self) -> Result<(String, String), String> {
        let url = self
            .driver
            .current_url()
            .await
            .map_err(|e| e.to_string())?
            .to_string();
        let title = self
            .driver
            .title()
            .await
            .map_err(|e| e.to_string())
            .unwrap_or_default();
        Ok((url, title))
    }

    // BiDi-first call sites below lock `self.seam` inline and drop the
    // guard before falling back (a closure-based helper cannot express
    // this on stable Rust: the seam call's future borrows both the seam
    // and the caller's arguments, which a `for<'a>` closure bound cannot
    // capture). Pattern per site:
    //   let seam_result = { let g = self.seam.lock().await;
    //     match g.as_ref() { Some(s) => Some(s.<call>().await), None => None } };
    //   match seam_result { Some(Ok(())) => ..., Some(Err(e)) => warn + fall back, None => fall back }
}

#[async_trait]
impl BrowserDriver for RealDriver {
    async fn navigate(&self, url: &str, timeout: Duration) -> Result<(String, String), String> {
        let seam_result = {
            let guard = self.seam.lock().await;
            match guard.as_ref() {
                Some(seam) => Some(seam.navigate(url, timeout).await),
                None => None,
            }
        };
        match seam_result {
            Some(Ok(())) => return self.current_url_and_title().await,
            Some(Err(e)) => warn!("browser: BiDi navigate failed, classic goto fallback: {e}"),
            None => {} // classic-only session: fall through to the fallback
        }
        self.driver
            .goto(url)
            .await
            .map_err(|e| e.to_string())?;
        self.wait_ready_state(timeout).await?;
        self.current_url_and_title().await
    }

    async fn click(&self, selector: &str) -> Result<String, String> {
        let el = self
            .driver
            .find(By::Css(selector))
            .await
            .map_err(|e| format!("no element for selector '{selector}': {e}"))?;
        el.click().await.map_err(|e| e.to_string())?;
        Ok(format!("Clicked '{selector}'."))
    }

    async fn type_text(&self, selector: &str, text: &str) -> Result<String, String> {
        let el = self
            .driver
            .find(By::Css(selector))
            .await
            .map_err(|e| format!("no element for selector '{selector}': {e}"))?;
        el.click().await.map_err(|e| e.to_string())?;
        el.send_keys(text).await.map_err(|e| e.to_string())?;
        Ok(format!("Typed {} chars into '{selector}'.", text.chars().count()))
    }

    async fn screenshot_png(&self) -> Result<Vec<u8>, String> {
        self.driver
            .screenshot_as_png()
            .await
            .map_err(|e| e.to_string())
    }

    async fn get_text(&self) -> Result<String, String> {
        let ret = self
            .driver
            .execute(GET_TEXT_SCRIPT, vec![])
            .await
            .map_err(|e| e.to_string())?;
        Ok(ret.json().as_str().unwrap_or("").to_string())
    }

    async fn current_url(&self) -> Result<String, String> {
        Ok(self
            .driver
            .current_url()
            .await
            .map_err(|e| e.to_string())?
            .to_string())
    }

    async fn back(&self) -> Result<(), String> {
        self.driver.back().await.map_err(|e| e.to_string())
    }

    async fn evaluate(&self, script: &str) -> Result<String, String> {
        let ret = self
            .driver
            .execute(script, vec![])
            .await
            .map_err(|e| e.to_string())?;
        Ok(ret.json().to_string())
    }

    async fn input_click(&self, x: i64, y: i64) -> Result<String, String> {
        let seam_result = {
            let guard = self.seam.lock().await;
            match guard.as_ref() {
                Some(seam) => Some(seam.click_at(x, y).await),
                None => None,
            }
        };
        match seam_result {
            Some(Ok(())) => return Ok(format!("Clicked at ({x}, {y}).")),
            Some(Err(e)) => warn!("browser: BiDi click_at failed, ActionChain fallback: {e}"),
            None => {} // classic-only session: fall through to the fallback
        }
        self.driver
            .action_chain()
            .move_to(x, y)
            .click()
            .perform()
            .await
            .map_err(|e| e.to_string())?;
        Ok(format!("Clicked at ({x}, {y})."))
    }

    async fn input_type(&self, text: &str) -> Result<String, String> {
        let seam_result = {
            let guard = self.seam.lock().await;
            match guard.as_ref() {
                Some(seam) => Some(seam.type_text(text).await),
                None => None,
            }
        };
        match seam_result {
            Some(Ok(())) => return Ok(format!("Typed {} chars.", text.chars().count())),
            Some(Err(e)) => warn!("browser: BiDi type_text failed, ActionChain fallback: {e}"),
            None => {} // classic-only session: fall through to the fallback
        }
        self.driver
            .action_chain()
            .send_keys(text)
            .perform()
            .await
            .map_err(|e| e.to_string())?;
        Ok(format!("Typed {} chars.", text.chars().count()))
    }

    async fn input_key(&self, key: &str) -> Result<String, String> {
        let seam_result = {
            let guard = self.seam.lock().await;
            match guard.as_ref() {
                Some(seam) => Some(seam.press_key(key).await),
                None => None,
            }
        };
        match seam_result {
            Some(Ok(())) => return Ok(format!("Pressed {key}.")),
            Some(Err(e)) => warn!("browser: BiDi press_key failed, ActionChain fallback: {e}"),
            None => {} // classic-only session: fall through to the fallback
        }
        // Classic fallback: single chars via send_keys, named keys via Key.
        let mut chain = self.driver.action_chain();
        match key {
            "Enter" => chain = chain.key_down(Key::Enter).key_up(Key::Enter),
            "Tab" => chain = chain.key_down(Key::Tab).key_up(Key::Tab),
            "Escape" | "Esc" => chain = chain.key_down(Key::Escape).key_up(Key::Escape),
            "Backspace" => chain = chain.key_down(Key::Backspace).key_up(Key::Backspace),
            "Delete" => chain = chain.key_down(Key::Delete).key_up(Key::Delete),
            "ArrowLeft" => chain = chain.key_down(Key::Left).key_up(Key::Left),
            "ArrowRight" => chain = chain.key_down(Key::Right).key_up(Key::Right),
            "ArrowUp" => chain = chain.key_down(Key::Up).key_up(Key::Up),
            "ArrowDown" => chain = chain.key_down(Key::Down).key_up(Key::Down),
            "Home" => chain = chain.key_down(Key::Home).key_up(Key::Home),
            "End" => chain = chain.key_down(Key::End).key_up(Key::End),
            "PageUp" => chain = chain.key_down(Key::PageUp).key_up(Key::PageUp),
            "PageDown" => chain = chain.key_down(Key::PageDown).key_up(Key::PageDown),
            "Space" => chain = chain.key_down(Key::Space).key_up(Key::Space),
            single if single.chars().count() == 1 => {
                chain = chain.send_keys(single);
            }
            other => return Err(format!("unknown key '{other}'")),
        }
        chain.perform().await.map_err(|e| e.to_string())?;
        Ok(format!("Pressed {key}."))
    }

    async fn bidi_ready(&self) -> bool {
        let guard = self.seam.lock().await;
        match guard.as_ref() {
            Some(seam) => seam.status().await.unwrap_or(false),
            None => false,
        }
    }

    async fn last_nav_event(&self) -> Option<NavEvent> {
        let guard = self.seam.lock().await;
        guard.as_ref().and_then(|seam| seam.events().last())
    }

    async fn shutdown(&self) -> Result<(), String> {
        // Single-session rule: always session.end, then quit the driver.
        // The seam is taken out so a second shutdown is a no-op for BiDi.
        let mut guard = self.seam.lock().await;
        if let Some(mut seam) = guard.take() {
            if let Err(e) = seam.end().await {
                warn!("browser: BiDi session end failed during shutdown: {e}");
            }
        }
        drop(guard);
        // WebDriver is Clone (Arc wrapper); quitting the clone ends the
        // shared session.
        self.driver.clone().quit().await.map_err(|e| e.to_string())?;
        info!("browser: driver quit");
        Ok(())
    }
}

// ── Fake driver (tests) ──────────────────────────────────────────────────

/// In-memory stub driver for tests (no Firefox in the sandbox).
/// Records calls so tests can assert the session drove it correctly.
pub struct FakeDriver {
    state: std::sync::Mutex<FakeState>,
}

struct FakeState {
    url: String,
    title: String,
    text: String,
    clicks: Vec<String>,
    typed: Vec<(String, String)>,
    input_clicks: Vec<(i64, i64)>,
    input_typed: Vec<String>,
    input_keys: Vec<String>,
    bidi: bool,
    last_nav: Option<NavEvent>,
    shut_down: bool,
}

impl FakeDriver {
    pub fn new() -> Self {
        Self {
            state: std::sync::Mutex::new(FakeState {
                url: "about:blank".to_string(),
                title: String::new(),
                text: "fake page text".to_string(),
                clicks: Vec::new(),
                typed: Vec::new(),
                input_clicks: Vec::new(),
                input_typed: Vec::new(),
                input_keys: Vec::new(),
                bidi: true,
                last_nav: None,
                shut_down: false,
            }),
        }
    }

    /// Set the page text `get_text` returns (for password-guard tests).
    pub fn set_text(&self, text: &str) {
        self.state.lock().unwrap().text = text.to_string();
    }

    pub fn clicks(&self) -> Vec<String> {
        self.state.lock().unwrap().clicks.clone()
    }

    pub fn typed(&self) -> Vec<(String, String)> {
        self.state.lock().unwrap().typed.clone()
    }

    pub fn input_clicks(&self) -> Vec<(i64, i64)> {
        self.state.lock().unwrap().input_clicks.clone()
    }

    pub fn input_typed(&self) -> Vec<String> {
        self.state.lock().unwrap().input_typed.clone()
    }

    pub fn input_keys(&self) -> Vec<String> {
        self.state.lock().unwrap().input_keys.clone()
    }

    pub fn was_shut_down(&self) -> bool {
        self.state.lock().unwrap().shut_down
    }

    /// Synchronous current-URL accessor for sync tests (the async
    /// `current_url` is the trait method).
    pub fn current_url_sync(&self) -> String {
        self.state.lock().unwrap().url.clone()
    }
}

impl Default for FakeDriver {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl BrowserDriver for FakeDriver {
    async fn navigate(&self, url: &str, _timeout: Duration) -> Result<(String, String), String> {
        let mut st = self.state.lock().unwrap();
        st.url = url.to_string();
        st.title = format!("Fake: {url}");
        st.last_nav = Some(NavEvent {
            kind: super::bidi::NavEventKind::Load,
            url: url.to_string(),
            at: std::time::Instant::now(),
        });
        Ok((st.url.clone(), st.title.clone()))
    }

    async fn click(&self, selector: &str) -> Result<String, String> {
        self.state.lock().unwrap().clicks.push(selector.to_string());
        Ok(format!("Clicked '{selector}'."))
    }

    async fn type_text(&self, selector: &str, text: &str) -> Result<String, String> {
        self.state
            .lock()
            .unwrap()
            .typed
            .push((selector.to_string(), text.to_string()));
        Ok(format!("Typed {} chars into '{selector}'.", text.chars().count()))
    }

    async fn screenshot_png(&self) -> Result<Vec<u8>, String> {
        Ok(FAKE_PNG_1X1.to_vec())
    }

    async fn get_text(&self) -> Result<String, String> {
        Ok(self.state.lock().unwrap().text.clone())
    }

    async fn current_url(&self) -> Result<String, String> {
        Ok(self.state.lock().unwrap().url.clone())
    }

    async fn back(&self) -> Result<(), String> {
        Ok(())
    }

    async fn evaluate(&self, script: &str) -> Result<String, String> {
        Ok(format!("fake-eval: {script}"))
    }

    async fn input_click(&self, x: i64, y: i64) -> Result<String, String> {
        self.state.lock().unwrap().input_clicks.push((x, y));
        Ok(format!("Clicked at ({x}, {y})."))
    }

    async fn input_type(&self, text: &str) -> Result<String, String> {
        self.state.lock().unwrap().input_typed.push(text.to_string());
        Ok(format!("Typed {} chars.", text.chars().count()))
    }

    async fn input_key(&self, key: &str) -> Result<String, String> {
        self.state.lock().unwrap().input_keys.push(key.to_string());
        Ok(format!("Pressed {key}."))
    }

    async fn bidi_ready(&self) -> bool {
        self.state.lock().unwrap().bidi
    }

    async fn last_nav_event(&self) -> Option<NavEvent> {
        self.state.lock().unwrap().last_nav.clone()
    }

    async fn shutdown(&self) -> Result<(), String> {
        self.state.lock().unwrap().shut_down = true;
        Ok(())
    }
}

// ── Session manager ──────────────────────────────────────────────────────

/// The browser session manager: one shared Firefox session, started
/// lazily on first use, restarted on demand.
pub struct BrowserSession {
    config: BrowserConfig,
    driver: Option<Arc<dyn BrowserDriver>>,
    /// Test seam: when set, `ensure_started` uses this driver instead of
    /// launching a real Firefox. Only used by `with_driver` (tests).
    #[cfg(test)]
    test_driver: Option<Arc<dyn BrowserDriver>>,
    restarts: u32,
    last_error: Option<String>,
}

impl BrowserSession {
    pub fn new(config: BrowserConfig) -> Self {
        Self {
            config,
            driver: None,
            #[cfg(test)]
            test_driver: None,
            restarts: 0,
            last_error: None,
        }
    }

    /// Test constructor: the session drives `driver` (a FakeDriver) and
    /// never launches a real Firefox.
    #[cfg(test)]
    pub fn with_driver(config: BrowserConfig, driver: Arc<dyn BrowserDriver>) -> Self {
        Self {
            config,
            driver: Some(Arc::clone(&driver)),
            test_driver: Some(driver),
            restarts: 0,
            last_error: None,
        }
    }

    pub fn config(&self) -> &BrowserConfig {
        &self.config
    }

    pub fn is_started(&self) -> bool {
        self.driver.is_some()
    }

    /// The live driver, starting the session first when needed.
    /// Returns the disabled message when `[browser] enabled = false`.
    pub async fn ensure_started(&mut self) -> Result<Arc<dyn BrowserDriver>, String> {
        if !self.config.is_enabled() {
            return Err(disabled_message());
        }
        if let Some(driver) = &self.driver {
            return Ok(Arc::clone(driver));
        }
        #[cfg(test)]
        if let Some(driver) = &self.test_driver {
            let driver = Arc::clone(driver);
            self.driver = Some(Arc::clone(&driver));
            return Ok(driver);
        }
        match RealDriver::start(&self.config).await {
            Ok(real) => {
                let driver: Arc<dyn BrowserDriver> = Arc::new(real);
                self.driver = Some(Arc::clone(&driver));
                self.last_error = None;
                Ok(driver)
            }
            Err(e) => {
                self.last_error = Some(e.clone());
                Err(e)
            }
        }
    }

    /// Stop the session: end the BiDi session, quit the driver, clear it.
    /// Always safe to call, even when nothing is running.
    pub async fn stop(&mut self) -> Result<String, String> {
        match self.driver.take() {
            Some(driver) => match driver.shutdown().await {
                Ok(()) => Ok(
                    "Browser session stopped (BiDi session ended, driver quit).".to_string(),
                ),
                Err(e) => Err(format!(
                    "Browser shutdown reported an error (session cleared anyway): {e}"
                )),
            },
            None => Ok("Browser was not running.".to_string()),
        }
    }

    /// Restart the session: stop, kill leftover firefox/geckodriver
    /// processes (the single-session recovery — a refused BiDi session
    /// means restart Firefox, don't retry the same session), start fresh.
    /// Explicit action only; never automatic.
    pub async fn restart(&mut self) -> Result<String, String> {
        let _ = self.stop().await;
        self.restarts += 1;
        kill_leftover_browser_processes();
        // Give the OS a moment to reap the killed processes before the
        // new session asks geckodriver for a fresh Firefox.
        tokio::time::sleep(Duration::from_millis(500)).await;
        match self.ensure_started().await {
            Ok(_) => Ok(format!("Browser restarted (restart #{}).", self.restarts)),
            Err(e) => Err(format!("Browser restart failed: {e}")),
        }
    }

    /// One-line status (no driver start — a status check never launches
    /// a browser).
    pub async fn status_text(&self) -> String {
        let mut lines = vec![format!(
            "Browser: {}",
            if self.config.is_enabled() {
                "enabled"
            } else {
                "disabled ([browser] enabled = false)"
            }
        )];
        match &self.driver {
            Some(driver) => {
                lines.push(format!("Session: running (restarts: {})", self.restarts));
                lines.push(format!("BiDi: {}", if driver.bidi_ready().await { "ready" } else { "not connected (classic-only)" }));
                if let Ok(url) = driver.current_url().await {
                    lines.push(format!("URL: {url}"));
                }
                if let Some(ev) = driver.last_nav_event().await {
                    lines.push(format!("Last nav event: {:?} {}", ev.kind, ev.url));
                }
            }
            None => lines.push("Session: not running".to_string()),
        }
        if let Some(e) = &self.last_error {
            lines.push(format!("Last error: {e}"));
        }
        lines.join("\n")
    }

    /// The action dispatch for the `browser` tool (task 352).
    /// Returns short text results; errors are plain strings.
    pub async fn act(&mut self, action: &str, args: &Value) -> Result<String, String> {
        match action {
            "status" => Ok(self.status_text().await),
            "restart" => self.restart().await,
            "gmail_oauth_check" => self.gmail_oauth_check().await,
            "gmail_oauth" => self.gmail_oauth().await,
            "navigate" => {
                let url = required_str(args, "url")?;
                self.navigate_to(&url).await
            }
            "click" => {
                // Two forms: selector-based ("selector") for the LLM when it
                // knows the element, or coordinate-based (x, y) — the same
                // form the 2FA box uses. The tool schema advertises both.
                if let Some(selector) = args.get("selector").and_then(|v| v.as_str()) {
                    let driver = self.ensure_started().await?;
                    driver.click(selector).await
                } else {
                    let x = args.get("x").and_then(|v| v.as_i64());
                    let y = args.get("y").and_then(|v| v.as_i64());
                    match (x, y) {
                        (Some(x), Some(y)) => {
                            self.input("click", Some(x), Some(y), None, None).await
                        }
                        _ => Err(
                            "click needs either a 'selector' (CSS) or integer 'x' and 'y' coordinates"
                                .to_string(),
                        ),
                    }
                }
            }
            "type" => {
                let text = required_str(args, "text")?;
                if text.chars().count() > TYPE_MAX_CHARS {
                    return Err(format!(
                        "text too long (max {TYPE_MAX_CHARS} chars in one type action)"
                    ));
                }
                if let Some(selector) = args.get("selector").and_then(|v| v.as_str()) {
                    let driver = self.ensure_started().await?;
                    driver.type_text(selector, &text).await
                } else {
                    // Coordinate form: click to focus first, then type into
                    // the focused element (the 2FA box does the click and
                    // the type as separate calls; the LLM gets one call).
                    let x = args.get("x").and_then(|v| v.as_i64());
                    let y = args.get("y").and_then(|v| v.as_i64());
                    if let (Some(x), Some(y)) = (x, y) {
                        self.input("click", Some(x), Some(y), None, None).await?;
                    }
                    self.input("type", None, None, Some(&text), None).await
                }
            }
            "key" => {
                let key = required_str(args, "key")?;
                self.input("key", None, None, None, Some(&key)).await
            }
            "screenshot" => {
                let png = self.screenshot().await?;
                Ok(format!(
                    "Screenshot saved: {} ({} bytes).",
                    save_screenshot(&png)?,
                    png.len()
                ))
            }
            "get_text" => {
                let driver = self.ensure_started().await?;
                let text = driver.get_text().await?;
                Ok(truncate(&text, GET_TEXT_MAX_CHARS))
            }
            "current_url" => {
                let driver = self.ensure_started().await?;
                driver.current_url().await
            }
            "back" => {
                let driver = self.ensure_started().await?;
                driver.back().await?;
                let url = driver.current_url().await?;
                Ok(format!("Went back. Now at {url}."))
            }
            "evaluate" => {
                let script = required_str(args, "script")?;
                // Password guard (task 353): the tool never returns
                // password field contents. A script that even mentions
                // "password" is refused — use click/type/get_text for the
                // sign-in flow instead. (Substring guard, not a sandbox;
                // the real protection is that no tool path reads a
                // password value — get_text strips the fields first.)
                if script.to_lowercase().contains("password") {
                    return Err(
                        "evaluate scripts must not touch password fields (the browser tool never returns password values). Use click/type/get_text for the sign-in flow instead."
                            .to_string(),
                    );
                }
                let driver = self.ensure_started().await?;
                let out = driver.evaluate(&script).await?;
                Ok(truncate(&out, EVALUATE_MAX_CHARS))
            }
            other => Err(format!(
                "Unknown browser action '{other}'. Actions: navigate, click, type, key, screenshot, get_text, current_url, back, evaluate, gmail_oauth, gmail_oauth_check, restart, status."
            )),
        }
    }

    /// Navigate with the allowlist enforced (task 351/353).
    pub async fn navigate_to(&mut self, url: &str) -> Result<String, String> {
        if !self.config.host_allowed(url) {
            warn!("browser: navigation refused (allowlist): {url}");
            return Err(format!(
                "Host of '{url}' is not in the browser navigation allowlist ([browser] allowed_hosts; default is Google + localhost). Ask the user to confirm this host, or add it to the config."
            ));
        }
        let driver = self.ensure_started().await?;
        let timeout = Duration::from_secs(self.config.action_timeout_secs);
        let (current, title) = driver.navigate(url, timeout).await?;
        Ok(format!("Navigated to {current} — {title}"))
    }

    /// One-shot screenshot (the 2FA affordance, task 355).
    pub async fn screenshot(&mut self) -> Result<Vec<u8>, String> {
        let driver = self.ensure_started().await?;
        let png = driver.screenshot_png().await?;
        if png.len() > self.config.screenshot_max_bytes {
            warn!(
                "browser: screenshot is {} bytes (cap {}); returning it anyway — the viewport is already small, see [browser] screenshot_max_bytes",
                png.len(),
                self.config.screenshot_max_bytes
            );
        }
        Ok(png)
    }

    /// One input action for the 2FA affordance (task 355).
    /// `kind`: "click" (needs x, y), "type" (needs text), "key" (needs key).
    pub async fn input(
        &mut self,
        kind: &str,
        x: Option<i64>,
        y: Option<i64>,
        text: Option<&str>,
        key: Option<&str>,
    ) -> Result<String, String> {
        let driver = self.ensure_started().await?;
        match kind {
            "click" => {
                let (x, y) = match (x, y) {
                    (Some(x), Some(y)) => (x, y),
                    _ => return Err("click needs integer x and y".to_string()),
                };
                if !(0..=10000).contains(&x) || !(0..=10000).contains(&y) {
                    return Err("click coordinates must be between 0 and 10000".to_string());
                }
                driver.input_click(x, y).await
            }
            "type" => {
                let text = text.unwrap_or("");
                if text.is_empty() {
                    return Err("type needs non-empty text".to_string());
                }
                if text.chars().count() > TYPE_MAX_CHARS {
                    return Err(format!(
                        "text too long (max {TYPE_MAX_CHARS} chars in one input)"
                    ));
                }
                driver.input_type(text).await
            }
            "key" => {
                let key = key.unwrap_or("");
                if !crate::io::browser::bidi::is_valid_key(key) {
                    return Err(format!(
                        "unknown key '{key}' — use a named key (Enter, Tab, Escape, Backspace, ArrowLeft, ...) or a single character"
                    ));
                }
                driver.input_key(key).await
            }
            other => Err(format!(
                "Unknown input kind '{other}'. Kinds: click, type, key."
            )),
        }
    }

    // ── Gmail OAuth (task 354) ──────────────────────────────────────

    /// Start the Gmail OAuth flow through the browser.
    /// 1. Probe google-mcp with a trivial gmail_search — a dead token
    ///    makes google-mcp surface its consent URL.
    /// 2. If the search already works, the token is valid: report that.
    /// 3. Otherwise navigate the browser to the consent URL and report
    ///    the next steps for John (sign in, 2FA via the 355 affordance).
    /// The redirect lands on R630 localhost where google-mcp listens, so
    /// no SSH tunnel is needed at any step.
    pub async fn gmail_oauth(&mut self) -> Result<String, String> {
        let probe = crate::mcp_client::try_execute(
            "mcp__google__gmail_search",
            &json!({"query": "in:inbox", "max_results": 1}),
        );
        let probe = match probe {
            Some(out) => out,
            None => {
                return Err(
                    "google-mcp is not available (MCP disabled or no google server). Enable [mcp] with the google server, then retry."
                        .to_string(),
                )
            }
        };
        if let Some(url) = extract_consent_url(&probe) {
            let navigated = self.navigate_to(&url).await?;
            return Ok(format!(
                "Gmail OAuth started.\n{navigated}\n\
                 Next steps for John: sign in — your Google password autofills from Sync. \
                 Then complete 2FA with the Google prompt on your phone or an authenticator-app code, \
                 using the browser screenshot + input buttons in the web UI (they need the web token). \
                 A security-key (YubiKey) challenge CANNOT be completed here — use 'try another way' if Google offers the key first. \
                 The redirect lands on this machine (localhost), where google-mcp listens; the token is saved to ~/.config/google-mcp/token.json. \
                 When done, run the browser action gmail_oauth_check to verify."
            ));
        }
        if looks_like_search_success(&probe) {
            return Ok(format!(
                "Gmail already works — the google-mcp search returned results, so the token is valid and no OAuth flow is needed.\nSearch output (truncated): {}",
                truncate(&probe, 500)
            ));
        }
        Err(format!(
            "google-mcp did not surface a consent URL, and the search did not clearly succeed. \
             Likely causes: the NEW client secret (created Nov 14, ending hhdr) is not in the live /home/cobble/helix/config.toml (the old secret was deleted and is dead), or google-mcp is not running. \
             Check the secret first, then retry.\n\
             google-mcp output (truncated): {}",
            truncate(&probe, 500)
        ))
    }

    /// Verify the Gmail OAuth result: the token file (scope + expiry) and
    /// a live gmail_search through Helix. No browser needed.
    pub async fn gmail_oauth_check(&self) -> Result<String, String> {
        let path = self.config.resolved_gmail_token_path();
        let text = std::fs::read_to_string(&path).map_err(|e| {
            format!(
                "Token file not found at {}: {e} (run the gmail_oauth flow first).",
                path.display()
            )
        })?;
        let token: Value = serde_json::from_str(&text)
            .map_err(|e| format!("Token file is not valid JSON: {e}"))?;
        let scope = token
            .get("scope")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                token.get("scopes").and_then(|s| s.as_array()).map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .collect::<Vec<_>>()
                        .join(" ")
                })
            })
            .unwrap_or_default();
        let has_gmail = scope.contains("gmail.modify");
        let refresh = token.get("refresh_token").is_some();
        let expiry_note = token_expiry_note(&token);
        let probe = crate::mcp_client::try_execute(
            "mcp__google__gmail_search",
            &json!({"query": "in:inbox", "max_results": 1}),
        );
        let live = match probe {
            Some(out) if looks_like_search_success(&out) => {
                format!("Live gmail_search: OK ({}).", truncate(&out, 200))
            }
            Some(out) => format!(
                "Live gmail_search: did not clearly succeed. Output: {}",
                truncate(&out, 300)
            ),
            None => "Live gmail_search: google-mcp unavailable.".to_string(),
        };
        Ok(format!(
            "Gmail OAuth check:\nToken file: {}\nScope: {} (gmail.modify: {})\nRefresh token present: {}\nExpiry: {}\n{}",
            path.display(),
            if scope.is_empty() { "(none recorded)" } else { &scope },
            has_gmail,
            refresh,
            expiry_note,
            live
        ))
    }
}

/// The disabled message, shared by the tool and the session.
pub fn disabled_message() -> String {
    "Browser is disabled ([browser] enabled = false in config.toml). Enable it to use the browser tool.".to_string()
}

/// Required string arg for a tool action.
fn required_str(args: &Value, name: &str) -> Result<String, String> {
    args.get(name)
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| format!("browser action needs a non-empty string argument '{name}'"))
}

/// Truncate `s` to `max` chars, noting the cut.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let kept: String = s.chars().take(max).collect();
    format!("{kept}\n[...truncated to {max} chars]")
}

/// Save a screenshot under artifacts/ with a collision-proof name and
/// return its /artifacts/ URL.
fn save_screenshot(png: &[u8]) -> Result<String, String> {
    let dir = std::path::Path::new("artifacts");
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("cannot create artifacts dir: {e}"))?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    for suffix in ["", "-2", "-3", "-4", "-5"] {
        let name = format!("browser-{stamp}{suffix}.png");
        let path = dir.join(&name);
        if path.exists() {
            continue;
        }
        std::fs::write(&path, png).map_err(|e| format!("cannot save screenshot: {e}"))?;
        return Ok(format!("/artifacts/{name}"));
    }
    Err("could not find a free screenshot filename".to_string())
}

/// Kill leftover firefox/geckodriver processes (the single-session
/// recovery for `restart`). Best-effort: no browser running is fine.
/// `-x` matches the exact process name, so nothing else is touched.
fn kill_leftover_browser_processes() {
    for proc in ["firefox", "geckodriver"] {
        match std::process::Command::new("pkill")
            .args(["-x", proc])
            .status()
        {
            Ok(status) if status.success() => {
                info!("browser: killed leftover {proc} processes (restart recovery)");
            }
            Ok(_) => debug!("browser: no leftover {proc} processes"),
            Err(e) => warn!("browser: could not run pkill -x {proc}: {e}"),
        }
    }
}

/// Extract a Google OAuth consent URL from google-mcp output, if one is
/// present. google-mcp prints the interactive URL when its token is dead.
fn extract_consent_url(text: &str) -> Option<String> {
    static RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r#"https://accounts\.google\.com/o/oauth2/v2/auth[^\s"'<>]+"#)
            .expect("consent URL regex must compile")
    });
    RE.find(text).map(|m| m.as_str().to_string())
}

/// Heuristic: does this google-mcp output look like a successful search
/// (message IDs / an empty result) rather than an auth failure? The exact
/// failure text is the consent URL (handled above) or an error mentioning
/// auth; anything else is treated as a working search.
fn looks_like_search_success(text: &str) -> bool {
    let lower = text.to_lowercase();
    !(lower.contains("error") || lower.contains("auth") || lower.contains("consent"))
}

/// Human-readable expiry note for a google-mcp token file. The token
/// JSON shape varies (oauth2 Token: `expiry` RFC3339; some builds use
/// `expires_at` unix seconds); when neither parses, say so — the live
/// search in the same check is the real proof.
fn token_expiry_note(token: &Value) -> String {
    if let Some(expiry) = token.get("expiry").and_then(|v| v.as_str()) {
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(expiry) {
            let now = chrono::Utc::now();
            if dt.with_timezone(&chrono::Utc) > now {
                return format!("{expiry} (valid)");
            }
            return format!("{expiry} (EXPIRED — run the gmail_oauth flow)");
        }
        return format!("{expiry} (unparsed)");
    }
    if let Some(secs) = token.get("expires_at").and_then(|v| v.as_i64()) {
        if let Some(dt) = chrono::DateTime::from_timestamp(secs, 0) {
            let now = chrono::Utc::now();
            if dt > now {
                return format!("{} (valid)", dt.to_rfc3339());
            }
            return format!("{} (EXPIRED — run the gmail_oauth flow)", dt.to_rfc3339());
        }
    }
    "unknown (no expiry field parsed — the live search below is the real proof)".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled_config() -> BrowserConfig {
        BrowserConfig {
            enabled: true,
            ..BrowserConfig::default()
        }
    }

    fn session_with_fake() -> (BrowserSession, Arc<FakeDriver>) {
        let fake = Arc::new(FakeDriver::new());
        let session = BrowserSession::with_driver(enabled_config(), Arc::clone(&fake) as Arc<dyn BrowserDriver>);
        (session, fake)
    }

    #[tokio::test]
    async fn act_navigate_uses_driver_and_returns_url_and_title() {
        let (mut session, _fake) = session_with_fake();
        let out = session
            .act("navigate", &json!({"url": "https://accounts.google.com/"}))
            .await
            .expect("navigate");
        assert!(out.contains("https://accounts.google.com/"), "got: {out}");
        assert!(out.contains("Fake:"), "got: {out}");
    }

    #[tokio::test]
    async fn act_navigate_refuses_non_allowlisted_host() {
        let (mut session, fake) = session_with_fake();
        let err = session
            .act("navigate", &json!({"url": "https://evil.example.com/"}))
            .await
            .expect_err("non-allowlisted host must be refused");
        assert!(err.contains("allowlist"), "got: {err}");
        // The driver must never have been asked to navigate there.
        assert_eq!(
            fake.clicks().len(),
            0,
            "refused navigation must not touch the driver"
        );
        let url = fake.current_url().await.unwrap();
        assert_eq!(url, "about:blank", "driver URL must be untouched");
    }

    #[tokio::test]
    async fn act_click_and_type_drive_the_fake() {
        let (mut session, fake) = session_with_fake();
        session
            .act("click", &json!({"selector": "#signin"}))
            .await
            .expect("click");
        session
            .act(
                "type",
                &json!({"selector": "#code", "text": "123456"}),
            )
            .await
            .expect("type");
        assert_eq!(fake.clicks(), vec!["#signin".to_string()]);
        assert_eq!(
            fake.typed(),
            vec![("#code".to_string(), "123456".to_string())]
        );
    }

    #[tokio::test]
    async fn act_click_type_key_coordinate_forms() {
        // The tool schema advertises x/y for click/type (the same form
        // the 2FA box uses) — a coordinate call must reach the driver,
        // not fail on a missing "selector".
        let (mut session, fake) = session_with_fake();
        session
            .act("click", &json!({"x": 120, "y": 340}))
            .await
            .expect("coordinate click");
        assert_eq!(fake.input_clicks(), vec![(120, 340)]);
        // Coordinate type clicks to focus first, then types.
        session
            .act("type", &json!({"x": 120, "y": 340, "text": "123456"}))
            .await
            .expect("coordinate type");
        assert_eq!(fake.input_clicks(), vec![(120, 340), (120, 340)]);
        assert_eq!(fake.input_typed(), vec!["123456".to_string()]);
        // Bare type (no selector, no coords) types into the focused
        // element.
        session
            .act("type", &json!({"text": "abc"}))
            .await
            .expect("bare type");
        assert_eq!(
            fake.input_typed(),
            vec!["123456".to_string(), "abc".to_string()]
        );
        // Key action.
        session
            .act("key", &json!({"key": "Enter"}))
            .await
            .expect("key");
        assert_eq!(fake.input_keys(), vec!["Enter".to_string()]);
        // Click with neither selector nor coords is a usage error.
        let err = session
            .act("click", &json!({}))
            .await
            .expect_err("click without selector or coords must fail");
        assert!(err.contains("selector") && err.contains("x"), "got: {err}");
        // Unknown key is rejected by the key mapping.
        let err = session
            .act("key", &json!({"key": "Frobnicator"}))
            .await
            .expect_err("unknown key must fail");
        assert!(err.contains("unknown key"), "got: {err}");
    }

    #[tokio::test]
    async fn act_evaluate_refuses_password_scripts() {
        let (mut session, _fake) = session_with_fake();
        let err = session
            .act(
                "evaluate",
                &json!({"script": "document.querySelector('input[type=password]').value"}),
            )
            .await
            .expect_err("password-touching script must be refused");
        assert!(err.contains("password"), "got: {err}");
        let out = session
            .act("evaluate", &json!({"script": "return document.title"}))
            .await
            .expect("benign script must run");
        assert!(out.contains("fake-eval"), "got: {out}");
    }

    #[tokio::test]
    async fn act_screenshot_saves_to_artifacts_and_cleans_up() {
        let (mut session, _fake) = session_with_fake();
        let out = session
            .act("screenshot", &json!({}))
            .await
            .expect("screenshot");
        assert!(out.contains("/artifacts/browser-"), "got: {out}");
        assert!(out.contains("67 bytes"), "fake PNG is 67 bytes, got: {out}");
        let rel = out
            .split("Screenshot saved: ")
            .nth(1)
            .and_then(|r| r.split(' ').next())
            .unwrap_or("");
        let path = rel.trim_start_matches('/');
        assert!(std::path::Path::new(path).exists(), "screenshot file must exist");
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_dir("artifacts");
    }

    #[tokio::test]
    async fn act_status_does_not_start_a_browser() {
        let (mut session, _fake) = session_with_fake();
        let out = session.act("status", &json!({})).await.expect("status");
        assert!(out.contains("enabled"), "got: {out}");
        assert!(out.contains("running"), "got: {out}");
        // Status on a fresh (non-started) session must not launch Firefox:
        let mut fresh = BrowserSession::new(enabled_config());
        let out = fresh.act("status", &json!({})).await.expect("status");
        assert!(out.contains("not running"), "got: {out}");
    }

    #[tokio::test]
    async fn act_restart_restores_the_driver() {
        let (mut session, fake) = session_with_fake();
        let out = session.act("restart", &json!({})).await.expect("restart");
        assert!(out.contains("restarted"), "got: {out}");
        assert!(fake.was_shut_down(), "restart must shut the old driver down");
        // The session is usable again after restart.
        let out = session
            .act("navigate", &json!({"url": "https://mail.google.com/"}))
            .await
            .expect("navigate after restart");
        assert!(out.contains("mail.google.com"), "got: {out}");
    }

    #[tokio::test]
    async fn act_disabled_config_reports_disabled() {
        let mut session = BrowserSession::new(BrowserConfig::default());
        let err = session
            .act("navigate", &json!({"url": "https://accounts.google.com/"}))
            .await
            .expect_err("disabled browser must refuse");
        assert!(err.contains("disabled"), "got: {err}");
    }

    #[tokio::test]
    async fn act_unknown_action_lists_actions() {
        let (mut session, _fake) = session_with_fake();
        let err = session
            .act("explode", &json!({}))
            .await
            .expect_err("unknown action must error");
        assert!(err.contains("navigate"), "got: {err}");
    }

    #[tokio::test]
    async fn input_click_type_key_drive_the_fake() {
        let (mut session, fake) = session_with_fake();
        let out = session
            .input("click", Some(100), Some(200), None, None)
            .await
            .expect("input click");
        assert!(out.contains("(100, 200)"), "got: {out}");
        assert_eq!(fake.input_clicks(), vec![(100, 200)]);
        session
            .input("type", None, None, Some("123456"), None)
            .await
            .expect("input type");
        assert_eq!(fake.input_typed(), vec!["123456".to_string()]);
        session
            .input("key", None, None, None, Some("Enter"))
            .await
            .expect("input key");
        assert_eq!(fake.input_keys(), vec!["Enter".to_string()]);
    }

    #[tokio::test]
    async fn input_validates_coordinates_and_keys() {
        let (mut session, _fake) = session_with_fake();
        let err = session
            .input("click", Some(99999), Some(0), None, None)
            .await
            .expect_err("out-of-range click must be refused");
        assert!(err.contains("between 0 and 10000"), "got: {err}");
        let err = session
            .input("key", None, None, None, Some("F13"))
            .await
            .expect_err("unknown key must be refused");
        assert!(err.contains("unknown key"), "got: {err}");
        let err = session
            .input("type", None, None, Some(""), None)
            .await
            .expect_err("empty type must be refused");
        assert!(err.contains("non-empty"), "got: {err}");
    }

    #[test]
    fn consent_url_extracted_from_google_mcp_output() {
        let output = "Error: authentication required. Visit https://accounts.google.com/o/oauth2/v2/auth?client_id=abc&redirect_uri=http://localhost:51234&scope=openid to continue.";
        let url = extract_consent_url(output).expect("consent URL");
        assert!(url.starts_with("https://accounts.google.com/o/oauth2/v2/auth"));
        assert!(url.contains("redirect_uri=http://localhost:51234"));
        assert!(extract_consent_url("search results: 3 messages").is_none());
    }

    #[test]
    fn search_success_heuristic() {
        assert!(looks_like_search_success("id: 12345\nid: 67890"));
        assert!(looks_like_search_success("no messages found"));
        assert!(!looks_like_search_success("Error: auth required"));
        assert!(!looks_like_search_success("consent needed"));
    }

    #[test]
    fn token_expiry_note_parses_rfc3339() {
        let token = json!({"scope": "gmail.modify", "expiry": "2099-01-01T00:00:00Z"});
        assert!(token_expiry_note(&token).contains("valid"));
        let token = json!({"scope": "gmail.modify", "expiry": "2001-01-01T00:00:00Z"});
        assert!(token_expiry_note(&token).contains("EXPIRED"));
        let token = json!({"scope": "gmail.modify"});
        assert!(token_expiry_note(&token).contains("unknown"));
    }

    // ── Real-browser smoke test (ignored in the sandbox: no Firefox) ──
    // Run on the R630 after the install (task 356/357):
    //   cargo test -- --ignored browser_real_smoke
    #[tokio::test]
    #[ignore]
    async fn browser_real_smoke() {
        // GIVEN a real headless Firefox (installed by task 360) and the
        // manager able to download geckodriver:
        let config = BrowserConfig {
            enabled: true,
            ..BrowserConfig::default()
        };
        let mut session = BrowserSession::new(config);
        // WHEN navigating to a local page:
        let out = session
            .act("navigate", &json!({"url": "https://accounts.google.com/"}))
            .await
            .expect("real navigate must succeed on the R630");
        assert!(out.contains("accounts.google.com"), "got: {out}");
        // THEN the BiDi seam is connected and a screenshot works:
        assert!(session.is_started());
        let png = session.screenshot().await.expect("real screenshot");
        assert!(!png.is_empty());
        assert_eq!(&png[0..4], &[0x89, b'P', b'N', b'G'], "must be a PNG");
        session.stop().await.expect("stop");
    }
}
