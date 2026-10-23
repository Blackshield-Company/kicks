use crate::sync::LockOrRecover;
use kicks_core::persistence;
use kicks_core::scene::SceneCollection;
use kicks_core::signal_chain::SignalChain;
use serde::{Deserialize, Serialize};
use tauri::State;

use super::signal_chain::{push_undo_state, sync_engine_to_chain};
use crate::AppState;

/// A scene descriptor returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneInfo {
    pub index: usize,
    pub name: String,
    pub slot_count: usize,
    pub is_active: bool,
}

/// Full scene data including signal chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneDetail {
    pub index: usize,
    pub name: String,
    pub signal_chain: SignalChain,
}

/// Persist scenes to disk (best-effort).
fn persist_scenes(scenes: &SceneCollection) {
    if let Err(e) = persistence::save_scenes(scenes) {
        tracing::warn!("Failed to persist scenes: {}", e);
    }
}

/// List all scenes.
#[tauri::command]
pub fn list_scenes(state: State<'_, AppState>) -> Vec<SceneInfo> {
    let scenes = state.scenes.lock_or_recover();
    let active = scenes.active_index();
    scenes
        .scenes
        .iter()
        .enumerate()
        .map(|(i, s)| SceneInfo {
            index: i,
            name: s.name.clone(),
            slot_count: s.signal_chain.slots.len(),
            is_active: active == Some(i),
        })
        .collect()
}

/// Get detailed info for a single scene.
#[tauri::command]
pub fn get_scene(state: State<'_, AppState>, index: usize) -> Result<SceneDetail, String> {
    let scenes = state.scenes.lock().map_err(|e| e.to_string())?;
    let scene = scenes
        .get_scene(index)
        .ok_or_else(|| format!("Scene at index {} not found", index))?;
    Ok(SceneDetail {
        index,
        name: scene.name.clone(),
        signal_chain: scene.signal_chain.clone(),
    })
}

/// Save the current signal chain as a new scene.
#[tauri::command]
pub fn save_scene(state: State<'_, AppState>, name: String) -> Result<SceneInfo, String> {
    let signal_chain = {
        let chain = state.signal_chain.lock().map_err(|e| e.to_string())?;
        chain.clone()
    };

    let mut scenes = state.scenes.lock().map_err(|e| e.to_string())?;
    let index = scenes.len();
    scenes.add_scene(name.clone(), signal_chain);
    persist_scenes(&scenes);

    tracing::info!("Scene '{}' saved at index {}", name, index);
    Ok(SceneInfo {
        index,
        name,
        slot_count: 0,
        is_active: false,
    })
}

/// Overwrite an existing scene with the current signal chain.
#[tauri::command]
pub fn update_scene(state: State<'_, AppState>, index: usize) -> Result<(), String> {
    let signal_chain = {
        let chain = state.signal_chain.lock().map_err(|e| e.to_string())?;
        chain.clone()
    };

    let mut scenes = state.scenes.lock().map_err(|e| e.to_string())?;
    if index >= scenes.len() {
        return Err(format!("Scene index {} out of range", index));
    }
    scenes.scenes[index].signal_chain = signal_chain;
    persist_scenes(&scenes);

    tracing::info!("Scene at index {} updated", index);
    Ok(())
}

/// Load a scene: replace the current signal chain and apply to engine.
#[tauri::command]
pub fn load_scene(state: State<'_, AppState>, index: usize) -> Result<(), String> {
    let scene = {
        let mut scenes = state.scenes.lock().map_err(|e| e.to_string())?;
        let scene = scenes
            .get_scene(index)
            .ok_or_else(|| format!("Scene at index {} not found", index))?
            .clone();
        scenes.set_active(index);
        persist_scenes(&scenes);
        scene
    };

    push_undo_state(&state);
    {
        let mut chain = state.signal_chain.lock().map_err(|e| e.to_string())?;
        *chain = scene.signal_chain.clone();
        persistence::save_signal_chain(&chain)
            .map_err(|e| format!("Failed to persist signal chain: {}", e))?;
    }

    sync_engine_to_chain(&state, &scene.signal_chain);
    tracing::info!("Applied scene '{}' to engine", scene.name);

    tracing::info!("Scene '{}' (index {}) loaded", scene.name, index);
    Ok(())
}

/// Delete a scene by index.
#[tauri::command]
pub fn delete_scene(state: State<'_, AppState>, index: usize) -> Result<(), String> {
    let mut scenes = state.scenes.lock().map_err(|e| e.to_string())?;
    let removed = scenes.remove_scene(index);
    match removed {
        Some(_) => {
            persist_scenes(&scenes);
            tracing::info!("Deleted scene at index {}", index);
            Ok(())
        }
        None => Err(format!("Scene at index {} not found", index)),
    }
}

/// Rename a scene.
#[tauri::command]
pub fn rename_scene(
    state: State<'_, AppState>, index: usize, new_name: String,
) -> Result<(), String> {
    let mut scenes = state.scenes.lock().map_err(|e| e.to_string())?;
    scenes
        .rename_scene(index, new_name)
        .ok_or_else(|| format!("Scene at index {} not found", index))?;
    persist_scenes(&scenes);
    Ok(())
}

/// Reorder a scene (move from one index to another).
#[tauri::command]
pub fn reorder_scene(
    state: State<'_, AppState>, from_index: usize, to_index: usize,
) -> Result<(), String> {
    let mut scenes = state.scenes.lock().map_err(|e| e.to_string())?;
    scenes.reorder_scene(from_index, to_index).ok_or_else(|| {
        format!(
            "Failed to reorder scene from {} to {}",
            from_index, to_index
        )
    })?;
    persist_scenes(&scenes);
    Ok(())
}

/// Move to the next scene in the setlist.
#[tauri::command]
pub fn next_scene(state: State<'_, AppState>) -> Result<Option<SceneInfo>, String> {
    let scene_info = {
        let mut scenes = state.scenes.lock().map_err(|e| e.to_string())?;
        let idx = scenes.next_scene();
        persist_scenes(&scenes);
        idx.map(|i| {
            let s = &scenes.scenes[i];
            SceneInfo {
                index: i,
                name: s.name.clone(),
                slot_count: s.signal_chain.slots.len(),
                is_active: true,
            }
        })
    };

    // Load the next scene's chain
    if let Some(info) = &scene_info {
        let scene = {
            let scenes = state.scenes.lock().map_err(|e| e.to_string())?;
            scenes.get_scene(info.index).cloned()
        };
        if let Some(scene) = scene {
            let mut chain = state.signal_chain.lock().map_err(|e| e.to_string())?;
            *chain = scene.signal_chain.clone();
            persistence::save_signal_chain(&chain)
                .map_err(|e| format!("Failed to persist signal chain: {}", e))?;
            drop(chain);
            sync_engine_to_chain(&state, &scene.signal_chain);
        }
    }

    Ok(scene_info)
}

/// Move to the previous scene in the setlist.
#[tauri::command]
pub fn prev_scene(state: State<'_, AppState>) -> Result<Option<SceneInfo>, String> {
    let scene_info = {
        let mut scenes = state.scenes.lock().map_err(|e| e.to_string())?;
        let idx = scenes.prev_scene();
        persist_scenes(&scenes);
        idx.map(|i| {
            let s = &scenes.scenes[i];
            SceneInfo {
                index: i,
                name: s.name.clone(),
                slot_count: s.signal_chain.slots.len(),
                is_active: true,
            }
        })
    };

    if let Some(info) = &scene_info {
        let scene = {
            let scenes = state.scenes.lock().map_err(|e| e.to_string())?;
            scenes.get_scene(info.index).cloned()
        };
        if let Some(scene) = scene {
            let mut chain = state.signal_chain.lock().map_err(|e| e.to_string())?;
            *chain = scene.signal_chain.clone();
            persistence::save_signal_chain(&chain)
                .map_err(|e| format!("Failed to persist signal chain: {}", e))?;
            drop(chain);
            sync_engine_to_chain(&state, &scene.signal_chain);
        }
    }

    Ok(scene_info)
}
