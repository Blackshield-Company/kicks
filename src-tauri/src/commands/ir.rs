use crate::sync::LockOrRecover;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{State, Wry};
use tauri_plugin_dialog::DialogExt;

use crate::AppState;

/// Metadata about a discovered IR file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrFileInfo {
    pub path: String,
    pub name: String,
    pub sample_rate: u32,
    pub sample_count: u32,
    pub duration_ms: u32,
    pub channels: u16,
}

/// List all IR files in the configured IR directories.
#[tauri::command]
pub fn list_ir_files(state: State<'_, AppState>) -> Vec<IrFileInfo> {
    let config = state.config.lock_or_recover();
    let mut results = Vec::new();

    for dir in &config.ir_directories {
        let path = PathBuf::from(dir);
        if !path.exists() {
            continue;
        }
        let entries = match std::fs::read_dir(&path) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let fpath = entry.path();
            let ext = fpath
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_lowercase())
                .unwrap_or_default();

            if ext != "wav" && ext != "irs" && ext != "nam" {
                continue;
            }

            let name = fpath
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();

            // Try to parse WAV files for metadata
            let info = match parse_wav_metadata(&fpath) {
                Some(meta) => IrFileInfo {
                    path: fpath.to_string_lossy().to_string(),
                    name,
                    sample_rate: meta.0,
                    sample_count: meta.1,
                    duration_ms: if meta.0 > 0 {
                        (meta.1 as u64 * 1000 / meta.0 as u64) as u32
                    } else {
                        0
                    },
                    channels: meta.2,
                },
                None => IrFileInfo {
                    path: fpath.to_string_lossy().to_string(),
                    name,
                    sample_rate: 0,
                    sample_count: 0,
                    duration_ms: 0,
                    channels: 0,
                },
            };

            results.push(info);
        }
    }

    results
}

/// Open a file dialog to pick an IR file, returning its metadata.
#[tauri::command]
pub async fn pick_ir_file(app: tauri::AppHandle<Wry>) -> Result<Option<IrFileInfo>, String> {
    let file = app
        .dialog()
        .file()
        .add_filter("Impulse Responses", &["wav", "irs", "nam"])
        .add_filter("WAV Audio", &["wav"])
        .set_title("Select Impulse Response")
        .blocking_pick_file();

    match file {
        Some(path) => {
            let path_str = path.to_string();
            let path = std::path::PathBuf::from(&path_str);
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();

            let info = match parse_wav_metadata(&path) {
                Some((sample_rate, sample_count, channels)) => IrFileInfo {
                    path: path.to_string_lossy().to_string(),
                    name,
                    sample_rate,
                    sample_count,
                    duration_ms: if sample_rate > 0 {
                        (sample_count as u64 * 1000 / sample_rate as u64) as u32
                    } else {
                        0
                    },
                    channels,
                },
                None => IrFileInfo {
                    path: path.to_string_lossy().to_string(),
                    name,
                    sample_rate: 0,
                    sample_count: 0,
                    duration_ms: 0,
                    channels: 0,
                },
            };

            Ok(Some(info))
        }
        None => Ok(None),
    }
}

/// Parse a WAV file and return (sample_rate, sample_count, channels).
fn parse_wav_metadata(path: &std::path::Path) -> Option<(u32, u32, u16)> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    if ext != "wav" {
        return None;
    }

    let reader = hound::WavReader::open(path).ok()?;
    let spec = reader.spec();
    let sample_count = reader.duration();
    let channels = spec.channels;
    let sample_rate = spec.sample_rate;

    Some((sample_rate, sample_count, channels))
}

/// Load an impulse response file into the Cab plugin.
#[tauri::command]
pub fn load_ir_to_cab(state: State<'_, AppState>, path: String) -> Result<IrLoadResult, String> {
    // Validate file exists
    let fpath = std::path::Path::new(&path);
    if !fpath.exists() {
        return Err(format!("File not found: {}", path));
    }

    // Read WAV file and extract float samples
    let mut reader =
        hound::WavReader::open(fpath).map_err(|e| format!("Failed to open WAV: {}", e))?;
    let spec = reader.spec();

    let sample_rate = spec.sample_rate as f32;
    let channels = spec.channels as usize;
    let total_samples = reader.duration() as usize;

    // Read samples based on bit depth
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => {
            // 32-bit float WAV
            reader.samples::<f32>().filter_map(|s| s.ok()).collect()
        }
        hound::SampleFormat::Int => match spec.bits_per_sample {
            16 => reader
                .samples::<i16>()
                .filter_map(|s| s.ok())
                .map(|s| s as f32 / 32768.0)
                .collect(),
            24 => reader
                .samples::<i32>()
                .filter_map(|s| s.ok())
                .map(|s| s as f32 / 8388608.0)
                .collect(),
            32 => reader
                .samples::<i32>()
                .filter_map(|s| s.ok())
                .map(|s| s as f32 / 2147483648.0)
                .collect(),
            _ => return Err(format!("Unsupported bit depth: {}", spec.bits_per_sample)),
        },
    };

    // Down-mix to mono if stereo by averaging channels
    let mono_samples: Vec<f32> = if channels > 1 {
        let frame_count = samples.len() / channels;
        (0..frame_count)
            .map(|f| {
                let sum: f32 = (0..channels).map(|ch| samples[f * channels + ch]).sum();
                sum / channels as f32
            })
            .collect()
    } else {
        samples
    };

    let file_name = fpath
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    tracing::info!(
        "Loading IR: {} ({} samples, {} ch, {} Hz)",
        file_name,
        mono_samples.len(),
        channels,
        spec.sample_rate
    );

    // Load into Cab plugin via engine
    {
        let engine_guard = state.engine.lock().map_err(|e| e.to_string())?;
        if let Some(ref eng_arc) = *engine_guard {
            let mut engine = eng_arc.lock().map_err(|e| e.to_string())?;
            engine.load_ir_to_cab(path.clone(), mono_samples, sample_rate);
        } else {
            return Err("Engine not running".to_string());
        }
    }

    // Save active IR path to config
    {
        let mut config = state.config.lock().map_err(|e| e.to_string())?;
        config.active_ir_path = path.clone();
        // Persist config
        let _ = kicks_core::persistence::save_config(&config);
    }

    let ir_len_ms = if sample_rate > 0.0 {
        (total_samples as f32) / sample_rate * 1000.0
    } else {
        0.0
    };

    Ok(IrLoadResult {
        path,
        file_name,
        sample_rate: spec.sample_rate,
        length_samples: total_samples,
        length_ms: ir_len_ms as u32,
    })
}

/// Result of loading an IR file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrLoadResult {
    pub path: String,
    pub file_name: String,
    pub sample_rate: u32,
    pub length_samples: usize,
    pub length_ms: u32,
}

/// Get info about the currently loaded IR in the Cab plugin.
#[tauri::command]
pub fn get_cab_ir_info(state: State<'_, AppState>) -> Result<Option<IrLoadResult>, String> {
    let engine_guard = state.engine.lock().map_err(|e| e.to_string())?;
    if let Some(ref eng_arc) = *engine_guard {
        let engine = eng_arc.lock().map_err(|e| e.to_string())?;
        let info = engine.cab_ir_info();
        Ok(info.map(|i| IrLoadResult {
            path: i.path,
            file_name: i.file_name,
            sample_rate: i.sample_rate as u32,
            length_samples: i.length_samples,
            length_ms: i.length_ms as u32,
        }))
    } else {
        Ok(None)
    }
}

/// Clear the loaded IR from the Cab plugin.
#[tauri::command]
pub fn clear_cab_ir(state: State<'_, AppState>) -> Result<(), String> {
    {
        let engine_guard = state.engine.lock().map_err(|e| e.to_string())?;
        if let Some(ref eng_arc) = *engine_guard {
            let mut engine = eng_arc.lock().map_err(|e| e.to_string())?;
            engine.clear_cab_ir();
        }
    }
    {
        let mut config = state.config.lock().map_err(|e| e.to_string())?;
        config.active_ir_path = String::new();
        let _ = kicks_core::persistence::save_config(&config);
    }
    Ok(())
}

/// Scan for IR files in a specific directory.
#[tauri::command]
pub fn scan_ir_directory(
    state: State<'_, AppState>, dir_path: String,
) -> Result<Vec<IrFileInfo>, String> {
    let _config = state.config.lock().map_err(|e| e.to_string())?;
    let mut results = Vec::new();
    let path = PathBuf::from(&dir_path);

    if !path.exists() {
        return Err(format!("Directory '{}' does not exist", dir_path));
    }

    let entries =
        std::fs::read_dir(&path).map_err(|e| format!("Failed to read directory: {}", e))?;

    for entry in entries.flatten() {
        let fpath = entry.path();
        let ext = fpath
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();

        if ext != "wav" && ext != "irs" && ext != "nam" {
            continue;
        }

        let name = fpath
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        let info = match parse_wav_metadata(&fpath) {
            Some((sample_rate, sample_count, channels)) => IrFileInfo {
                path: fpath.to_string_lossy().to_string(),
                name,
                sample_rate,
                sample_count,
                duration_ms: if sample_rate > 0 {
                    (sample_count as u64 * 1000 / sample_rate as u64) as u32
                } else {
                    0
                },
                channels,
            },
            None => IrFileInfo {
                path: fpath.to_string_lossy().to_string(),
                name,
                sample_rate: 0,
                sample_count: 0,
                duration_ms: 0,
                channels: 0,
            },
        };

        results.push(info);
    }

    Ok(results)
}
