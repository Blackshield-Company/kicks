use crate::nam::NamModelInfo;
use crate::plugins::{IrInfo, PluginRegistry};

/// The trait each audio engine backend must implement.
pub trait AudioEngine: Send {
    /// Initialize the engine with the given sample rate and buffer size.
    fn init(&mut self, sample_rate: f64, buffer_size: u32) -> anyhow::Result<()>;

    /// Process one buffer of audio (interleaved stereo or mono).
    fn process(&mut self, input: &[f32], output: &mut [f32]) -> anyhow::Result<()>;

    /// Shut down the engine and release resources.
    fn shutdown(&mut self) -> anyhow::Result<()>;

    /// Get the current value of a named parameter.
    fn get_parameter(&self, id: &str) -> Option<f32>;

    /// Set a named parameter, with smooth transition.
    fn set_parameter(&mut self, id: &str, value: f32);
}

/// Which preset chain to build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainKind {
    Default,
    Bass,
    Practice,
    Looper,
}

/// Build and initialise a chain. Allocates heavily, so call it **without**
/// holding the engine lock, then install it with [`KicksEngine::swap_registry`].
pub fn build_registry(kind: ChainKind, sample_rate: f64) -> anyhow::Result<PluginRegistry> {
    let mut r = PluginRegistry::new();
    match kind {
        ChainKind::Default => r.build_default_chain(),
        ChainKind::Bass => r.build_bass_chain(),
        ChainKind::Practice => r.build_practice_chain(),
        ChainKind::Looper => r.build_looper_chain(),
    }
    r.init_all(sample_rate)?;
    Ok(r)
}

/// The default internal processing engine using kicks-dsp plugins.
pub struct KicksEngine {
    plugin_registry: PluginRegistry,
    sample_rate: f64,
    buffer_size: u32,
    /// Active WAV capture, if recording (see [`crate::recorder`]).
    record_tap: Option<crate::recorder::RecordTap>,
}

impl Default for KicksEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl KicksEngine {
    pub fn new() -> Self {
        let mut registry = PluginRegistry::new();
        registry.build_default_chain();
        Self {
            plugin_registry: registry,
            sample_rate: 48000.0,
            buffer_size: 256,
            record_tap: None,
        }
    }

    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    /// Replace the whole chain in O(1). Returns the old chain so the caller
    /// can drop it after releasing the engine lock (dropping frees memory).
    pub fn swap_registry(&mut self, new: PluginRegistry) -> PluginRegistry {
        std::mem::replace(&mut self.plugin_registry, new)
    }

    /// Install a convolver prepared with [`crate::Cab::prepare_ir`].
    /// Returns the replaced convolver (drop it outside the lock).
    pub fn install_cab_ir(
        &mut self, path: String, conv: crate::Convolver,
    ) -> Result<Option<crate::Convolver>, crate::Convolver> {
        self.plugin_registry.install_cab_ir(path, conv)
    }

    /// Install (or remove, with `None`) the recording tap. Returns the
    /// previous tap; drop it after releasing the engine lock.
    pub fn set_record_tap(
        &mut self, tap: Option<crate::recorder::RecordTap>,
    ) -> Option<crate::recorder::RecordTap> {
        std::mem::replace(&mut self.record_tap, tap)
    }

    pub fn is_recording(&self) -> bool {
        self.record_tap.is_some()
    }

    /// Reorder the running chain to follow `order` (plugin names).
    /// See [`PluginRegistry::reorder`].
    pub fn reorder_plugins<S: AsRef<str>>(&mut self, order: &[S]) -> bool {
        self.plugin_registry.reorder(order)
    }

    /// Plugin names in processing order.
    pub fn plugin_names(&self) -> Vec<String> {
        self.plugin_registry
            .plugin_names()
            .into_iter()
            .map(str::to_owned)
            .collect()
    }

    /// Apply a parameter directly (control thread, engine lock held).
    /// Accepts the same ids as the param queue, including `"plugin/param"`,
    /// `"plugin/enabled"` and `"plugin/wet_dry"`.
    pub fn apply_param(&mut self, id: &str, value: f32) {
        self.plugin_registry.set_parameter(id, value);
    }

    /// Set a parameter on a specific plugin by name.
    pub fn set_parameter_on_plugin(&mut self, plugin_name: &str, id: &str, value: f32) -> bool {
        self.plugin_registry
            .set_parameter_on_plugin(plugin_name, id, value)
    }

    /// Enable or disable a plugin by name.
    pub fn set_plugin_enabled(&mut self, plugin_name: &str, enabled: bool) -> bool {
        self.plugin_registry
            .set_plugin_enabled(plugin_name, enabled)
    }

    /// Update only the parameter value cache (HashMap) without iterating plugins.
    /// Used by the main thread for immediate `get_parameter` consistency.
    /// The audio callback applies the full plugin update via the SPSC queue drain.
    pub fn set_parameter_value(&mut self, id: &str, value: f32) {
        self.plugin_registry.set_parameter_value(id, value);
    }

    /// Return the current per-plugin RMS audio levels (0..1 range).
    /// Updated every `process_all` cycle (~5 ms).
    pub fn audio_levels(&self) -> Vec<f32> {
        self.plugin_registry.audio_levels()
    }

    /// Load an impulse response into the Cab plugin.
    pub fn load_ir_to_cab(&mut self, path: String, ir_data: Vec<f32>, ir_sample_rate: f32) -> bool {
        self.plugin_registry
            .load_ir_to_cab(path, ir_data, ir_sample_rate)
    }

    /// Get info about the currently loaded IR in the Cab plugin.
    pub fn cab_ir_info(&self) -> Option<IrInfo> {
        self.plugin_registry.cab_ir_info()
    }

    /// Clear the loaded IR from the Cab plugin.
    pub fn clear_cab_ir(&mut self) -> bool {
        self.plugin_registry.clear_cab_ir()
    }

    /// Load a NAM neural model into the Nam plugin.
    pub fn load_nam_to_plugin(
        &mut self, path: String, neural_model: crate::nam::NeuralModel,
    ) -> bool {
        self.plugin_registry.load_nam_to_plugin(path, neural_model)
    }

    /// Get info about the currently loaded NAM model.
    pub fn nam_model_info(&self) -> Option<NamModelInfo> {
        self.plugin_registry.nam_model_info()
    }

    /// Clear the loaded NAM model.
    pub fn clear_nam_model(&mut self) -> bool {
        self.plugin_registry.clear_nam_model()
    }

    /// Get tuner info: (frequency_hz, note_name, cents_deviation, confidence).
    pub fn tuner_info(&self) -> Option<(f32, String, f32, f32)> {
        self.plugin_registry.tuner_info()
    }

    /// Get metronome state: (bpm, beats_per_bar, running).
    pub fn metronome_state(&self) -> Option<(f32, u8, bool)> {
        self.plugin_registry.metronome_state()
    }

    /// Get looper state: (mode_str, loop_time_seconds, has_loop).
    pub fn looper_state(&self) -> Option<(String, f32, bool)> {
        self.plugin_registry.looper_state()
    }

    /// Trigger a looper mode change.
    pub fn trigger_looper_mode(&mut self, mode_value: f32) -> bool {
        self.plugin_registry.trigger_looper_mode(mode_value)
    }

    /// Undo the last looper overdub.
    pub fn looper_undo(&mut self) -> bool {
        self.plugin_registry.looper_undo()
    }

    /// Clear the looper buffer.
    pub fn looper_clear(&mut self) -> bool {
        self.plugin_registry.looper_clear()
    }

    /// Build a default guitar signal chain.
    pub fn build_default_chain(&mut self) {
        self.plugin_registry.build_default_chain();
        self.plugin_registry.init_all(self.sample_rate).ok();
    }

    /// Build a bass-oriented signal chain.
    pub fn build_bass_chain(&mut self) {
        self.plugin_registry.build_bass_chain();
        self.plugin_registry.init_all(self.sample_rate).ok();
    }

    /// Build a practice chain with tuner + metronome.
    pub fn build_practice_chain(&mut self) {
        self.plugin_registry.build_practice_chain();
        self.plugin_registry.init_all(self.sample_rate).ok();
    }

    /// Build a looper-focused chain.
    pub fn build_looper_chain(&mut self) {
        self.plugin_registry.build_looper_chain();
        self.plugin_registry.init_all(self.sample_rate).ok();
    }
}

impl KicksEngine {
    /// Real-time entry point used by every audio backend: apply queued
    /// parameter changes, then process one block.
    ///
    /// Allocation-free (see `tests/no_alloc.rs`). Errors from plugins are
    /// turned into silence rather than propagated.
    pub fn process_block(
        &mut self, rx: &mut crate::param::ParamConsumer, input: &[f32], output: &mut [f32],
    ) {
        while let Some((key, value)) = crate::param::try_recv(rx) {
            self.plugin_registry.set_parameter_rt(key.as_str(), value);
        }
        if self.plugin_registry.process_all(input, output).is_err() {
            output.fill(0.0);
        }
        if let Some(tap) = self.record_tap.as_mut() {
            tap.push(input, output);
        }
    }
}

impl AudioEngine for KicksEngine {
    fn init(&mut self, sample_rate: f64, buffer_size: u32) -> anyhow::Result<()> {
        self.sample_rate = sample_rate;
        self.buffer_size = buffer_size;
        self.plugin_registry.init_all(sample_rate)?;
        tracing::info!(
            "KicksEngine initialized at {} Hz, buffer size {}",
            sample_rate,
            buffer_size
        );
        Ok(())
    }

    fn process(&mut self, input: &[f32], output: &mut [f32]) -> anyhow::Result<()> {
        self.plugin_registry.process_all(input, output)
    }

    fn shutdown(&mut self) -> anyhow::Result<()> {
        tracing::info!("KicksEngine shutting down");
        Ok(())
    }

    fn get_parameter(&self, id: &str) -> Option<f32> {
        self.plugin_registry.get_parameter(id)
    }

    fn set_parameter(&mut self, id: &str, value: f32) {
        self.plugin_registry.set_parameter(id, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swap_registry_installs_prebuilt_chain() {
        let mut eng = KicksEngine::new();
        eng.init(48_000.0, 256).unwrap();
        let bass = build_registry(ChainKind::Bass, eng.sample_rate()).unwrap();
        let old = eng.swap_registry(bass);
        assert!(!old.is_empty());
        let mut out = vec![0.0; 256];
        eng.process(&[0.1; 256], &mut out).unwrap();
        assert!(out.iter().all(|s| s.is_finite()));
        assert_eq!(old.len(), 16);
        assert_eq!(eng.audio_levels().len(), 5);
    }

    #[test]
    fn process_block_feeds_record_tap_with_dry_and_wet() {
        let path =
            std::env::temp_dir().join(format!("kicks-engine-rec-{}.wav", std::process::id()));
        let mut eng = KicksEngine::new();
        eng.init(48_000.0, 256).unwrap();
        let (_tx, mut rx) = crate::param::param_channel();
        let (tap, rec) = crate::recorder::Recording::start(&path, 48_000).unwrap();
        assert!(eng.set_record_tap(Some(tap)).is_none());
        assert!(eng.is_recording());
        let input = [0.25f32; 256];
        let mut out = [0.0f32; 256];
        for _ in 0..4 {
            eng.process_block(&mut rx, &input, &mut out);
        }
        drop(eng.set_record_tap(None));
        let sum = rec.stop().unwrap();
        assert_eq!(sum.frames, 1024);
        let mut r = hound::WavReader::open(&path).unwrap();
        let s: Vec<f32> = r.samples::<f32>().map(Result::unwrap).collect();
        assert_eq!(s[s.len() - 2], 0.25, "channel 1 is the dry DI");
        assert_eq!(
            s[s.len() - 1],
            out[255],
            "channel 2 is the processed output"
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn install_cab_ir_returns_previous() {
        let mut eng = KicksEngine::new();
        eng.init(48_000.0, 256).unwrap();
        let ir = vec![1.0, 0.5, 0.25];
        let a = crate::Cab::prepare_ir(&ir, 48_000.0);
        assert!(matches!(eng.install_cab_ir("a.wav".into(), a), Ok(None)));
        let b = crate::Cab::prepare_ir(&ir, 48_000.0);
        assert!(matches!(eng.install_cab_ir("b.wav".into(), b), Ok(Some(_))));
        assert_eq!(eng.cab_ir_info().unwrap().path, "b.wav");
    }
}
