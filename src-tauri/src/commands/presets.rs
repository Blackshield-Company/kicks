use crate::sync::LockOrRecover;
use kicks_core::preset::{Bank, Preset};
use serde::Serialize;
use tauri::State;

use super::signal_chain::push_undo_state;
use crate::AppState;

/// A lightweight preset descriptor for list views.
#[derive(Debug, Serialize)]
pub struct PresetDescriptor {
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub created: String,
    pub modified: String,
}

/// A bank descriptor for the frontend.
#[derive(Debug, Serialize)]
pub struct BankDescriptor {
    pub name: String,
    pub presets: Vec<PresetDescriptor>,
}

/// List all presets organized by bank.
#[tauri::command]
pub fn list_presets(state: State<'_, AppState>) -> Vec<BankDescriptor> {
    let collection = state.presets.lock_or_recover();
    collection
        .banks
        .iter()
        .map(|bank| BankDescriptor {
            name: bank.name.clone(),
            presets: bank
                .presets
                .iter()
                .map(|p| PresetDescriptor {
                    name: p.name.clone(),
                    description: p.description.clone(),
                    tags: p.tags.clone(),
                    created: p.created.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
                    modified: p.modified.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
                })
                .collect(),
        })
        .collect()
}

/// Save the current signal chain as a named preset.
#[tauri::command]
pub fn save_preset(
    state: State<'_, AppState>, bank_name: String, preset_name: String,
    description: Option<String>, tags: Option<Vec<String>>,
) -> Result<(), String> {
    let chain = state.signal_chain.lock().map_err(|e| e.to_string())?;
    let mut collection = state.presets.lock().map_err(|e| e.to_string())?;

    let now = chrono::Utc::now();
    let preset = Preset {
        name: preset_name.clone(),
        description: description.unwrap_or_default(),
        tags: tags.unwrap_or_default(),
        signal_chain: chain.clone(),
        created: now,
        modified: now,
    };

    // Find or create the bank
    if let Some(bank) = collection.banks.iter_mut().find(|b| b.name == bank_name) {
        bank.presets.push(preset);
    } else {
        collection.banks.push(Bank {
            name: bank_name,
            presets: vec![preset],
        });
    }

    if let Err(e) = kicks_core::persistence::save_presets(&collection) {
        tracing::warn!("Failed to persist presets: {}", e);
    }

    tracing::info!("Preset '{}' saved", preset_name);
    Ok(())
}

/// Load a preset: replace the current signal chain and apply it to the engine.
#[tauri::command]
pub fn load_preset(
    state: State<'_, AppState>, bank_name: String, preset_name: String,
) -> Result<(), String> {
    let preset_chain = {
        let collection = state.presets.lock().map_err(|e| e.to_string())?;
        let bank = collection
            .banks
            .iter()
            .find(|b| b.name == bank_name)
            .ok_or_else(|| format!("Bank '{}' not found", bank_name))?;
        bank.presets
            .iter()
            .find(|p| p.name == preset_name)
            .ok_or_else(|| format!("Preset '{}' not found", preset_name))?
            .signal_chain
            .clone()
    };

    push_undo_state(&state);
    {
        let mut chain = state.signal_chain.lock().map_err(|e| e.to_string())?;
        *chain = preset_chain.clone();
        kicks_core::persistence::save_signal_chain(&chain)
            .map_err(|e| format!("Failed to persist signal chain: {}", e))?;
    }

    // Push preset parameters through the lock-free channel
    // (no engine mutex needed — the audio callback drains the queue)
    if let Ok(tx_guard) = state.param_tx.lock() {
        if let Some(ref tx) = *tx_guard {
            for slot in &preset_chain.slots {
                let _ = tx.send(
                    format!("{}/enabled", slot.id),
                    if slot.enabled { 1.0 } else { 0.0 },
                );
                for (param_id, value) in &slot.parameters {
                    let _ = tx.send(format!("{}/{}", slot.id, param_id), *value);
                }
            }
        }
    }

    tracing::info!("Preset '{}' loaded from bank '{}'", preset_name, bank_name);
    Ok(())
}

/// Delete a preset from a bank.
#[tauri::command]
pub fn delete_preset(
    state: State<'_, AppState>, bank_name: String, preset_name: String,
) -> Result<(), String> {
    let mut collection = state.presets.lock().map_err(|e| e.to_string())?;

    if let Some(bank) = collection.banks.iter_mut().find(|b| b.name == bank_name) {
        let len_before = bank.presets.len();
        bank.presets.retain(|p| p.name != preset_name);
        if bank.presets.len() < len_before {
            tracing::info!("Preset '{}' deleted from bank '{}'", preset_name, bank_name);
            if let Err(e) = kicks_core::persistence::save_presets(&collection) {
                tracing::warn!("Failed to persist presets: {}", e);
            }
            return Ok(());
        }
    }

    Err(format!(
        "Preset '{}' not found in bank '{}'",
        preset_name, bank_name
    ))
}

/// Rename a preset.
#[tauri::command]
pub fn rename_preset(
    state: State<'_, AppState>, bank_name: String, old_name: String, new_name: String,
) -> Result<(), String> {
    let mut collection = state.presets.lock().map_err(|e| e.to_string())?;

    // Find bank index + preset index to avoid holding mutable refs during save
    let bank_idx = collection
        .banks
        .iter()
        .position(|b| b.name == bank_name)
        .ok_or_else(|| format!("Bank '{}' not found", bank_name))?;

    let preset_idx = collection.banks[bank_idx]
        .presets
        .iter()
        .position(|p| p.name == old_name)
        .ok_or_else(|| format!("Preset '{}' not found", old_name))?;

    collection.banks[bank_idx].presets[preset_idx].name = new_name.clone();
    tracing::info!("Preset renamed from '{}' to '{}'", old_name, new_name);

    if let Err(e) = kicks_core::persistence::save_presets(&collection) {
        tracing::warn!("Failed to persist presets: {}", e);
    }

    Ok(())
}
