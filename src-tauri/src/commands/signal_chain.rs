use crate::sync::LockOrRecover;
use kicks_core::persistence;
use kicks_core::signal_chain::SignalChain;
use serde::Serialize;
use tauri::State;

use crate::{AppState, UNDO_LIMIT};

/// Information about a slot in the current signal chain.
#[derive(Debug, Serialize)]
pub struct ChainSlotInfo {
    pub id: String,
    pub plugin_type: String,
    pub enabled: bool,
    pub wet_dry: f32,
    pub parameters: std::collections::HashMap<String, f32>,
}

/// Full signal chain snapshot for the frontend.
#[derive(Debug, Serialize)]
pub struct ChainSnapshot {
    pub slots: Vec<ChainSlotInfo>,
}

/// Persist the signal chain to disk (best-effort).
fn persist_chain(chain: &SignalChain) {
    if let Err(e) = persistence::save_signal_chain(chain) {
        tracing::warn!("Failed to persist signal chain: {}", e);
    }
}

/// Save the current signal chain state onto the undo stack and clear redo.
///
/// Call this **before** any mutation to enable undo for that operation.
pub fn push_undo_state(state: &AppState) {
    let current = state.signal_chain.lock_or_recover().clone();
    let mut undo = state.undo_chain.lock_or_recover();
    undo.push(current);
    if undo.len() > UNDO_LIMIT {
        undo.remove(0);
    }
    // Any new mutation invalidates the redo stack
    let mut redo = state.redo_chain.lock_or_recover();
    redo.clear();
}

/// Get the current signal chain.
#[tauri::command]
pub fn get_signal_chain(state: State<'_, AppState>) -> ChainSnapshot {
    let chain = state.signal_chain.lock_or_recover();
    let slots = chain
        .slots
        .iter()
        .map(|s| ChainSlotInfo {
            id: s.id.clone(),
            plugin_type: format!("{:?}", s.plugin_type),
            enabled: s.enabled,
            wet_dry: s.wet_dry,
            parameters: s.parameters.clone(),
        })
        .collect();
    ChainSnapshot { slots }
}

/// Reset the signal chain to the default (Boost → Amp → Cab → Delay → Reverb).
#[tauri::command]
pub fn build_default_chain(state: State<'_, AppState>) -> Result<(), String> {
    push_undo_state(&state);
    let mut chain = state.signal_chain.lock().map_err(|e| e.to_string())?;
    *chain = SignalChain::default();
    persist_chain(&chain);
    // Rebuild the running engine's chain to match the default signal chain
    if let Ok(engine_guard) = state.engine.try_lock() {
        if let Some(ref eng_arc) = *engine_guard {
            if let Ok(mut eng) = eng_arc.try_lock() {
                eng.build_default_chain();
                for slot in &chain.slots {
                    eng.set_plugin_enabled(&slot.id, slot.enabled);
                    for (param_id, value) in &slot.parameters {
                        eng.set_parameter_on_plugin(&slot.id, param_id, *value);
                    }
                }
            }
        }
    }
    tracing::info!("Signal chain reset to default");
    Ok(())
}

/// Update a plugin slot's parameters in the signal chain.
#[tauri::command]
pub fn update_slot(
    state: State<'_, AppState>, slot_id: String, enabled: Option<bool>, wet_dry: Option<f32>,
    parameters: Option<std::collections::HashMap<String, f32>>,
) -> Result<(), String> {
    push_undo_state(&state);
    let mut chain = state.signal_chain.lock().map_err(|e| e.to_string())?;

    let slot_idx = chain.slots.iter().position(|s| s.id == slot_id);
    match slot_idx {
        Some(idx) => {
            let slot = &mut chain.slots[idx];
            let plugin_name = slot.id.clone();
            if let Some(en) = enabled {
                slot.enabled = en;
                // Also apply to the running engine
                if let Ok(engine_guard) = state.engine.try_lock() {
                    if let Some(ref eng_arc) = *engine_guard {
                        if let Ok(mut eng) = eng_arc.try_lock() {
                            eng.set_plugin_enabled(&plugin_name, en);
                        }
                    }
                }
            }
            if let Some(wd) = wet_dry {
                slot.wet_dry = wd.clamp(0.0, 1.0);
            }
            if let Some(params) = parameters {
                for (key, val) in &params {
                    slot.parameters.insert(key.clone(), val.clamp(0.0, 1.0));
                    // Also apply to the running engine
                    if let Ok(engine_guard) = state.engine.try_lock() {
                        if let Some(ref eng_arc) = *engine_guard {
                            if let Ok(mut eng) = eng_arc.try_lock() {
                                eng.set_parameter_on_plugin(&plugin_name, key, *val);
                            }
                        }
                    }
                }
            }
            persist_chain(&chain);
            Ok(())
        }
        None => Err(format!("Slot '{}' not found", slot_id)),
    }
}

/// Toggle a slot's enabled state.
#[tauri::command]
pub fn toggle_slot(state: State<'_, AppState>, slot_id: String) -> Result<bool, String> {
    push_undo_state(&state);
    let mut chain = state.signal_chain.lock().map_err(|e| e.to_string())?;

    let slot_idx = chain.slots.iter().position(|s| s.id == slot_id);
    match slot_idx {
        Some(idx) => {
            chain.slots[idx].enabled = !chain.slots[idx].enabled;
            let new_state = chain.slots[idx].enabled;
            let plugin_name = chain.slots[idx].id.clone();
            // Also apply to the running engine
            if let Ok(engine_guard) = state.engine.try_lock() {
                if let Some(ref eng_arc) = *engine_guard {
                    if let Ok(mut eng) = eng_arc.try_lock() {
                        eng.set_plugin_enabled(&plugin_name, new_state);
                    }
                }
            }
            persist_chain(&chain);
            Ok(new_state)
        }
        None => Err(format!("Slot '{}' not found", slot_id)),
    }
}

/// Move a slot from one index to another in the signal chain (drag-and-drop reorder).
///
/// `from_idx` and `to_idx` are 0-based positions. Only movable plugin slots
/// (non-Input/non-Output) can be reordered; fixed slots are skipped.
#[tauri::command]
pub fn move_slot(state: State<'_, AppState>, from_idx: usize, to_idx: usize) -> Result<(), String> {
    push_undo_state(&state);
    let mut chain = state.signal_chain.lock().map_err(|e| e.to_string())?;

    let len = chain.slots.len();
    if from_idx >= len || to_idx >= len {
        return Err(format!(
            "Invalid indices: from={}, to={}, len={}",
            from_idx, to_idx, len
        ));
    }
    if from_idx == to_idx {
        return Ok(());
    }

    // Don't allow moving fixed slots (Input/Output)
    let from_type = &chain.slots[from_idx].plugin_type;
    if *from_type == kicks_core::signal_chain::PluginType::Input
        || *from_type == kicks_core::signal_chain::PluginType::Output
    {
        return Err("Cannot move Input or Output slots".to_string());
    }

    let removed = chain.slots.remove(from_idx);
    // If moving forward, to_idx shifts by -1 after removal
    let insert_at = if from_idx < to_idx {
        to_idx - 1
    } else {
        to_idx
    };
    chain.slots.insert(insert_at, removed);

    tracing::info!("Moved slot from index {} to {}", from_idx, to_idx);
    persist_chain(&chain);
    Ok(())
}

/// Undo the last signal chain change.
#[tauri::command]
pub fn undo_signal_chain(state: State<'_, AppState>) -> Result<(), String> {
    let mut undo = state.undo_chain.lock().map_err(|e| e.to_string())?;
    let previous = match undo.pop() {
        Some(s) => s,
        None => return Err("Nothing to undo".to_string()),
    };

    // Save current state onto the redo stack
    {
        let current = state.signal_chain.lock().map_err(|e| e.to_string())?;
        let mut redo = state.redo_chain.lock().map_err(|e| e.to_string())?;
        redo.push(current.clone());
        if redo.len() > UNDO_LIMIT {
            redo.remove(0);
        }
    }

    // Restore the previous state
    {
        let mut chain = state.signal_chain.lock().map_err(|e| e.to_string())?;
        *chain = previous;
        persist_chain(&chain);
    }

    tracing::info!("Undo: signal chain restored");
    Ok(())
}

/// Redo a previously undone signal chain change.
#[tauri::command]
pub fn redo_signal_chain(state: State<'_, AppState>) -> Result<(), String> {
    let mut redo = state.redo_chain.lock().map_err(|e| e.to_string())?;
    let next = match redo.pop() {
        Some(s) => s,
        None => return Err("Nothing to redo".to_string()),
    };

    // Save current state onto the undo stack
    {
        let current = state.signal_chain.lock().map_err(|e| e.to_string())?;
        let mut undo = state.undo_chain.lock().map_err(|e| e.to_string())?;
        undo.push(current.clone());
        if undo.len() > UNDO_LIMIT {
            undo.remove(0);
        }
    }

    // Restore the next state
    {
        let mut chain = state.signal_chain.lock().map_err(|e| e.to_string())?;
        *chain = next;
        persist_chain(&chain);
    }

    tracing::info!("Redo: signal chain restored");
    Ok(())
}
