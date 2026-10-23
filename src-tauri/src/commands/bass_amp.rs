use kicks_dsp::{build_registry, ChainKind};
use tauri::State;

use crate::AppState;

/// Build a chain off the engine lock, then swap it in.
///
/// The audio callback only ever `try_lock`s the engine, so holding the lock
/// while allocating a chain (the old behaviour) caused audible dropouts.
/// Here the lock is held only for an O(1) pointer swap, and the old chain is
/// dropped after the lock is released.
fn switch_chain(state: &AppState, kind: ChainKind) -> Result<(), String> {
    let eng_arc = {
        let guard = state.engine.lock().map_err(|e| e.to_string())?;
        guard
            .clone()
            .ok_or_else(|| "Engine not running".to_string())?
    };
    let sample_rate = eng_arc.lock().map_err(|e| e.to_string())?.sample_rate();
    let new_chain = build_registry(kind, sample_rate).map_err(|e| e.to_string())?;
    let old_chain = {
        let mut eng = eng_arc.lock().map_err(|e| e.to_string())?;
        eng.swap_registry(new_chain)
    };
    drop(old_chain);
    tracing::info!("Switched to {:?} signal chain", kind);
    Ok(())
}

/// Switch the signal chain to bass mode (BassAmp instead of regular Amp).
#[tauri::command]
pub fn switch_to_bass_chain(state: State<'_, AppState>) -> Result<(), String> {
    switch_chain(&state, ChainKind::Bass)
}

/// Switch the signal chain to practice mode (Tuner + Metronome + Amp + Cab).
#[tauri::command]
pub fn switch_to_practice_chain(state: State<'_, AppState>) -> Result<(), String> {
    switch_chain(&state, ChainKind::Practice)
}

/// Switch the signal chain to looper mode.
#[tauri::command]
pub fn switch_to_looper_chain(state: State<'_, AppState>) -> Result<(), String> {
    switch_chain(&state, ChainKind::Looper)
}
