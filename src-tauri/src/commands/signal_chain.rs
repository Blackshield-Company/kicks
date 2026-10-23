use crate::sync::LockOrRecover;
use kicks_core::persistence;
use kicks_core::signal_chain::{ChainSlot, SignalChain};
use kicks_dsp::param::ParamSendError;
use kicks_dsp::plugins::WET_DRY_PARAM;
use kicks_dsp::KicksEngine;
use serde::Serialize;
use std::sync::{Arc, Mutex};
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
    if cfg!(test) {
        // Unit tests must never touch the user's real config directory.
        return;
    }
    if let Err(e) = persistence::save_signal_chain(chain) {
        tracing::warn!("Failed to persist signal chain: {}", e);
    }
}

/// The engine-side id for a slot parameter (`"slot/param"`).
fn slot_key(slot_id: &str, param: &str) -> String {
    format!("{slot_id}/{param}")
}

/// Every engine message needed to make the running engine match `slot`.
pub(crate) fn slot_messages(slot: &ChainSlot) -> Vec<(String, f32)> {
    let mut msgs = Vec::with_capacity(slot.parameters.len() + 2);
    msgs.push((
        slot_key(&slot.id, "enabled"),
        if slot.enabled { 1.0 } else { 0.0 },
    ));
    msgs.push((slot_key(&slot.id, WET_DRY_PARAM), slot.wet_dry));
    for (k, v) in &slot.parameters {
        msgs.push((slot_key(&slot.id, k), *v));
    }
    msgs
}

/// Running engine handle, if audio is started. Clones the `Arc` so the
/// `AppState::engine` lock is released immediately.
fn running_engine(state: &AppState) -> Option<Arc<Mutex<KicksEngine>>> {
    state.engine.lock_or_recover().clone()
}

/// Send parameter changes to the running engine.
///
/// Uses the lock-free param queue when audio is running (the audio thread
/// applies them at the next block). Full queues are retried in order on
/// this control thread. Without a queue, changes use the engine lock.
/// Previously these commands used nested `try_lock`s and silently dropped the change
/// whenever the audio callback held the lock.
pub(crate) fn push_to_engine(state: &AppState, msgs: &[(String, f32)]) {
    let mut pending: Vec<&(String, f32)> = Vec::new();
    {
        let tx = state.param_tx.lock_or_recover();
        match tx.as_ref() {
            Some(tx) => {
                for m in msgs {
                    loop {
                        match tx.send(&m.0, m.1) {
                            Ok(()) => break,
                            Err(ParamSendError::Full) => {
                                std::thread::sleep(std::time::Duration::from_millis(1));
                            }
                            Err(ParamSendError::KeyTooLong) => {
                                // This key cannot have an older queued update.
                                pending.push(m);
                                break;
                            }
                        }
                    }
                }
            }
            None => pending.extend(msgs.iter()),
        }
    }
    if pending.is_empty() {
        return;
    }
    if let Some(eng) = running_engine(state) {
        let mut eng = eng.lock_or_recover();
        for (k, v) in pending {
            eng.apply_param(k, *v);
        }
    }
}

/// Restore a complete chain before audio starts (no queue exists yet).
pub(crate) fn initialize_engine_chain(eng: &mut KicksEngine, chain: &SignalChain) {
    reorder_engine(eng, chain);
    for slot in &chain.slots {
        for (key, value) in slot_messages(slot) {
            eng.apply_param(&key, value);
        }
    }
}

fn reorder_engine(eng: &mut KicksEngine, chain: &SignalChain) {
    let order: Vec<&str> = chain.slots.iter().map(|s| s.id.as_str()).collect();
    eng.reorder_plugins(&order);
}

/// Make the running engine match `chain`: plugin order, bypass, wet/dry and
/// every parameter. Used after chain edits and preset/scene recall.
pub(crate) fn sync_engine_to_chain(state: &AppState, chain: &SignalChain) {
    let Some(eng) = running_engine(state) else {
        return;
    };
    reorder_engine(&mut eng.lock_or_recover(), chain);
    let msgs: Vec<(String, f32)> = chain.slots.iter().flat_map(slot_messages).collect();
    push_to_engine(state, &msgs);
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
    let snapshot = chain.clone();
    drop(chain);
    if let Some(eng) = running_engine(&state) {
        eng.lock_or_recover().build_default_chain();
    }
    sync_engine_to_chain(&state, &snapshot);
    tracing::info!("Signal chain reset to default");
    Ok(())
}

/// Update a plugin slot's parameters in the signal chain.
#[tauri::command]
pub fn update_slot(
    state: State<'_, AppState>, slot_id: String, enabled: Option<bool>, wet_dry: Option<f32>,
    parameters: Option<std::collections::HashMap<String, f32>>,
) -> Result<(), String> {
    update_slot_impl(&state, &slot_id, enabled, wet_dry, parameters)
}

pub(crate) fn update_slot_impl(
    state: &AppState, slot_id: &str, enabled: Option<bool>, wet_dry: Option<f32>,
    parameters: Option<std::collections::HashMap<String, f32>>,
) -> Result<(), String> {
    if !state
        .signal_chain
        .lock_or_recover()
        .slots
        .iter()
        .any(|s| s.id == slot_id)
    {
        return Err(format!("Slot '{}' not found", slot_id));
    }
    push_undo_state(state);
    let mut msgs = Vec::new();
    {
        let mut chain = state.signal_chain.lock_or_recover();
        let slot = chain
            .slots
            .iter_mut()
            .find(|s| s.id == slot_id)
            .ok_or_else(|| format!("Slot '{}' not found", slot_id))?;
        if let Some(en) = enabled {
            slot.enabled = en;
            msgs.push((slot_key(slot_id, "enabled"), if en { 1.0 } else { 0.0 }));
        }
        if let Some(wd) = wet_dry {
            slot.wet_dry = wd.clamp(0.0, 1.0);
            msgs.push((slot_key(slot_id, WET_DRY_PARAM), slot.wet_dry));
        }
        if let Some(params) = parameters {
            for (key, val) in params {
                let v = val.clamp(0.0, 1.0);
                slot.parameters.insert(key.clone(), v);
                msgs.push((slot_key(slot_id, &key), v));
            }
        }
        persist_chain(&chain);
    }
    push_to_engine(state, &msgs);
    Ok(())
}

/// Toggle a slot's enabled state.
#[tauri::command]
pub fn toggle_slot(state: State<'_, AppState>, slot_id: String) -> Result<bool, String> {
    toggle_slot_impl(&state, &slot_id)
}

pub(crate) fn toggle_slot_impl(state: &AppState, slot_id: &str) -> Result<bool, String> {
    let current = state
        .signal_chain
        .lock_or_recover()
        .slots
        .iter()
        .find(|s| s.id == slot_id)
        .map(|s| s.enabled)
        .ok_or_else(|| format!("Slot '{}' not found", slot_id))?;
    update_slot_impl(state, slot_id, Some(!current), None, None)?;
    Ok(!current)
}

/// Move a slot from one index to another in the signal chain (drag-and-drop reorder).
///
/// `from_idx` and `to_idx` are 0-based positions. Only movable plugin slots
/// (non-Input/non-Output) can be reordered; fixed slots are skipped.
#[tauri::command]
pub fn move_slot(state: State<'_, AppState>, from_idx: usize, to_idx: usize) -> Result<(), String> {
    move_slot_impl(&state, from_idx, to_idx)
}

pub(crate) fn move_slot_impl(
    state: &AppState, from_idx: usize, to_idx: usize,
) -> Result<(), String> {
    let chain = state.signal_chain.lock_or_recover();

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
    let to_type = &chain.slots[to_idx].plugin_type;
    if *to_type == kicks_core::signal_chain::PluginType::Input
        || *to_type == kicks_core::signal_chain::PluginType::Output
    {
        return Err("Cannot move a slot past Input or Output".to_string());
    }
    drop(chain);
    push_undo_state(state);
    let mut chain = state.signal_chain.lock_or_recover();

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
    let snapshot = chain.clone();
    drop(chain);
    sync_engine_to_chain(state, &snapshot);
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
    let snapshot = state.signal_chain.lock_or_recover().clone();
    sync_engine_to_chain(&state, &snapshot);

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
    let snapshot = state.signal_chain.lock_or_recover().clone();
    sync_engine_to_chain(&state, &snapshot);

    tracing::info!("Redo: signal chain restored");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kicks_dsp::engine::AudioEngine;
    use kicks_dsp::param::param_channel;

    fn running(state: &AppState) -> Arc<Mutex<KicksEngine>> {
        let mut eng = KicksEngine::new();
        eng.init(48_000.0, 256).unwrap();
        let eng = Arc::new(Mutex::new(eng));
        *state.engine.lock().unwrap() = Some(eng.clone());
        eng
    }

    fn param(eng: &Arc<Mutex<KicksEngine>>, id: &str) -> Option<f32> {
        eng.lock().unwrap().get_parameter(id)
    }

    #[test]
    fn update_slot_applies_even_while_audio_holds_the_lock() {
        let state = AppState::for_test();
        let eng = running(&state);
        let (tx, mut rx) = param_channel();
        *state.param_tx.lock().unwrap() = Some(tx);

        // Simulate the audio callback holding the engine lock. The old
        // nested try_lock code dropped the change in this situation.
        {
            let _audio = eng.lock().unwrap();
            let params = [("drive".to_string(), 0.2)].into_iter().collect();
            update_slot_impl(&state, "amp", Some(false), Some(0.4), Some(params)).unwrap();
        }
        // Next audio block drains the queue.
        let input = [0.0f32; 64];
        let mut out = [0.0f32; 64];
        eng.lock().unwrap().process_block(&mut rx, &input, &mut out);
        assert_eq!(param(&eng, "amp/enabled"), Some(0.0));
        assert_eq!(param(&eng, "amp/wet_dry"), Some(0.4));
        assert_eq!(param(&eng, "amp/drive"), Some(0.2));
        let chain = state.signal_chain.lock().unwrap();
        let amp = chain.slots.iter().find(|s| s.id == "amp").unwrap();
        assert!(!amp.enabled);
        assert_eq!(amp.wet_dry, 0.4);
    }

    #[test]
    fn full_queue_preserves_the_latest_update() {
        let state = AppState::for_test();
        let eng = running(&state);
        let (tx, mut rx) = param_channel();
        for _ in 0..kicks_dsp::param::PARAM_QUEUE_CAPACITY {
            tx.send("amp/drive", 0.1).unwrap();
        }
        *state.param_tx.lock().unwrap() = Some(tx);

        std::thread::scope(|scope| {
            let (done_tx, done_rx) = std::sync::mpsc::channel();
            let sender = scope.spawn(move || {
                push_to_engine(&state, &[("amp/drive".to_string(), 0.9)]);
                done_tx.send(()).unwrap();
            });
            // A full queue must backpressure the sender, rather than allowing
            // a direct write that the older queued value could overwrite.
            assert_eq!(
                done_rx.recv_timeout(std::time::Duration::from_millis(50)),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            );
            let input = [0.0; 64];
            let mut output = [0.0; 64];
            while !sender.is_finished() {
                eng.lock()
                    .unwrap()
                    .process_block(&mut rx, &input, &mut output);
                std::thread::yield_now();
            }
            sender.join().unwrap();
            eng.lock()
                .unwrap()
                .process_block(&mut rx, &input, &mut output);
        });
        assert_eq!(param(&eng, "amp/drive"), Some(0.9));
    }

    #[test]
    fn startup_and_recall_restore_complete_chain() {
        let mut chain = SignalChain::default();
        let delay = chain.slots.iter().position(|s| s.id == "delay").unwrap();
        let boost = chain.slots.iter().position(|s| s.id == "boost").unwrap();
        chain.slots.swap(delay, boost);
        let amp = chain.slots.iter_mut().find(|s| s.id == "amp").unwrap();
        amp.enabled = false;
        amp.wet_dry = 0.25;
        amp.parameters.insert("drive".to_string(), 0.7);

        for startup in [true, false] {
            let state = AppState::for_test();
            let eng = running(&state);
            if startup {
                initialize_engine_chain(&mut eng.lock().unwrap(), &chain);
            } else {
                let (tx, mut rx) = param_channel();
                *state.param_tx.lock().unwrap() = Some(tx);
                sync_engine_to_chain(&state, &chain);
                eng.lock()
                    .unwrap()
                    .process_block(&mut rx, &[0.0; 64], &mut [0.0; 64]);
            }
            assert_eq!(param(&eng, "amp/enabled"), Some(0.0));
            assert_eq!(param(&eng, "amp/wet_dry"), Some(0.25));
            assert_eq!(param(&eng, "amp/drive"), Some(0.7));
            let names = eng.lock().unwrap().plugin_names();
            assert!(
                names.iter().position(|s| s == "delay") < names.iter().position(|s| s == "boost")
            );
        }
    }

    #[test]
    fn update_slot_without_queue_applies_under_lock() {
        let state = AppState::for_test();
        let eng = running(&state);
        let first = toggle_slot_impl(&state, "delay").unwrap();
        let as_f = |b: bool| if b { 1.0 } else { 0.0 };
        assert_eq!(param(&eng, "delay/enabled"), Some(as_f(first)));
        let second = toggle_slot_impl(&state, "delay").unwrap();
        assert_eq!(second, !first);
        assert_eq!(param(&eng, "delay/enabled"), Some(as_f(second)));
    }

    #[test]
    fn unknown_slot_is_an_error_and_not_undoable() {
        let state = AppState::for_test();
        assert!(update_slot_impl(&state, "nope", Some(true), None, None).is_err());
        assert!(state.undo_chain.lock().unwrap().is_empty());
    }

    #[test]
    fn move_slot_reorders_the_running_engine() {
        let state = AppState::for_test();
        let eng = running(&state);
        let ids = |s: &AppState| -> Vec<String> {
            s.signal_chain
                .lock()
                .unwrap()
                .slots
                .iter()
                .map(|s| s.id.clone())
                .collect()
        };
        let before = ids(&state);
        let delay = before.iter().position(|s| s == "delay").unwrap();
        let boost = before.iter().position(|s| s == "boost").unwrap();
        move_slot_impl(&state, delay, boost).unwrap();
        let after = ids(&state);
        let engine_order = eng.lock().unwrap().plugin_names();
        let engine_core: Vec<&String> = engine_order.iter().filter(|n| after.contains(n)).collect();
        let chain_core: Vec<&String> = after.iter().filter(|n| engine_order.contains(n)).collect();
        assert_eq!(engine_core, chain_core);
        assert!(
            engine_order.iter().position(|n| n == "delay")
                < engine_order.iter().position(|n| n == "boost")
        );
        assert_eq!(state.undo_chain.lock().unwrap().len(), 1);
    }

    #[test]
    fn move_slot_rejects_fixed_slots() {
        let state = AppState::for_test();
        let last = state.signal_chain.lock().unwrap().slots.len() - 1;
        assert!(move_slot_impl(&state, 0, 2).is_err());
        assert!(move_slot_impl(&state, 2, last).is_err());
        assert!(move_slot_impl(&state, 2, 99).is_err());
        assert!(state.undo_chain.lock().unwrap().is_empty());
    }

    #[test]
    fn default_chain_matches_engine_default_chain() {
        let chain = SignalChain::default();
        let ids: Vec<String> = chain.slots.iter().map(|s| s.id.clone()).collect();
        let eng = KicksEngine::new();
        assert_eq!(eng.plugin_names(), ids);
        for slot in &chain.slots {
            let on = eng.get_parameter(&format!("{}/enabled", slot.id));
            // Delay is enabled in the engine but bypassed by the saved chain;
            // sync_engine_to_chain resolves that at start.
            if slot.id != "delay" {
                assert_eq!(
                    on,
                    Some(if slot.enabled { 1.0 } else { 0.0 }),
                    "{}",
                    slot.id
                );
            }
            for k in slot.parameters.keys() {
                assert!(
                    eng.get_parameter(&format!("{}/{}", slot.id, k)).is_some(),
                    "engine has no {}/{}",
                    slot.id,
                    k
                );
            }
        }
    }

    #[test]
    fn slot_messages_cover_bypass_mix_and_params() {
        let chain = SignalChain::default();
        let amp = chain.slots.iter().find(|s| s.id == "amp").unwrap();
        let msgs = slot_messages(amp);
        assert!(msgs.iter().any(|(k, _)| k == "amp/enabled"));
        assert!(msgs.iter().any(|(k, _)| k == "amp/wet_dry"));
        assert_eq!(msgs.len(), amp.parameters.len() + 2);
    }
}
