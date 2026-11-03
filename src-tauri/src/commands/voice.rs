use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::voice::{audio, model, transcribe};
use serde::Serialize;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct VoiceStatus {
    /// The model is downloaded and voice input can be used.
    ready: bool,
    downloading: bool,
    /// Download size, for the first-use prompt.
    model_bytes: u64,
}

#[derive(Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Progress {
    received: u64,
    total: u64,
}

#[tauri::command]
#[specta::specta]
pub fn voice_status(state: State<'_, AppState>) -> VoiceStatus {
    VoiceStatus {
        ready: model::is_downloaded(&state.voice.model_path),
        downloading: state.voice.downloading.load(Ordering::Relaxed),
        model_bytes: model::MODEL_BYTES,
    }
}

/// Downloads the Whisper model once, from Hugging Face, checking its pinned SHA-256.
/// Progress arrives as `voice://progress` events.
#[tauri::command]
#[specta::specta]
pub async fn voice_download(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    let voice = &state.voice;
    if voice.downloading.swap(true, Ordering::SeqCst) {
        return Err("The voice model is already downloading.".into());
    }
    voice.cancel_download.store(false, Ordering::SeqCst);
    let mut last = Instant::now() - Duration::from_secs(1);
    let result = model::download(
        &state.http,
        model::MODEL_URL,
        model::MODEL_SHA256,
        &voice.model_path,
        &voice.cancel_download,
        |received, total| {
            // About 10 updates a second is plenty for a progress bar.
            if last.elapsed() >= Duration::from_millis(100) || received == total {
                last = Instant::now();
                let _ = app.emit("voice://progress", Progress { received, total });
            }
        },
    )
    .await;
    voice.downloading.store(false, Ordering::SeqCst);
    result
}

#[tauri::command]
#[specta::specta]
pub fn voice_cancel_download(state: State<'_, AppState>) {
    state.voice.cancel_download.store(true, Ordering::SeqCst);
}

#[tauri::command]
#[specta::specta]
pub fn voice_delete_model(state: State<'_, AppState>) -> AppResult<()> {
    state.voice.remove_model()
}

/// Starts recording from the default microphone. While it runs, `voice://level` events
/// carry the loudness (0–1); `voice://blocked` means the system is withholding the audio.
#[tauri::command]
#[specta::specta]
pub fn voice_start(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    let emitter = app.clone();
    state.voice.start(move |event| {
        let _ = match event {
            audio::Event::Level(level) => emitter.emit("voice://level", level),
            audio::Event::Blocked => emitter.emit("voice://blocked", audio::MIC_DENIED),
        };
    })?;
    // Load the model while the person talks, so stopping is quick.
    let preload = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _ = preload.state::<AppState>().voice.context();
    });
    Ok(())
}

/// Stops recording and returns what was said. `prompt` lists table and column names to
/// help Whisper spell them. Runs on a worker thread; the audio is dropped afterwards.
#[tauri::command]
#[specta::specta]
pub async fn voice_stop(app: AppHandle, prompt: Option<String>) -> AppResult<String> {
    tauri::async_runtime::spawn_blocking(move || {
        let voice = &app.state::<AppState>().voice;
        let recording = voice.stop().ok_or("Not recording.")?;
        let audio = recording.finish()?;
        let ctx = voice.context()?;
        transcribe::transcribe(&ctx, &audio, prompt.as_deref().unwrap_or(""))
    })
    .await
    .map_err(|e| AppError::User(format!("Transcription failed: {e}")))?
}

/// Stops recording and throws the audio away.
#[tauri::command]
#[specta::specta]
pub fn voice_cancel(state: State<'_, AppState>) {
    drop(state.voice.stop());
}
