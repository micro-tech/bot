//! Voice-box HTTP client (tasks 267.2, 268.2).
//!
//! Talks to the audio.cpp server's OpenAI-compatible endpoints:
//! - TTS: POST {base}/v1/audio/speech  {model, input, voice, response_format}
//!   -> WAV bytes.
//! - STT: POST {base}/v1/audio/transcriptions (multipart: file, model?, language?)
//!   -> {"text": "..."}.
//!
//! Secrets hygiene (task 265): failures log metadata only (status codes,
//! byte sizes, endpoint names). Audio bytes, spoken text and transcripts are
//! NEVER logged.

use log::warn;
use std::fmt;
use std::time::Duration;

use super::config::VoiceConfig;

/// Typed voice-box errors. Display strings are metadata-only by construction.
#[derive(Debug)]
pub enum VoiceError {
    /// Transport / protocol failure talking to the voice box.
    Http(String),
    /// The voice box did not answer within `timeout_ms`.
    Timeout,
    /// Non-2xx from the voice box.
    BadStatus(u16),
    /// TTS response was not a RIFF/WAVE payload.
    InvalidAudio(&'static str),
    /// STT returned no usable transcript.
    EmptyTranscript,
}

impl fmt::Display for VoiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VoiceError::Http(e) => write!(f, "voice box request failed: {e}"),
            VoiceError::Timeout => write!(f, "voice box timed out"),
            VoiceError::BadStatus(s) => write!(f, "voice box returned status {s}"),
            VoiceError::InvalidAudio(why) => write!(f, "voice box returned invalid audio ({why})"),
            VoiceError::EmptyTranscript => write!(f, "voice box returned an empty transcript"),
        }
    }
}

impl std::error::Error for VoiceError {}

fn client_for(cfg: &VoiceConfig) -> Result<reqwest::Client, VoiceError> {
    reqwest::Client::builder()
        .timeout(Duration::from_millis(cfg.timeout_ms.max(1_000)))
        .build()
        .map_err(|e| VoiceError::Http(e.to_string()))
}

fn map_reqwest_err(e: reqwest::Error) -> VoiceError {
    if e.is_timeout() {
        VoiceError::Timeout
    } else {
        VoiceError::Http(e.to_string())
    }
}

/// Synthesize `text` via /v1/audio/speech. Returns raw WAV bytes on success.
///
/// The WAV magic (RIFF....WAVE) is verified before returning — a JSON error
/// body from the server must never be handed to an audio player as "audio".
pub async fn speak(
    cfg: &VoiceConfig,
    text: &str,
    voice: Option<&str>,
) -> Result<Vec<u8>, VoiceError> {
    let client = client_for(cfg)?;
    let url = format!("{}/v1/audio/speech", cfg.base_url());
    let voice = voice.unwrap_or(&cfg.default_voice);
    let body = serde_json::json!({
        "model": cfg.tts_model,
        "input": text,
        "voice": voice,
        "response_format": "wav",
    });

    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            warn!("voice speak: request to {url} failed: {e}");
            map_reqwest_err(e)
        })?;
    let status = resp.status();
    if !status.is_success() {
        warn!("voice speak: {url} returned {status}");
        return Err(VoiceError::BadStatus(status.as_u16()));
    }
    let bytes = resp.bytes().await.map_err(|e| {
        warn!("voice speak: reading response body failed: {e}");
        map_reqwest_err(e)
    })?;
    verify_wav(&bytes)?;
    Ok(bytes.to_vec())
}

/// Transcribe audio via /v1/audio/transcriptions (multipart upload).
/// Returns the transcript text. `mime` should match the container
/// (e.g. "audio/wav", "audio/webm"); the server's ASR handles both.
pub async fn transcribe(
    cfg: &VoiceConfig,
    audio: &[u8],
    filename: &str,
    mime: &str,
    language: Option<&str>,
) -> Result<String, VoiceError> {
    let client = client_for(cfg)?;
    let url = format!("{}/v1/audio/transcriptions", cfg.base_url());

    let part = reqwest::multipart::Part::bytes(audio.to_vec())
        .file_name(filename.to_string())
        .mime_str(mime)
        .map_err(|e| VoiceError::Http(format!("bad mime type: {e}")))?;
    let mut form = reqwest::multipart::Form::new().part("file", part);
    if !cfg.stt_model.is_empty() {
        form = form.text("model", cfg.stt_model.clone());
    }
    if let Some(lang) = language {
        form = form.text("language", lang.to_string());
    }

    let resp = client
        .post(&url)
        .multipart(form)
        .send()
        .await
        .map_err(|e| {
            warn!("voice transcribe: request to {url} failed: {e}");
            map_reqwest_err(e)
        })?;
    let status = resp.status();
    if !status.is_success() {
        warn!("voice transcribe: {url} returned {status}");
        return Err(VoiceError::BadStatus(status.as_u16()));
    }
    let v: serde_json::Value = resp.json().await.map_err(|e| {
        warn!("voice transcribe: parsing response failed: {e}");
        map_reqwest_err(e)
    })?;
    let text = v
        .get("text")
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if text.is_empty() {
        warn!("voice transcribe: {url} returned an empty transcript");
        return Err(VoiceError::EmptyTranscript);
    }
    Ok(text)
}

/// Verify the 12-byte RIFF/WAVE header. Metadata-only errors.
fn verify_wav(bytes: &[u8]) -> Result<(), VoiceError> {
    if bytes.len() < 12 {
        return Err(VoiceError::InvalidAudio("too short for a WAV header"));
    }
    if &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(VoiceError::InvalidAudio("missing RIFF/WAVE magic"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::State, routing::post, Json, Router};
    use std::sync::Arc;

    fn test_cfg(port: u16) -> VoiceConfig {
        VoiceConfig {
            host: "127.0.0.1".into(),
            port,
            timeout_ms: 5_000,
            ..Default::default()
        }
    }

    /// Minimal canned WAV: 12-byte header + 4 payload bytes.
    fn fake_wav() -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&[0u8; 4]); // chunk size placeholder
        v.extend_from_slice(b"WAVE");
        v.extend_from_slice(&[1, 2, 3, 4]);
        v
    }

    /// Spawn a mock voice box on 127.0.0.1:0; returns the bound port.
    /// The handler asserts the TTS request shape, then serves canned audio.
    async fn mock_tts_server(mode: Arc<str>) -> u16 {
        let app = Router::new().route(
            "/v1/audio/speech",
            post(
                |State(mode): State<Arc<str>>,
                 Json(body): Json<serde_json::Value>| async move {
                    match mode.as_ref() {
                        "ok" => {
                            assert_eq!(body["model"], "supertonic-tts");
                            assert_eq!(body["input"], "hello");
                            assert_eq!(body["voice"], "M1");
                            assert_eq!(body["response_format"], "wav");
                            (
                                axum::http::StatusCode::OK,
                                [(axum::http::header::CONTENT_TYPE, "audio/wav")],
                                fake_wav(),
                            )
                                .into_response()
                        }
                        "bad-json" => (
                            axum::http::StatusCode::OK,
                            [(axum::http::header::CONTENT_TYPE, "application/json")],
                            br#"{"error":"nope"}"#.to_vec(),
                        )
                            .into_response(),
                        _ => (
                            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                            "boom",
                        )
                            .into_response(),
                    }
                },
            ),
        );
        let app = app.with_state(mode);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        port
    }

    use axum::response::IntoResponse;

    #[tokio::test]
    async fn speak_returns_wav_on_ok() {
        let port = mock_tts_server(Arc::from("ok")).await;
        let out = speak(&test_cfg(port), "hello", None).await.unwrap();
        assert_eq!(out, fake_wav());
    }

    #[tokio::test]
    async fn speak_rejects_non_wav_body() {
        let port = mock_tts_server(Arc::from("bad-json")).await;
        let err = speak(&test_cfg(port), "hello", None).await.unwrap_err();
        assert!(matches!(err, VoiceError::InvalidAudio(_)), "{err}");
    }

    #[tokio::test]
    async fn speak_maps_5xx_to_bad_status() {
        let port = mock_tts_server(Arc::from("boom")).await;
        let err = speak(&test_cfg(port), "hello", None).await.unwrap_err();
        assert!(matches!(err, VoiceError::BadStatus(500)), "{err}");
    }

    #[tokio::test]
    async fn speak_voice_override_reaches_server() {
        let app = Router::new().route(
            "/v1/audio/speech",
            post(|Json(body): Json<serde_json::Value>| async move {
                assert_eq!(body["voice"], "F1");
                (
                    axum::http::StatusCode::OK,
                    [(axum::http::header::CONTENT_TYPE, "audio/wav")],
                    fake_wav(),
                )
                    .into_response()
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let out = speak(&test_cfg(port), "hello", Some("F1")).await.unwrap();
        assert_eq!(out, fake_wav());
    }

    /// Mock transcriptions endpoint: requires multipart, returns a transcript.
    async fn mock_stt_server() -> u16 {
        let app = Router::new().route(
            "/v1/audio/transcriptions",
            post(
                |req: axum::extract::Request| async move {
                    let ct = req
                        .headers()
                        .get(axum::http::header::CONTENT_TYPE)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string();
                    assert!(
                        ct.starts_with("multipart/form-data"),
                        "expected multipart, got {ct}"
                    );
                    axum::Json(serde_json::json!({"text": "hello world"}))
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        port
    }

    #[tokio::test]
    async fn transcribe_returns_text() {
        let port = mock_stt_server().await;
        let text = transcribe(&test_cfg(port), &[1, 2, 3], "audio.wav", "audio/wav", None)
            .await
            .unwrap();
        assert_eq!(text, "hello world");
    }

    #[tokio::test]
    async fn transcribe_empty_text_is_error() {
        let app = Router::new().route(
            "/v1/audio/transcriptions",
            post(|| async { axum::Json(serde_json::json!({"text": "  "})) }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let err = transcribe(&test_cfg(port), &[1, 2, 3], "a.wav", "audio/wav", None)
            .await
            .unwrap_err();
        assert!(matches!(err, VoiceError::EmptyTranscript), "{err}");
    }

    #[test]
    fn verify_wav_accepts_valid_header() {
        assert!(verify_wav(&fake_wav()).is_ok());
    }

    #[test]
    fn verify_wav_rejects_garbage() {
        assert!(verify_wav(b"not audio at all!!!!!!!!").is_err());
        assert!(verify_wav(b"short").is_err());
        assert!(verify_wav(b"RIFF\x00\x00\x00\x00XXXX....").is_err());
    }
}
