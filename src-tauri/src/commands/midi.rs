use crate::sync::LockOrRecover;
use kicks_core::persistence;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::AppState;

/// MIDI device info returned to the frontend.
#[derive(Debug, Serialize)]
pub struct MidiDeviceInfo {
    pub name: String,
    pub connected: bool,
}

/// A processed MIDI event report for the frontend.
#[derive(Debug, Serialize)]
pub struct MidiEventReport {
    pub cc: u8,
    pub channel: u8,
    pub raw_value: u8,
    pub normalized: f32,
    pub mapped_parameter: Option<String>,
    pub mapped_value: Option<f32>,
}

/// MIDI config payload for frontend CRUD.
#[derive(Debug, Serialize, Deserialize)]
pub struct MidiConfigPayload {
    pub active_device: Option<String>,
    pub channel: u8,
    pub mappings: Vec<MidiMappingPayload>,
    pub learn_mode: bool,
    pub last_cc: Option<u8>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MidiMappingPayload {
    pub cc: u8,
    pub channel: u8,
    pub parameter_id: String,
    pub label: String,
    pub min: f32,
    pub max: f32,
}

impl From<&kicks_core::midi::MidiConfig> for MidiConfigPayload {
    fn from(c: &kicks_core::midi::MidiConfig) -> Self {
        Self {
            active_device: c.active_device.clone(),
            channel: c.channel,
            mappings: c
                .mappings
                .iter()
                .map(|m| MidiMappingPayload {
                    cc: m.cc,
                    channel: m.channel,
                    parameter_id: m.parameter_id.clone(),
                    label: m.label.clone(),
                    min: m.min,
                    max: m.max,
                })
                .collect(),
            learn_mode: c.learn_mode,
            last_cc: c.last_cc,
        }
    }
}

/// List available MIDI input devices.
#[tauri::command]
pub fn list_midi_devices(state: State<'_, AppState>) -> Result<Vec<MidiDeviceInfo>, String> {
    let ports = crate::midi::MidiManager::list_ports()?;
    let manager = state.midi_manager.lock().map_err(|e| e.to_string())?;

    let connected = manager.is_connected();
    Ok(ports
        .into_iter()
        .map(|name| MidiDeviceInfo { connected, name })
        .collect())
}

/// Get the current MIDI configuration.
#[tauri::command]
pub fn get_midi_config(state: State<'_, AppState>) -> MidiConfigPayload {
    let config = state.midi_config.lock_or_recover();
    MidiConfigPayload::from(&*config)
}

/// Save the MIDI configuration to disk.
#[tauri::command]
pub fn save_midi_config(
    state: State<'_, AppState>, config: MidiConfigPayload,
) -> Result<(), String> {
    let mut midi_config = state.midi_config.lock().map_err(|e| e.to_string())?;

    // Preserve runtime fields from the incoming payload
    midi_config.active_device = config.active_device;
    midi_config.channel = config.channel;
    midi_config.mappings = config
        .mappings
        .into_iter()
        .map(|m| kicks_core::midi::MidiMapping {
            cc: m.cc,
            channel: m.channel,
            parameter_id: m.parameter_id,
            label: m.label,
            min: m.min,
            max: m.max,
        })
        .collect();
    midi_config.learn_mode = config.learn_mode;

    // Persist to disk
    persistence::save_midi_config(&midi_config).map_err(|e| e.to_string())?;
    tracing::info!("MIDI config saved");
    Ok(())
}

/// Connect to a MIDI input device by name.
#[tauri::command]
pub fn connect_midi_device(state: State<'_, AppState>, device_name: String) -> Result<(), String> {
    let mut manager = state.midi_manager.lock().map_err(|e| e.to_string())?;
    manager.connect(&device_name)?;

    // Update config with active device
    if let Ok(mut config) = state.midi_config.lock() {
        config.active_device = Some(device_name);
    }

    Ok(())
}

/// Disconnect from the active MIDI input device.
#[tauri::command]
pub fn disconnect_midi_device(state: State<'_, AppState>) -> Result<(), String> {
    let mut manager = state.midi_manager.lock().map_err(|e| e.to_string())?;
    manager.disconnect();

    // Update config
    if let Ok(mut config) = state.midi_config.lock() {
        config.active_device = None;
    }

    Ok(())
}

/// Poll for pending MIDI CC events, route matched mappings to the engine,
/// and return reports for the frontend (learn mode UI).
#[tauri::command]
pub fn poll_midi_events(state: State<'_, AppState>) -> Result<Vec<MidiEventReport>, String> {
    let manager = state.midi_manager.lock().map_err(|e| e.to_string())?;
    let config = state.midi_config.lock().map_err(|e| e.to_string())?;

    let events = manager.drain_events();
    let mut reports = Vec::new();

    for (channel, cc, raw_value) in events {
        let normalized = raw_value as f32 / 127.0;

        // If learn mode is active, record the last CC
        if config.learn_mode {
            if let Ok(mut c) = state.midi_config.lock() {
                c.last_cc = Some(cc);
            }
            reports.push(MidiEventReport {
                cc,
                channel,
                raw_value,
                normalized,
                mapped_parameter: None,
                mapped_value: None,
            });
            continue;
        }

        // Look for a matching mapping (channel match or global)
        let matched = config
            .mappings
            .iter()
            .find(|m| m.cc == cc && (m.channel == channel || config.channel == 0));

        if let Some(mapping) = matched {
            let param_value = mapping.min + normalized * (mapping.max - mapping.min);

            // Push to lock-free parameter channel (no engine mutex needed)
            if let Ok(ref tx_guard) = state.param_tx.lock() {
                if let Some(ref tx) = **tx_guard {
                    let _ = tx.send(mapping.parameter_id.clone(), param_value);
                }
            }

            reports.push(MidiEventReport {
                cc,
                channel,
                raw_value,
                normalized,
                mapped_parameter: Some(mapping.parameter_id.clone()),
                mapped_value: Some(param_value),
            });
        } else {
            // Unmapped CC — still report it for learn mode display
            reports.push(MidiEventReport {
                cc,
                channel,
                raw_value,
                normalized,
                mapped_parameter: None,
                mapped_value: None,
            });
        }
    }

    Ok(reports)
}

/// Enable or disable MIDI learn mode.
#[tauri::command]
pub fn set_midi_learn(state: State<'_, AppState>, active: bool) -> Result<(), String> {
    let mut config = state.midi_config.lock().map_err(|e| e.to_string())?;
    config.learn_mode = active;
    if !active {
        config.last_cc = None;
    }
    Ok(())
}
