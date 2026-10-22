use std::path::Path;
use std::sync::{Arc, Mutex};

use kicks_core::config::AudioBackend;
use kicks_core::signal_chain::PluginType;
use kicks_dsp::param::param_channel;
use kicks_dsp::{AudioConfig, AudioEngine, CpalAudioIO, JackAudioIO, KicksEngine};
use serde::Serialize;
use tauri::State;

use crate::AppState;

/// Status information about the audio engine.
#[derive(Debug, Serialize)]
pub struct EngineStatus {
    pub running: bool,
    pub sample_rate: f64,
    pub buffer_size: u32,
    pub backend: String,
}

/// Helper to read a WAV file and load it into the engine's Cab plugin.
fn load_ir_from_file(path: &str, eng: &mut KicksEngine) -> Result<super::ir::IrLoadResult, String> {
    let fpath = Path::new(path);
    let mut reader =
        hound::WavReader::open(fpath).map_err(|e| format!("Failed to open WAV: {}", e))?;
    let spec = reader.spec();
    let channels = spec.channels as usize;

    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().filter_map(|s| s.ok()).collect(),
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

    let mono_samples: Vec<f32> = if channels > 1 {
        let frame_count = samples.len() / channels;
        (0..frame_count)
            .map(|f| {
                (0..channels)
                    .map(|ch| samples[f * channels + ch])
                    .sum::<f32>()
                    / channels as f32
            })
            .collect()
    } else {
        samples
    };

    let file_name = fpath
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    eng.load_ir_to_cab(
        path.to_string(),
        mono_samples.clone(),
        spec.sample_rate as f32,
    );

    let total_samples = mono_samples.len();
    let ir_len_ms = if spec.sample_rate > 0 {
        (total_samples as f32) / spec.sample_rate as f32 * 1000.0
    } else {
        0.0
    };

    Ok(super::ir::IrLoadResult {
        path: path.to_string(),
        file_name,
        sample_rate: spec.sample_rate,
        length_samples: total_samples,
        length_ms: ir_len_ms as u32,
    })
}

/// Start the internal audio engine with the selected audio backend.
#[tauri::command]
pub fn start_engine(state: State<'_, AppState>) -> Result<(), String> {
    let mut engine_guard = state.engine.lock().map_err(|e| e.to_string())?;

    if engine_guard.is_some() {
        tracing::info!("Engine already running; skipping start");
        return Ok(());
    }

    let config = state.config.lock().map_err(|e| e.to_string())?;
    let sample_rate = config.sample_rate as f64;
    let buffer_size = config.buffer_size;
    let audio_device = config.audio_device.clone();
    let input_device = config.input_device.clone();
    let output_device = config.output_device.clone();
    let audio_backend = config.audio_backend.clone();
    let jack_client_name = config.jack_client_name.clone();
    let input_channel = config.input_channel;
    drop(config);

    let eng = Arc::new(Mutex::new(KicksEngine::new()));
    {
        let mut eng_inner = eng.lock().map_err(|e| e.to_string())?;
        eng_inner
            .init(sample_rate, buffer_size)
            .map_err(|e| e.to_string())?;
    }

    // Auto-load any previously saved IR
    let config_ir = state.config.lock().map_err(|e| e.to_string())?;
    if !config_ir.active_ir_path.is_empty() {
        let ir_path = config_ir.active_ir_path.clone();
        drop(config_ir);

        let fpath = Path::new(&ir_path);
        if fpath.exists() {
            let mut eng_inner = eng.lock().map_err(|e| e.to_string())?;
            if let Ok(ir_result) = load_ir_from_file(&ir_path, &mut eng_inner) {
                tracing::info!(
                    "Auto-loaded IR: {} ({} samples)",
                    ir_result.file_name,
                    ir_result.length_samples
                );
            } else {
                tracing::warn!("Failed to auto-load IR from saved path: {}", ir_path);
            }
        } else {
            tracing::warn!("Saved IR path no longer exists: {}", ir_path);
        }
    } else {
        drop(config_ir);
    }

    // If the signal chain has a BassAmp, set bass_mode on the engine
    let chain = state.signal_chain.lock().map_err(|e| e.to_string())?;
    {
        let mut eng_inner = eng.lock().map_err(|e| e.to_string())?;
        if let Some(slot) = chain
            .slots
            .iter()
            .find(|s| matches!(s.plugin_type, PluginType::BassAmp))
        {
            let bass_val = slot.parameters.get("bass_mode").copied().unwrap_or(1.0);
            eng_inner.set_parameter("bass_mode", bass_val);
        }
        // Apply all saved signal chain parameters to the engine
        for slot in &chain.slots {
            let plugin_name = slot.id.clone();
            eng_inner.set_plugin_enabled(&plugin_name, slot.enabled);
            for (param_id, value) in &slot.parameters {
                eng_inner.set_parameter_on_plugin(&plugin_name, param_id, *value);
            }
        }
    }
    drop(chain);

    let eng_for_io = eng.clone();
    *engine_guard = Some(eng);
    drop(engine_guard);

    match audio_backend {
        AudioBackend::Jack => {
            // ── JACK backend ──
            let mut jack_io = state.jack_audio_io.lock().map_err(|e| e.to_string())?;
            if jack_io.is_none() {
                *jack_io = Some(JackAudioIO::new(kicks_dsp::JackConfig {
                    client_name: if jack_client_name.is_empty() {
                        "Kicks".to_string()
                    } else {
                        jack_client_name
                    },
                }));
            }
            if let Some(ref mut io) = *jack_io {
                let cpu_load = Arc::clone(&state.cpu_load);
                {
                    let (param_tx, param_rx) = param_channel();
                    *state.param_tx.lock().map_err(|e| e.to_string())? = Some(param_tx);
                    io.start(eng_for_io, param_rx, Some(cpu_load))
                }
                .map_err(|e| format!("Failed to start JACK audio: {}", e))?;
            }
            tracing::info!("Audio engine started with JACK I/O");
        }
        AudioBackend::Cpal => {
            // ── CPAL backend ──
            // Create lock-free parameter channel
            let (param_tx, param_rx) = param_channel();
            *state.param_tx.lock().map_err(|e| e.to_string())? = Some(param_tx);

            let audio_config = AudioConfig {
                sample_rate,
                buffer_size,
                input_channel,
                output_device: if output_device.is_empty() {
                    if audio_device.is_empty() {
                        None
                    } else {
                        Some(audio_device.clone())
                    }
                } else {
                    Some(output_device.clone())
                },
                input_device: if input_device.is_empty() {
                    if audio_device.is_empty() {
                        None
                    } else {
                        Some(audio_device)
                    }
                } else {
                    Some(input_device)
                },
            };

            let mut audio_io = state.audio_io.lock().map_err(|e| e.to_string())?;
            if audio_io.is_none() {
                *audio_io = Some(CpalAudioIO::new());
            }
            if let Some(ref mut io) = *audio_io {
                let cpu_load = Arc::clone(&state.cpu_load);
                io.start(eng_for_io, audio_config, param_rx, Some(cpu_load))
                    .map_err(|e| format!("Failed to start CPAL audio: {}", e))?;
            }
            tracing::info!(
                "Audio engine started with CPAL I/O ({} Hz, buffer {})",
                sample_rate,
                buffer_size
            );
        }
    }

    Ok(())
}

/// Stop the audio engine and all I/O backends.
#[tauri::command]
pub fn stop_engine(state: State<'_, AppState>) -> Result<(), String> {
    // Clear the parameter channel first (no more pushes from main thread)
    *state.param_tx.lock().map_err(|e| e.to_string())? = None;

    // Stop CPAL audio I/O
    let mut audio_io = state.audio_io.lock().map_err(|e| e.to_string())?;
    if let Some(ref mut io) = *audio_io {
        io.stop();
    }
    *audio_io = None;
    drop(audio_io);

    // Stop JACK audio I/O
    let mut jack_io = state.jack_audio_io.lock().map_err(|e| e.to_string())?;
    if let Some(ref mut io) = *jack_io {
        io.stop();
    }
    *jack_io = None;
    drop(jack_io);

    // Clear engine
    let mut engine_guard = state.engine.lock().map_err(|e| e.to_string())?;
    *engine_guard = None;

    tracing::info!("Audio engine stopped");
    Ok(())
}

/// Get the current engine status.
#[tauri::command]
pub fn engine_status(state: State<'_, AppState>) -> EngineStatus {
    let engine_guard = state.engine.lock();
    match engine_guard {
        Ok(guard) if guard.is_some() => {
            // Try to get actual config values; fall back to defaults if locked
            let (sr, bs, backend) = if let Ok(cfg) = state.config.try_lock() {
                (
                    cfg.sample_rate as f64,
                    cfg.buffer_size,
                    format!("{:?}", cfg.audio_backend),
                )
            } else {
                (48000.0, 256, "Unknown".to_string())
            };
            EngineStatus {
                running: true,
                sample_rate: sr,
                buffer_size: bs,
                backend,
            }
        }
        _ => EngineStatus {
            running: false,
            sample_rate: 0.0,
            buffer_size: 0,
            backend: "None".to_string(),
        },
    }
}

/// Get the DSP CPU load as a percentage (0.0 – 100.0+).
///
/// The audio callback measures processing time vs real-time budget
/// and stores the value as (percentage * 1000) in an atomic counter.
#[tauri::command]
pub fn get_cpu_load(state: State<'_, AppState>) -> f64 {
    let raw = state.cpu_load.load(std::sync::atomic::Ordering::Relaxed);
    raw as f64 / 1000.0
}

/// Live audio I/O status for the UI: backend, latency, xruns, input health.
#[derive(Debug, Clone, serde::Serialize, Default)]
pub struct AudioStatus {
    pub running: bool,
    pub backend: String,
    pub latency_ms: f64,
    pub underruns: u64,
    pub overruns: u64,
    pub callback_frames: u64,
    pub input_ok: bool,
    pub input_error: Option<String>,
}

#[tauri::command]
pub fn get_audio_status(state: State<'_, AppState>) -> AudioStatus {
    if let Ok(io) = state.audio_io.lock() {
        if let Some(ref io) = *io {
            if io.is_active() {
                let s = io.stats();
                return AudioStatus {
                    running: true,
                    backend: "Cpal".into(),
                    latency_ms: io.latency_ms(),
                    underruns: s.underruns,
                    overruns: s.overruns,
                    callback_frames: s.last_callback_frames,
                    input_ok: s.input_ok,
                    input_error: s.input_error,
                };
            }
        }
    }
    if let Ok(io) = state.jack_audio_io.lock() {
        if let Some(ref io) = *io {
            if io.is_active() {
                return AudioStatus {
                    running: true,
                    backend: "Jack".into(),
                    latency_ms: io.latency_ms(),
                    input_ok: true,
                    ..Default::default()
                };
            }
        }
    }
    AudioStatus::default()
}

/// Set a named parameter on the active engine.
///
/// Pushes the change to the lock-free SPSC parameter channel. The audio
/// callback drains the channel before each processing cycle, so the
/// change is applied without ever holding the engine mutex from the
/// main thread.  This eliminates the primary source of `try_lock`
/// failures in the audio callback.
///
/// As a best-effort consistency measure, the engine's parameter HashMap
/// is also updated immediately via `try_lock` (non-blocking). If the
/// lock is contended, the audio callback applies the change on the next
/// cycle and `get_parameter` will return the correct value after that.
#[tauri::command]
pub fn set_parameter(state: State<'_, AppState>, id: String, value: f32) -> Result<(), String> {
    // 1. Push to lock-free SPSC queue — always fast, never blocks on engine
    let tx_guard = state.param_tx.lock().map_err(|e| e.to_string())?;
    if let Some(ref tx) = *tx_guard {
        tx.send(id.clone(), value)
            .map_err(|_| "Parameter queue full".to_string())?;
    }
    // JACK backend doesn't use the param channel — parameter changes
    // are applied directly via try_lock below.
    drop(tx_guard);

    // 2. Opportunistically sync the engine's parameter HashMap so
    //    `get_parameter` returns the latest value immediately.
    //    Uses `try_lock` — if the engine lock is contended, the value
    //    is still guaranteed to arrive via the SPSC queue drain (CPAL)
    //    or the next process cycle (JACK).
    if let Ok(engine_guard) = state.engine.try_lock() {
        if let Some(ref eng_arc) = *engine_guard {
            if let Ok(mut eng) = eng_arc.try_lock() {
                eng.set_parameter_value(&id, value);
            }
        }
    }

    Ok(())
}

/// Get the current per-plugin RMS audio levels.
///
/// Returns a vector of f32 values in 0..1 range, one per plugin slot.
/// Updated by the audio callback every process cycle (~5 ms).
/// The frontend polls this at ~20 Hz for real-time VU meters.
#[tauri::command]
pub fn get_audio_levels(state: State<'_, AppState>) -> Result<Vec<f32>, String> {
    let engine_guard = state.engine.lock().map_err(|e| e.to_string())?;
    if let Some(ref eng) = *engine_guard {
        let eng_inner = eng.lock().map_err(|e| e.to_string())?;
        Ok(eng_inner.audio_levels())
    } else {
        Err("Engine not running".to_string())
    }
}

/// Get a parameter value from the active engine.
#[tauri::command]
pub fn get_parameter(state: State<'_, AppState>, id: String) -> Result<Option<f32>, String> {
    let engine_guard = state.engine.lock().map_err(|e| e.to_string())?;
    if let Some(ref eng) = *engine_guard {
        let eng_inner = eng.lock().map_err(|e| e.to_string())?;
        Ok(eng_inner.get_parameter(&id))
    } else {
        Err("Engine not running".to_string())
    }
}
