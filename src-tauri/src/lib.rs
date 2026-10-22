use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

use kicks_core::config::KicksConfig;
use kicks_core::midi::MidiConfig;
use kicks_core::persistence;
use kicks_core::preset::PresetCollection;
use kicks_core::scene::SceneCollection;
use kicks_core::signal_chain::SignalChain;
use kicks_dsp::param::ParamSender;
use kicks_dsp::{CpalAudioIO, JackAudioIO, KicksEngine};
use tauri::Manager;

mod ai;
mod commands;
mod midi;
mod path_guard;
mod sync;

/// Maximum number of undo/redo steps to keep.
const UNDO_LIMIT: usize = 50;

/// Shared application state managed by Tauri.
pub struct AppState {
    /// The DSP engine, shared with the CPAL audio callback via Arc<Mutex<>>.
    pub engine: Mutex<Option<Arc<Mutex<KicksEngine>>>>,
    /// The CPAL audio I/O backend (stream handles kept alive here).
    pub audio_io: Mutex<Option<CpalAudioIO>>,
    /// The JACK audio I/O backend (active when JACK is selected).
    pub jack_audio_io: Mutex<Option<JackAudioIO>>,
    /// Lock-free parameter channel: main thread pushes, audio callback drains.
    /// Created when the engine starts, cleared when it stops.
    pub param_tx: Arc<Mutex<Option<ParamSender>>>,
    pub signal_chain: Mutex<SignalChain>,
    pub presets: Mutex<PresetCollection>,
    pub scenes: Mutex<SceneCollection>,
    pub config: Mutex<KicksConfig>,
    pub midi_config: Arc<Mutex<MidiConfig>>,
    pub midi_manager: Mutex<midi::MidiManager>,
    /// Stack of previous signal chain states for undo (newest first).
    pub undo_chain: Mutex<Vec<SignalChain>>,
    /// Stack of undone states for redo (newest first).
    pub redo_chain: Mutex<Vec<SignalChain>>,
    /// CPU load from audio callback: value is (percentage * 1000) as u64.
    pub cpu_load: Arc<AtomicU64>,
    /// Active WAV recording (DI + processed), if any.
    pub recording: Mutex<Option<kicks_dsp::recorder::Recording>>,
    /// Files the user picked in a native dialog (allowed outside library dirs).
    pub path_grants: path_guard::PathGrants,
}

#[cfg(test)]
impl AppState {
    /// In-memory state for command tests (no disk, no audio device).
    pub(crate) fn for_test() -> Self {
        Self {
            engine: Mutex::new(None),
            audio_io: Mutex::new(None),
            jack_audio_io: Mutex::new(None),
            param_tx: Arc::new(Mutex::new(None)),
            signal_chain: Mutex::new(SignalChain::default()),
            presets: Mutex::new(PresetCollection::default()),
            scenes: Mutex::new(SceneCollection::default()),
            config: Mutex::new(KicksConfig::default()),
            midi_config: Arc::new(Mutex::new(MidiConfig::default())),
            midi_manager: Mutex::new(midi::MidiManager::new()),
            undo_chain: Mutex::new(Vec::new()),
            redo_chain: Mutex::new(Vec::new()),
            cpu_load: Arc::new(AtomicU64::new(0)),
            recording: Mutex::new(None),
            path_grants: path_guard::PathGrants::default(),
        }
    }
}

/// Configure tracing/logging for the application.
fn setup_logging() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "kicks=info,app=info".into()),
        )
        .with_file(true)
        .with_line_number(true)
        .init();
}

/// Load persisted state from disk, falling back to defaults.
fn load_state() -> (PresetCollection, KicksConfig, SignalChain) {
    if let Err(e) = persistence::ensure_config_dir() {
        tracing::warn!("Could not create config directory: {}", e);
    }

    let presets = persistence::load_presets();
    let config = persistence::load_config();
    let mut signal_chain = persistence::load_signal_chain();
    let added = signal_chain.add_missing_default_slots();
    if !added.is_empty() {
        tracing::info!("Added new pedals to saved chain (bypassed): {:?}", added);
    }

    (presets, config, signal_chain)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    setup_logging();

    let (presets, config, signal_chain) = load_state();
    let midi_config = persistence::load_midi_config();
    let scenes = persistence::load_scenes();

    tauri::Builder::default()
        .manage(AppState {
            engine: Mutex::new(None),
            audio_io: Mutex::new(None),
            jack_audio_io: Mutex::new(None),
            param_tx: Arc::new(Mutex::new(None)),
            signal_chain: Mutex::new(signal_chain),
            presets: Mutex::new(presets),
            scenes: Mutex::new(scenes),
            config: Mutex::new(config),
            midi_config: Arc::new(Mutex::new(midi_config)),
            midi_manager: Mutex::new(midi::MidiManager::new()),
            undo_chain: Mutex::new(Vec::new()),
            redo_chain: Mutex::new(Vec::new()),
            cpu_load: Arc::new(AtomicU64::new(0)),
            recording: Mutex::new(None),
            path_grants: path_guard::PathGrants::default(),
        })
        .setup(|app| {
            // NOTE: tauri_plugin_log is intentionally NOT registered here —
            // setup_logging() already installs a global tracing subscriber,
            // and registering a second logger panics at startup.

            app.handle().plugin(tauri_plugin_dialog::init())?;

            let _window = app.get_webview_window("main");
            tracing::info!("Kicks Guitar Workstation started");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Engine
            commands::engine::start_engine,
            commands::engine::stop_engine,
            commands::engine::engine_status,
            commands::engine::set_parameter,
            commands::engine::get_parameter,
            commands::engine::get_audio_levels,
            commands::engine::get_cpu_load,
            commands::engine::get_audio_status,
            // Recording
            commands::recording::start_recording,
            commands::recording::stop_recording,
            commands::recording::recording_status,
            // Signal Chain
            commands::signal_chain::get_signal_chain,
            commands::signal_chain::build_default_chain,
            commands::signal_chain::update_slot,
            commands::signal_chain::toggle_slot,
            commands::signal_chain::move_slot,
            commands::signal_chain::undo_signal_chain,
            commands::signal_chain::redo_signal_chain,
            // Presets
            commands::presets::list_presets,
            commands::presets::save_preset,
            commands::presets::load_preset,
            commands::presets::delete_preset,
            commands::presets::rename_preset,
            // IR Browser & Loading
            commands::ir::list_ir_files,
            commands::ir::pick_ir_file,
            commands::ir::scan_ir_directory,
            commands::ir::load_ir_to_cab,
            commands::ir::get_cab_ir_info,
            commands::ir::clear_cab_ir,
            // NAM Model
            commands::nam::list_nam_files,
            commands::nam::scan_nam_directory,
            commands::nam::load_nam_model,
            commands::nam::get_nam_info,
            commands::nam::clear_nam_model,
            // Tuner
            commands::tuner::get_tuner_info,
            // Metronome
            commands::metronome::get_metronome_state,
            // Looper
            commands::looper::get_looper_state,
            commands::looper::trigger_looper_mode,
            commands::looper::looper_undo,
            commands::looper::looper_clear,
            // Bass Amp / Chain modes
            commands::bass_amp::switch_to_bass_chain,
            commands::bass_amp::switch_to_practice_chain,
            commands::bass_amp::switch_to_looper_chain,
            // Settings
            commands::settings::get_settings,
            commands::settings::save_settings,
            commands::settings::list_audio_devices,
            commands::settings::get_version,
            // MIDI
            commands::midi::list_midi_devices,
            commands::midi::get_midi_config,
            commands::midi::save_midi_config,
            commands::midi::connect_midi_device,
            commands::midi::disconnect_midi_device,
            commands::midi::poll_midi_events,
            commands::midi::set_midi_learn,
            // AI
            commands::ai::generate_ai_preset,
            commands::ai::apply_ai_preset,
            // Amp Presets
            commands::amp_presets::list_amp_presets,
            commands::amp_presets::apply_amp_preset,
            // Scenes (Live Mode)
            commands::scenes::list_scenes,
            commands::scenes::get_scene,
            commands::scenes::save_scene,
            commands::scenes::update_scene,
            commands::scenes::load_scene,
            commands::scenes::delete_scene,
            commands::scenes::rename_scene,
            commands::scenes::reorder_scene,
            commands::scenes::next_scene,
            commands::scenes::prev_scene,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Kicks");
}
