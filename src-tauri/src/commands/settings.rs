use crate::sync::LockOrRecover;
use kicks_core::config::{AiProvider, AudioBackend, EngineMode, KicksConfig};
use kicks_dsp::param::param_channel;
use kicks_dsp::{AudioConfig, AudioEngine};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::State;

use crate::AppState;

/// Serializable settings payload for the frontend.
#[derive(Debug, Serialize, Deserialize)]
pub struct SettingsPayload {
    pub guitarix_host: String,
    pub guitarix_port: u16,
    pub engine_mode: String,
    pub jack_client_name: String,

    // Audio backend
    pub audio_backend: String,

    // CPAL audio device settings
    pub sample_rate: u32,
    pub buffer_size: u32,
    pub audio_device: String,
    pub input_device: String,
    pub output_device: String,

    pub ir_directories: Vec<String>,
    pub nam_directories: Vec<String>,
    pub preset_directories: Vec<String>,

    // AI provider settings
    pub ai_provider: String,
    pub ai_endpoint_url: String,
    pub ai_api_key: String,
    pub ai_model: String,
}

impl From<&KicksConfig> for SettingsPayload {
    fn from(cfg: &KicksConfig) -> Self {
        Self {
            guitarix_host: cfg.guitarix_host.clone(),
            guitarix_port: cfg.guitarix_port,
            engine_mode: format!("{:?}", cfg.active_engine),
            jack_client_name: cfg.jack_client_name.clone(),
            audio_backend: format!("{:?}", cfg.audio_backend),
            sample_rate: cfg.sample_rate,
            buffer_size: cfg.buffer_size,
            audio_device: cfg.audio_device.clone(),
            input_device: cfg.input_device.clone(),
            output_device: cfg.output_device.clone(),
            ir_directories: cfg.ir_directories.clone(),
            nam_directories: cfg.nam_directories.clone(),
            preset_directories: cfg.preset_directories.clone(),
            ai_provider: format!("{:?}", cfg.ai_provider),
            ai_endpoint_url: cfg.ai_endpoint_url.clone(),
            ai_api_key: cfg.ai_api_key.clone(),
            ai_model: cfg.ai_model.clone(),
        }
    }
}

/// Get the current application settings.
#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> SettingsPayload {
    let config = state.config.lock_or_recover();
    SettingsPayload::from(&*config)
}

/// Update and persist application settings.
///
/// When audio configuration changes (sample rate, buffer size, device, or backend)
/// and the engine is running, the audio stream is automatically restarted
/// with the new settings — no manual stop/start needed.
#[tauri::command]
pub fn save_settings(state: State<'_, AppState>, settings: SettingsPayload) -> Result<(), String> {
    // ── 1. Snapshot old audio config and detect changes ──
    let (old_sr, old_bs, old_dev, old_in, old_out, old_backend) = {
        let cfg = state.config.lock().map_err(|e| e.to_string())?;
        (
            cfg.sample_rate,
            cfg.buffer_size,
            cfg.audio_device.clone(),
            cfg.input_device.clone(),
            cfg.output_device.clone(),
            cfg.audio_backend.clone(),
        )
    };
    let new_backend = match settings.audio_backend.as_str() {
        "Jack" => AudioBackend::Jack,
        _ => AudioBackend::Cpal,
    };
    let audio_changed = old_sr != settings.sample_rate
        || old_bs != settings.buffer_size
        || old_dev != settings.audio_device
        || old_in != settings.input_device
        || old_out != settings.output_device
        || old_backend != new_backend;

    // ── 2. Apply all changes to the in-memory config ──
    {
        let mut config = state.config.lock().map_err(|e| e.to_string())?;

        config.guitarix_host = settings.guitarix_host;
        config.guitarix_port = settings.guitarix_port;
        config.jack_client_name = settings.jack_client_name;

        config.audio_backend = new_backend;
        config.sample_rate = settings.sample_rate;
        config.buffer_size = settings.buffer_size;
        config.audio_device = settings.audio_device.clone();
        config.input_device = settings.input_device.clone();
        config.output_device = settings.output_device.clone();

        config.ir_directories = settings.ir_directories;
        config.nam_directories = settings.nam_directories;
        config.preset_directories = settings.preset_directories;

        config.active_engine = match settings.engine_mode.as_str() {
            "Internal" => EngineMode::Internal,
            "Guitarix" => EngineMode::Guitarix,
            "Auto" => EngineMode::Auto,
            _ => EngineMode::Auto,
        };

        config.ai_provider = match settings.ai_provider.as_str() {
            "OpenAI" => AiProvider::OpenAI,
            _ => AiProvider::Anthropic,
        };
        config.ai_endpoint_url = settings.ai_endpoint_url;
        config.ai_api_key = settings.ai_api_key;
        config.ai_model = settings.ai_model;
    }

    // ── 3. Restart engine if audio config changed ──

    if audio_changed {
        let engine_opt = state.engine.lock().map_err(|e| e.to_string())?;
        if let Some(ref eng_arc) = *engine_opt {
            let sr = settings.sample_rate as f64;
            let bs = settings.buffer_size;
            let audio_device = settings.audio_device.clone();
            let input_device = settings.input_device.clone();
            let output_device = settings.output_device.clone();
            let backend = match settings.audio_backend.as_str() {
                "Jack" => AudioBackend::Jack,
                _ => AudioBackend::Cpal,
            };

            // Re-init the engine at the new sample rate/buffer size
            {
                let mut eng = eng_arc.lock().map_err(|e| e.to_string())?;
                eng.init(sr, bs)
                    .map_err(|e| format!("Engine re-init failed: {}", e))?;
            }

            // Stop current audio I/O
            let mut audio_io = state.audio_io.lock().map_err(|e| e.to_string())?;
            if let Some(ref mut io) = *audio_io {
                io.stop();
            }
            *audio_io = None;
            drop(audio_io);

            let mut jack_io = state.jack_audio_io.lock().map_err(|e| e.to_string())?;
            if let Some(ref mut io) = *jack_io {
                io.stop();
            }
            *jack_io = None;
            drop(jack_io);

            match backend {
                AudioBackend::Jack => {
                    let jack_client_name = {
                        let cfg = state.config.lock().map_err(|e| e.to_string())?;
                        cfg.jack_client_name.clone()
                    };
                    let mut jack_io = state.jack_audio_io.lock().map_err(|e| e.to_string())?;
                    *jack_io = Some(kicks_dsp::JackAudioIO::new(kicks_dsp::JackConfig {
                        client_name: if jack_client_name.is_empty() {
                            "Kicks".to_string()
                        } else {
                            jack_client_name
                        },
                    }));
                    if let Some(ref mut io) = *jack_io {
                        let cpu_load = Arc::clone(&state.cpu_load);
                        {
                            let (param_tx, param_rx) = param_channel();
                            *state.param_tx.lock().map_err(|e| e.to_string())? = Some(param_tx);
                            io.start(eng_arc.clone(), param_rx, Some(cpu_load))
                        }
                        .map_err(|e| format!("Failed to restart JACK: {}", e))?;
                    }
                }
                AudioBackend::Cpal => {
                    // Create fresh lock-free param channel for the new stream
                    let (param_tx, param_rx) = param_channel();
                    *state.param_tx.lock().map_err(|e| e.to_string())? = Some(param_tx);

                    // Restart CPAL stream with new config
                    let audio_config = AudioConfig {
                        sample_rate: sr,
                        buffer_size: bs,
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
                    *audio_io = Some(kicks_dsp::CpalAudioIO::new());
                    if let Some(ref mut io) = *audio_io {
                        let cpu_load = Arc::clone(&state.cpu_load);
                        io.start(eng_arc.clone(), audio_config, param_rx, Some(cpu_load))
                            .map_err(|e| format!("Failed to restart CPAL: {}", e))?;
                    }
                }
            }

            tracing::info!(
                "Audio engine restarted with new config: {} Hz, buffer {}, backend {:?}",
                sr,
                bs,
                backend
            );
        }
        // If engine wasn't running, the new config will be picked up on next start
    }

    // ── 4. Persist to disk ──
    let config = state.config.lock().map_err(|e| e.to_string())?;
    tracing::info!(
        "Settings saved (sample_rate: {}, buffer_size: {}, backend: {:?})",
        config.sample_rate,
        config.buffer_size,
        config.audio_backend
    );

    if let Err(e) = kicks_core::persistence::save_config(&config) {
        tracing::warn!("Failed to persist config: {}", e);
    }

    Ok(())
}

/// Enumerate available audio devices via CPAL.
#[tauri::command]
pub fn list_audio_devices() -> Vec<kicks_dsp::DeviceInfo> {
    kicks_dsp::list_audio_devices()
}

/// Get the application version.
#[tauri::command]
pub fn get_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}
