use kicks_core::amp_preset::{built_in_amp_presets, AmpPreset};
use kicks_core::signal_chain::PluginType;
use serde::Serialize;
use tauri::State;

use crate::AppState;

/// A lightweight amp preset descriptor for the frontend.
#[derive(Debug, Clone, Serialize)]
pub struct AmpPresetInfo {
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub gain: f32,
    pub master: f32,
    pub bass: f32,
    pub mid: f32,
    pub treble: f32,
    pub drive: f32,
}

impl From<&AmpPreset> for AmpPresetInfo {
    fn from(p: &AmpPreset) -> Self {
        Self {
            name: p.name.clone(),
            description: p.description.clone(),
            tags: p.tags.clone(),
            gain: p.gain,
            master: p.master,
            bass: p.bass,
            mid: p.mid,
            treble: p.treble,
            drive: p.drive,
        }
    }
}

/// List all built-in amp presets.
#[tauri::command]
pub fn list_amp_presets() -> Vec<AmpPresetInfo> {
    built_in_amp_presets()
        .iter()
        .map(AmpPresetInfo::from)
        .collect()
}

/// Apply an amp preset to the Amp or BassAmp slot in the current signal chain.
///
/// Finds the first matching slot (Amp or BassAmp) and sets its parameters.
/// Bass presets also switch the slot type to BassAmp and push bass_mode to the engine.
#[tauri::command]
pub fn apply_amp_preset(state: State<'_, AppState>, preset_name: String) -> Result<(), String> {
    // Look up the preset
    let presets = built_in_amp_presets();
    let preset = presets
        .iter()
        .find(|p| p.name == preset_name)
        .ok_or_else(|| format!("Amp preset '{}' not found", preset_name))?;

    // Push undo state (reuse the signal chain undo stack)
    super::signal_chain::push_undo_state(&state);

    // Find and update the Amp or BassAmp slot
    let mut chain = state.signal_chain.lock().map_err(|e| e.to_string())?;
    let amp_slot = chain
        .slots
        .iter_mut()
        .find(|s| s.plugin_type == PluginType::Amp || s.plugin_type == PluginType::BassAmp)
        .ok_or_else(|| "No Amp or BassAmp slot found in signal chain".to_string())?;

    // Apply the preset parameters
    for (key, value) in preset.to_parameter_map() {
        amp_slot.parameters.insert(key, value);
    }

    // Switch slot type based on bass_mode
    let is_bass = preset.bass_mode > 0.5;
    if is_bass {
        amp_slot.plugin_type = PluginType::BassAmp;
        amp_slot.id = "bass-amp".to_string();
    } else {
        amp_slot.plugin_type = PluginType::Amp;
        amp_slot.id = "amp".to_string();
    }

    // Push bass_mode through the lock-free parameter channel
    if let Ok(tx_guard) = state.param_tx.lock() {
        if let Some(ref tx) = *tx_guard {
            let _ = tx.send("bass_mode", preset.bass_mode);
        }
    }

    tracing::info!("Amp preset '{}' applied (bass: {})", preset_name, is_bass);
    Ok(())
}
