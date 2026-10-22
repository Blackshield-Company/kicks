//! WAV recording / DI capture commands.
//!
//! Files go to `<config dir>/recordings/kicks-<time>.wav`: channel 1 is the
//! dry DI (for re-amping), channel 2 is the processed sound. The path is
//! chosen by the backend, never by the caller.

use crate::sync::LockOrRecover;
use crate::AppState;
use kicks_dsp::recorder::{timestamped_path, Recording, RecordingSummary};
use serde::Serialize;
use std::path::Path;
use tauri::State;

#[derive(Debug, Serialize, PartialEq)]
pub struct RecordingStatus {
    pub recording: bool,
    pub path: Option<String>,
    pub seconds: f64,
    pub dropped_frames: u64,
}

fn recordings_dir() -> std::path::PathBuf {
    kicks_core::persistence::config_dir().join("recordings")
}

pub(crate) fn start_recording_in(state: &AppState, dir: &Path) -> Result<String, String> {
    let mut slot = state.recording.lock_or_recover();
    if slot.is_some() {
        return Err("Already recording".into());
    }
    let engine = state
        .engine
        .lock_or_recover()
        .clone()
        .ok_or("Start the audio engine before recording")?;
    let sample_rate = engine.lock_or_recover().sample_rate() as u32;
    std::fs::create_dir_all(dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
    let path = timestamped_path(dir);
    let (tap, rec) = Recording::start(&path, sample_rate).map_err(|e| e.to_string())?;
    let old = engine.lock_or_recover().set_record_tap(Some(tap));
    drop(old);
    let shown = path.display().to_string();
    tracing::info!("Recording to {}", shown);
    *slot = Some(rec);
    Ok(shown)
}

pub(crate) fn stop_recording_impl(state: &AppState) -> Result<RecordingSummary, String> {
    let rec = state
        .recording
        .lock_or_recover()
        .take()
        .ok_or("Not recording")?;
    if let Some(engine) = state.engine.lock_or_recover().clone() {
        // Remove the tap under the lock, drop it after.
        let tap = engine.lock_or_recover().set_record_tap(None);
        drop(tap);
    }
    let summary = rec.stop().map_err(|e| e.to_string())?;
    if summary.dropped_frames > 0 {
        tracing::warn!("Recording dropped {} frames", summary.dropped_frames);
    }
    Ok(summary)
}

pub(crate) fn recording_status_impl(state: &AppState) -> RecordingStatus {
    let guard = state.recording.lock_or_recover();
    match guard.as_ref() {
        Some(rec) => {
            let sr = state
                .engine
                .lock_or_recover()
                .clone()
                .map(|e| e.lock_or_recover().sample_rate())
                .unwrap_or(48_000.0);
            RecordingStatus {
                recording: true,
                path: Some(rec.path().display().to_string()),
                seconds: rec.frames() as f64 / sr,
                dropped_frames: rec.dropped_frames(),
            }
        }
        None => RecordingStatus {
            recording: false,
            path: None,
            seconds: 0.0,
            dropped_frames: 0,
        },
    }
}

/// Start recording DI + processed audio. Returns the file path.
#[tauri::command]
pub fn start_recording(state: State<'_, AppState>) -> Result<String, String> {
    start_recording_in(&state, &recordings_dir())
}

/// Stop recording and finalise the WAV file.
#[tauri::command]
pub fn stop_recording(state: State<'_, AppState>) -> Result<RecordingSummary, String> {
    stop_recording_impl(&state)
}

#[tauri::command]
pub fn recording_status(state: State<'_, AppState>) -> RecordingStatus {
    recording_status_impl(&state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kicks_dsp::engine::AudioEngine;
    use kicks_dsp::param::param_channel;
    use kicks_dsp::KicksEngine;
    use std::sync::{Arc, Mutex};

    fn dir() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("kicks-rec-cmd-{}", std::process::id()))
    }

    #[test]
    fn needs_a_running_engine() {
        let state = AppState::for_test();
        assert!(start_recording_in(&state, &dir()).is_err());
        assert!(stop_recording_impl(&state).is_err());
        assert!(!recording_status_impl(&state).recording);
    }

    #[test]
    fn start_process_stop_writes_a_wav() {
        let state = AppState::for_test();
        let mut eng = KicksEngine::new();
        eng.init(48_000.0, 256).unwrap();
        let eng = Arc::new(Mutex::new(eng));
        *state.engine.lock().unwrap() = Some(eng.clone());

        let path = start_recording_in(&state, &dir()).unwrap();
        assert!(start_recording_in(&state, &dir()).is_err(), "double start");
        let (_tx, mut rx) = param_channel();
        let input = [0.1f32; 480];
        let mut out = [0.0f32; 480];
        for _ in 0..100 {
            eng.lock().unwrap().process_block(&mut rx, &input, &mut out);
        }
        let st = recording_status_impl(&state);
        assert!(st.recording);
        assert_eq!(st.path.as_deref(), Some(path.as_str()));

        let sum = stop_recording_impl(&state).unwrap();
        assert_eq!(sum.frames, 48_000);
        assert!((sum.seconds - 1.0).abs() < 1e-9);
        assert!(!eng.lock().unwrap().is_recording());
        let r = hound::WavReader::open(&path).unwrap();
        assert_eq!(r.spec().channels, 2);
        assert_eq!(r.duration(), 48_000);
        std::fs::remove_file(&path).ok();
    }
}
