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
    pub error: Option<String>,
    pub path: Option<String>,
    pub seconds: f64,
    pub dropped_frames: u64,
}

fn recordings_dir() -> std::path::PathBuf {
    kicks_core::persistence::config_dir().join("recordings")
}

pub(crate) fn start_recording_in(state: &AppState, dir: &Path) -> Result<String, String> {
    let mut slot = state.recording.lock_or_recover();
    if slot.as_ref().is_some_and(|rec| rec.completion().is_some()) {
        let _ = stop_recording_locked(state, &mut slot);
    }
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
    let mut slot = state.recording.lock_or_recover();
    stop_recording_locked(state, &mut slot)
}

pub(crate) fn stop_recording_locked(
    state: &AppState, slot: &mut Option<Recording>,
) -> Result<RecordingSummary, String> {
    let rec = slot.take().ok_or("Not recording")?;
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
            let completion = rec.completion();
            if completion.is_some() {
                if let Some(engine) = state.engine.lock_or_recover().clone() {
                    let tap = engine.lock_or_recover().set_record_tap(None);
                    drop(tap);
                }
            }
            RecordingStatus {
                recording: completion.is_none(),
                error: completion.and_then(Result::err),
                path: Some(rec.path().display().to_string()),
                seconds: rec.frames() as f64 / sr,
                dropped_frames: rec.dropped_frames(),
            }
        }
        None => RecordingStatus {
            recording: false,
            error: None,
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
    fn engine_shutdown_finalizes_recording_and_allows_recording_after_restart() {
        let state = AppState::for_test();
        let mut eng = KicksEngine::new();
        eng.init(48_000.0, 256).unwrap();
        let eng = Arc::new(Mutex::new(eng));
        *state.engine.lock().unwrap() = Some(eng.clone());
        let path = start_recording_in(&state, &dir().join("shutdown")).unwrap();
        let (_tx, mut rx) = param_channel();
        eng.lock()
            .unwrap()
            .process_block(&mut rx, &[0.1; 480], &mut [0.0; 480]);
        super::super::engine::stop_engine_impl(&state).unwrap();
        assert!(state.engine.lock().unwrap().is_none());
        assert!(state.recording.lock().unwrap().is_none());
        assert!(!eng.lock().unwrap().is_recording());
        assert!(!recording_status_impl(&state).recording);
        assert_eq!(hound::WavReader::open(&path).unwrap().duration(), 480);

        let mut restarted = KicksEngine::new();
        restarted.init(48_000.0, 256).unwrap();
        let restarted = Arc::new(Mutex::new(restarted));
        *state.engine.lock().unwrap() = Some(restarted.clone());
        let next = start_recording_in(&state, &dir().join("shutdown")).unwrap();
        assert!(recording_status_impl(&state).recording);
        restarted
            .lock()
            .unwrap()
            .process_block(&mut rx, &[0.2; 256], &mut [0.0; 256]);
        assert_eq!(stop_recording_impl(&state).unwrap().frames, 256);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(next).unwrap();
    }

    #[cfg(target_os = "linux")]
    fn failed_recording() -> (AppState, Arc<Mutex<KicksEngine>>) {
        let state = AppState::for_test();
        let mut eng = KicksEngine::new();
        eng.init(48_000.0, 256).unwrap();
        // The buffered header succeeds; /dev/full fails when audio is flushed.
        let (tap, rec) = Recording::start(Path::new("/dev/full"), 48_000).unwrap();
        eng.set_record_tap(Some(tap));
        let eng = Arc::new(Mutex::new(eng));
        *state.engine.lock().unwrap() = Some(eng.clone());
        *state.recording.lock().unwrap() = Some(rec);
        let (_tx, mut rx) = param_channel();
        for _ in 0..32 {
            eng.lock()
                .unwrap()
                .process_block(&mut rx, &[0.1; 256], &mut [0.0; 256]);
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while state
            .recording
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .completion()
            .is_none()
        {
            assert!(std::time::Instant::now() < deadline, "writer did not fail");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        (state, eng)
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn status_exposes_writer_failure_detaches_tap_and_allows_restart() {
        let (state, eng) = failed_recording();
        assert!(eng.lock().unwrap().is_recording());
        let status = recording_status_impl(&state);
        assert!(!status.recording);
        assert!(status.error.is_some());
        assert!(!eng.lock().unwrap().is_recording());
        // The error remains visible across polls until another take starts.
        assert_eq!(recording_status_impl(&state).error, status.error);
        let path = start_recording_in(&state, &dir().join("recovery")).unwrap();
        assert!(recording_status_impl(&state).recording);
        assert!(recording_status_impl(&state).error.is_none());
        stop_recording_impl(&state).unwrap();
        std::fs::remove_file(path).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn shutdown_clears_engine_and_recording_even_when_writer_failed() {
        let (state, eng) = failed_recording();
        assert!(super::super::engine::stop_engine_impl(&state).is_err());
        assert!(state.engine.lock().unwrap().is_none());
        assert!(state.recording.lock().unwrap().is_none());
        assert!(!eng.lock().unwrap().is_recording());
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
