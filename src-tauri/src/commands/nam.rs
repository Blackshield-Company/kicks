use crate::sync::LockOrRecover;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::AppState;

/// Info about a discovered .nam file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamFileInfo {
    pub path: String,
    pub name: String,
}

/// Result of loading a NAM model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamModelLoadResult {
    pub path: String,
    pub file_name: String,
    pub architecture: String,
    pub sample_rate: u32,
    pub num_parameters: usize,
}

/// List all .nam files in the configured NAM directories.
#[tauri::command]
pub fn list_nam_files(state: State<'_, AppState>) -> Vec<NamFileInfo> {
    let config = state.config.lock_or_recover();
    let mut results = Vec::new();

    for dir in &config.nam_directories {
        let Ok(path) = crate::path_guard::check_dir(dir, &state.path_grants) else {
            continue;
        };
        if !path.exists() {
            continue;
        }
        let entries = match std::fs::read_dir(&path) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let Ok(fpath) = crate::path_guard::check_file(
                &entry.path().to_string_lossy(),
                &state.path_grants,
                &["nam"],
                crate::path_guard::MAX_NAM_BYTES,
            ) else {
                continue;
            };
            let ext = fpath
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_lowercase())
                .unwrap_or_default();

            if ext != "nam" {
                continue;
            }

            let name = fpath
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();

            results.push(NamFileInfo {
                path: fpath.to_string_lossy().to_string(),
                name,
            });
        }
    }

    results
}

/// Scan a specific directory for .nam files.
#[tauri::command]
pub fn scan_nam_directory(
    state: State<'_, AppState>, dir_path: String,
) -> Result<Vec<NamFileInfo>, String> {
    let path = crate::path_guard::check_dir(&dir_path, &state.path_grants)?;
    let mut results = Vec::new();

    let entries =
        std::fs::read_dir(&path).map_err(|e| format!("Failed to read directory: {}", e))?;

    for entry in entries.flatten() {
        let Ok(fpath) = crate::path_guard::check_file(
            &entry.path().to_string_lossy(),
            &state.path_grants,
            &["nam"],
            crate::path_guard::MAX_NAM_BYTES,
        ) else {
            continue;
        };
        let ext = fpath
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();

        if ext != "nam" {
            continue;
        }

        let name = fpath
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        results.push(NamFileInfo {
            path: fpath.to_string_lossy().to_string(),
            name,
        });
    }

    Ok(results)
}

/// Load a NAM model file into the Nam plugin.
#[tauri::command]
pub fn load_nam_model(
    state: State<'_, AppState>, path: String,
) -> Result<NamModelLoadResult, String> {
    let resolved = crate::path_guard::check_file(
        &path,
        &state.path_grants,
        &["nam"],
        crate::path_guard::MAX_NAM_BYTES,
    )?;
    let path = resolved.to_string_lossy().into_owned();
    let fpath = resolved.as_path();

    let file_name = fpath
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    let (neural_model, num_parameters) = kicks_dsp::NeuralModel::from_file(&path)
        .map_err(|e| format!("Failed to load NAM model: {}", e))?;

    let architecture = neural_model.architecture().to_string();
    let sample_rate = neural_model.sample_rate();

    tracing::info!(
        "Loading NAM model: {} (arch: {}, {} Hz)",
        file_name,
        architecture,
        sample_rate
    );

    // Load into Nam plugin via engine
    {
        let engine_guard = state.engine.lock().map_err(|e| e.to_string())?;
        if let Some(ref eng_arc) = *engine_guard {
            let mut engine = eng_arc.lock().map_err(|e| e.to_string())?;
            engine.load_nam_to_plugin(path.clone(), neural_model);
        } else {
            return Err("Engine not running".to_string());
        }
    }

    Ok(NamModelLoadResult {
        path,
        file_name,
        architecture,
        sample_rate,
        num_parameters,
    })
}

/// Get info about the currently loaded NAM model.
#[tauri::command]
pub fn get_nam_info(state: State<'_, AppState>) -> Result<Option<NamModelLoadResult>, String> {
    let engine_guard = state.engine.lock().map_err(|e| e.to_string())?;
    if let Some(ref eng_arc) = *engine_guard {
        let engine = eng_arc.lock().map_err(|e| e.to_string())?;
        let info = engine.nam_model_info();
        Ok(info.map(|i| NamModelLoadResult {
            path: i.path,
            file_name: i.file_name,
            architecture: i.architecture,
            sample_rate: i.sample_rate,
            num_parameters: i.num_parameters,
        }))
    } else {
        Ok(None)
    }
}

/// Clear the loaded NAM model.
#[tauri::command]
pub fn clear_nam_model(state: State<'_, AppState>) -> Result<(), String> {
    {
        let engine_guard = state.engine.lock().map_err(|e| e.to_string())?;
        if let Some(ref eng_arc) = *engine_guard {
            let mut engine = eng_arc.lock().map_err(|e| e.to_string())?;
            engine.clear_nam_model();
        }
    }
    Ok(())
}
