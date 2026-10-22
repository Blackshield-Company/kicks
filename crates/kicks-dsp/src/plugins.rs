use std::any::Any;
use std::collections::HashMap;
use std::f32::consts::PI;

use crate::bass_amp::BassAmp;
use crate::convolution::Convolver;
use crate::looper::Looper;
use crate::metronome::Metronome;
use crate::nam::{NamModelInfo, NeuralModel};
use crate::tuner::Tuner;

// ═══════════════════════════════════════════════════════════════════════════════
// DSP Utilities
// ═══════════════════════════════════════════════════════════════════════════════

/// Compute the RMS (root-mean-square) level of a sample buffer, normalised to
/// the 0..1 range.  Used for VU-meter-style audio level visualisation.
///
/// Returns 0.0 for empty buffers.  Clamps the result to 1.0 so it can be used
/// directly as a meter fraction.
pub fn compute_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = samples.iter().map(|&s| s * s).sum();
    (sum_sq / samples.len() as f32).sqrt().min(1.0)
}

/// Biquad filter (Direct Form 1).
///
/// Supports low/high shelving and peaking filter types, recomputing coefficients
/// whenever a parameter changes.
struct BiquadFilter {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl BiquadFilter {
    fn process(&mut self, sample: f32) -> f32 {
        let out = self.b0 * sample + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = sample;
        self.y2 = self.y1;
        self.y1 = out;
        out
    }

    /// Low-shelving filter. gain_db: boost/cut in dB, shelf_freq: corner Hz.
    fn design_low_shelf(sample_rate: f32, gain_db: f32, shelf_freq: f32, q: f32) -> Self {
        let a = 10_f32.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * shelf_freq / sample_rate;
        let sin_w0 = w0.sin();
        let cos_w0 = w0.cos();
        let alpha = sin_w0 / (2.0 * q);
        let sqrt_2a = (2.0 * a).sqrt();

        let b0 = a * ((a + 1.0) - (a - 1.0) * cos_w0 + 2.0 * sqrt_2a * alpha);
        let b1 = 2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w0);
        let b2 = a * ((a + 1.0) - (a - 1.0) * cos_w0 - 2.0 * sqrt_2a * alpha);
        let a0 = (a + 1.0) + (a - 1.0) * cos_w0 + 2.0 * sqrt_2a * alpha;
        let a1 = -2.0 * ((a - 1.0) + (a + 1.0) * cos_w0);
        let a2 = (a + 1.0) + (a - 1.0) * cos_w0 - 2.0 * sqrt_2a * alpha;

        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    /// High-shelving filter. gain_db: boost/cut in dB, shelf_freq: corner Hz.
    fn design_high_shelf(sample_rate: f32, gain_db: f32, shelf_freq: f32, q: f32) -> Self {
        let a = 10_f32.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * shelf_freq / sample_rate;
        let sin_w0 = w0.sin();
        let cos_w0 = w0.cos();
        let alpha = sin_w0 / (2.0 * q);
        let sqrt_2a = (2.0 * a).sqrt();

        let b0 = a * ((a + 1.0) + (a - 1.0) * cos_w0 + 2.0 * sqrt_2a * alpha);
        let b1 = -2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w0);
        let b2 = a * ((a + 1.0) + (a - 1.0) * cos_w0 - 2.0 * sqrt_2a * alpha);
        let a0 = (a + 1.0) - (a - 1.0) * cos_w0 + 2.0 * sqrt_2a * alpha;
        let a1 = 2.0 * ((a - 1.0) - (a + 1.0) * cos_w0);
        let a2 = (a + 1.0) - (a - 1.0) * cos_w0 - 2.0 * sqrt_2a * alpha;

        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    /// Peaking (bell) filter. gain_db: boost/cut in dB, center_freq: Hz.
    fn design_peaking(sample_rate: f32, gain_db: f32, center_freq: f32, q: f32) -> Self {
        let a = 10_f32.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * center_freq / sample_rate;
        let sin_w0 = w0.sin();
        let cos_w0 = w0.cos();
        let alpha = sin_w0 / (2.0 * q);

        let b0 = 1.0 + alpha * a;
        let b1 = -2.0 * cos_w0;
        let b2 = 1.0 - alpha * a;
        let a0 = 1.0 + alpha / a;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha / a;

        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    /// Low-pass filter (12 dB/oct).
    fn design_lowpass(sample_rate: f32, cutoff_hz: f32, q: f32) -> Self {
        let w0 = 2.0 * PI * cutoff_hz / sample_rate;
        let sin_w0 = w0.sin();
        let cos_w0 = w0.cos();
        let alpha = sin_w0 / (2.0 * q);

        let b0 = (1.0 - cos_w0) / 2.0;
        let b1 = 1.0 - cos_w0;
        let b2 = (1.0 - cos_w0) / 2.0;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha;

        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    /// High-pass filter (12 dB/oct).
    fn design_highpass(sample_rate: f32, cutoff_hz: f32, q: f32) -> Self {
        let w0 = 2.0 * PI * cutoff_hz / sample_rate;
        let sin_w0 = w0.sin();
        let cos_w0 = w0.cos();
        let alpha = sin_w0 / (2.0 * q);

        let b0 = (1.0 + cos_w0) / 2.0;
        let b1 = -(1.0 + cos_w0);
        let b2 = (1.0 + cos_w0) / 2.0;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha;

        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }
}

/// A simple delay line backed by a circular buffer.
struct DelayLine {
    buffer: Vec<f32>,
    write_pos: usize,
    capacity: usize,
}

impl DelayLine {
    fn new(max_delay_samples: usize) -> Self {
        Self {
            buffer: vec![0.0; max_delay_samples],
            write_pos: 0,
            capacity: max_delay_samples,
        }
    }

    /// Write a sample into the delay line.
    fn write(&mut self, sample: f32) {
        self.buffer[self.write_pos] = sample;
        self.write_pos = (self.write_pos + 1) % self.capacity;
    }

    /// Read a sample from `delay_samples` ago (linear interpolation).
    fn read(&self, delay_samples: f32) -> f32 {
        let delay = delay_samples.clamp(0.0, (self.capacity - 1) as f32);
        let read_pos = (self.write_pos as f32 - delay).rem_euclid(self.capacity as f32);
        // f32 rounding in rem_euclid can yield exactly `capacity`; wrap it.
        let mut idx = read_pos as usize;
        if idx >= self.capacity {
            idx = 0;
        }
        let frac = (read_pos - read_pos.floor()).clamp(0.0, 1.0);
        let next = (idx + 1) % self.capacity;

        self.buffer[idx] * (1.0 - frac) + self.buffer[next] * frac
    }
}

/// Asymmetric waveshaper (tube-like even-order harmonics).
fn waveshape_asymmetric(sample: f32, drive: f32) -> f32 {
    let x = sample * drive;
    // Combination of soft and hard clipping
    let soft = x.tanh();
    let hard = (x * 1.5).tanh();
    // Mix asymmetric
    soft * 0.7 + hard * 0.3
}

// ═══════════════════════════════════════════════════════════════════════════════
// Plugin Trait (unchanged)
// ═══════════════════════════════════════════════════════════════════════════════

/// Trait for all DSP plugin types.
pub trait Plugin: Send {
    fn name(&self) -> &str;
    fn init(&mut self, sample_rate: f64) -> anyhow::Result<()>;
    fn process(&mut self, input: &[f32], output: &mut [f32]) -> anyhow::Result<()>;
    fn get_parameter(&self, id: &str) -> Option<f32>;
    fn set_parameter(&mut self, id: &str, value: f32);

    /// For downcasting to concrete plugin types (e.g. Cab for IR loading).
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

// ═══════════════════════════════════════════════════════════════════════════════
// Passthrough
// ═══════════════════════════════════════════════════════════════════════════════

/// A simple passthrough plugin (identity).
pub struct Passthrough;

impl Plugin for Passthrough {
    fn name(&self) -> &str {
        "passthrough"
    }

    fn init(&mut self, _sample_rate: f64) -> anyhow::Result<()> {
        Ok(())
    }

    fn process(&mut self, input: &[f32], output: &mut [f32]) -> anyhow::Result<()> {
        output.copy_from_slice(input);
        Ok(())
    }

    fn get_parameter(&self, _id: &str) -> Option<f32> {
        None
    }

    fn set_parameter(&mut self, _id: &str, _value: f32) {}

    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Boost — Simple gain stage
// ═══════════════════════════════════════════════════════════════════════════════

pub struct Boost {
    gain: f32,
}

impl Default for Boost {
    fn default() -> Self {
        Self::new()
    }
}

impl Boost {
    pub fn new() -> Self {
        Self { gain: 1.5 }
    }
}

impl Plugin for Boost {
    fn name(&self) -> &str {
        "boost"
    }

    fn init(&mut self, _sample_rate: f64) -> anyhow::Result<()> {
        Ok(())
    }

    fn process(&mut self, input: &[f32], output: &mut [f32]) -> anyhow::Result<()> {
        for (i, sample) in input.iter().enumerate() {
            output[i] = sample * self.gain;
        }
        Ok(())
    }

    fn get_parameter(&self, id: &str) -> Option<f32> {
        match id {
            "gain" => Some(self.gain / 2.0), // Normalize to 0..1
            _ => None,
        }
    }

    fn set_parameter(&mut self, id: &str, value: f32) {
        if id == "gain" {
            self.gain = value * 2.0; // 0..1 → 0..2
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Amp — Pre-gain → 3-band EQ → Waveshaper → Master
// ═══════════════════════════════════════════════════════════════════════════════

pub struct Amp {
    /// Input gain (0..1 mapped to 0..10)
    gain: f32,
    /// Master volume (0..1)
    master: f32,
    /// Bass shelf gain in dB (0..1 → -12..+12)
    bass: f32,
    /// Mid peaking gain in dB (0..1 → -12..+12)
    mid: f32,
    /// Treble shelf gain in dB (0..1 → -12..+12)
    treble: f32,
    /// Drive for waveshaper (0..1 → 1..10)
    drive: f32,
    /// Bass mode (0.0 = guitar, 1.0 = bass). Shifts EQ frequencies.
    bass_mode: f32,
    /// Sample rate for filter design
    sample_rate: f32,
    /// Pre-amp low shelf filter
    bass_filter: Option<BiquadFilter>,
    /// Mid peaking filter
    mid_filter: Option<BiquadFilter>,
    /// Treble high shelf filter
    treble_filter: Option<BiquadFilter>,
    /// Dirty flag — recalculate filters on next process
    params_dirty: bool,
}

impl Default for Amp {
    fn default() -> Self {
        Self::new()
    }
}

impl Amp {
    pub fn new() -> Self {
        Self {
            gain: 0.5,
            master: 0.7,
            bass: 0.5,
            mid: 0.5,
            treble: 0.5,
            drive: 0.5,
            bass_mode: 0.0,
            sample_rate: 48000.0,
            bass_filter: None,
            mid_filter: None,
            treble_filter: None,
            params_dirty: true,
        }
    }

    /// Returns true when in bass mode (EQ shifted to bass frequencies).
    fn is_bass_mode(&self) -> bool {
        self.bass_mode > 0.5
    }

    fn recalc_filters(&mut self) {
        let sr = self.sample_rate;
        // Map 0..1 to -12..+12 dB
        let bass_db = (self.bass * 24.0) - 12.0;
        let mid_db = (self.mid * 24.0) - 12.0;
        let treble_db = (self.treble * 24.0) - 12.0;

        if self.is_bass_mode() {
            // Bass-appropriate EQ frequencies
            self.bass_filter = Some(BiquadFilter::design_low_shelf(sr, bass_db, 100.0, 0.7));
            self.mid_filter = Some(BiquadFilter::design_peaking(sr, mid_db, 500.0, 0.7));
            self.treble_filter = Some(BiquadFilter::design_high_shelf(sr, treble_db, 4000.0, 0.7));
        } else {
            // Guitar-appropriate EQ frequencies
            self.bass_filter = Some(BiquadFilter::design_low_shelf(sr, bass_db, 250.0, 0.7));
            self.mid_filter = Some(BiquadFilter::design_peaking(sr, mid_db, 800.0, 0.7));
            self.treble_filter = Some(BiquadFilter::design_high_shelf(sr, treble_db, 3000.0, 0.7));
        }
    }

    /// Convert 0..1 drive to a gain value for the waveshaper.
    fn drive_gain(&self) -> f32 {
        1.0 + self.drive * 9.0 // 1..10
    }
}

impl Plugin for Amp {
    fn name(&self) -> &str {
        "amp"
    }

    fn init(&mut self, sample_rate: f64) -> anyhow::Result<()> {
        self.sample_rate = sample_rate as f32;
        self.params_dirty = true;
        Ok(())
    }

    fn process(&mut self, input: &[f32], output: &mut [f32]) -> anyhow::Result<()> {
        if self.params_dirty {
            self.recalc_filters();
            self.params_dirty = false;
        }

        let pre_gain = self.gain * 10.0; // 0..10
        let drive = self.drive_gain();
        let master = self.master;

        let bass = self.bass_filter.as_mut().unwrap();
        let mid = self.mid_filter.as_mut().unwrap();
        let treble = self.treble_filter.as_mut().unwrap();

        for (i, sample) in input.iter().enumerate() {
            // Pre-gain
            let mut s = sample * pre_gain;

            // 3-band EQ
            s = bass.process(s);
            s = mid.process(s);
            s = treble.process(s);

            // Distortion stage
            s = waveshape_asymmetric(s, drive);

            // Master volume
            s *= master;

            output[i] = s;
        }
        Ok(())
    }

    fn get_parameter(&self, id: &str) -> Option<f32> {
        match id {
            "gain" => Some(self.gain),
            "master" => Some(self.master),
            "bass" => Some(self.bass),
            "mid" => Some(self.mid),
            "treble" => Some(self.treble),
            "drive" => Some(self.drive),
            "bass_mode" => Some(self.bass_mode),
            _ => None,
        }
    }

    fn set_parameter(&mut self, id: &str, value: f32) {
        match id {
            "gain" => self.gain = value.clamp(0.0, 1.0),
            "master" => self.master = value.clamp(0.0, 1.0),
            "bass" => {
                self.bass = value.clamp(0.0, 1.0);
                self.params_dirty = true;
            }
            "mid" => {
                self.mid = value.clamp(0.0, 1.0);
                self.params_dirty = true;
            }
            "treble" => {
                self.treble = value.clamp(0.0, 1.0);
                self.params_dirty = true;
            }
            "drive" => self.drive = value.clamp(0.0, 1.0),
            "bass_mode" => {
                self.bass_mode = value.clamp(0.0, 1.0);
                self.params_dirty = true;
            }
            _ => {}
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Cab — Speaker cabinet simulation (biquad filters + convolution IR)
// ═══════════════════════════════════════════════════════════════════════════════

/// Information about a loaded impulse response.
#[derive(Debug, Clone)]
pub struct IrInfo {
    pub path: String,
    pub file_name: String,
    pub sample_rate: f32,
    pub length_samples: usize,
    pub length_ms: f32,
}

pub struct Cab {
    /// Level (0..1)
    level: f32,
    /// Low-cut frequency in Hz mapped from 0..1 (20..250 Hz)
    low_cut: f32,
    /// High-cut frequency in Hz mapped from 0..1 (2000..8000 Hz)
    high_cut: f32,
    sample_rate: f32,
    hp_filter: Option<BiquadFilter>,
    lp_filter: Option<BiquadFilter>,
    params_dirty: bool,
    /// Convolution engine for IR loading (None = no IR loaded).
    convolver: Option<Convolver>,
    /// Path of the currently loaded IR file.
    ir_path: Option<String>,
}

impl Default for Cab {
    fn default() -> Self {
        Self::new()
    }
}

impl Cab {
    pub fn new() -> Self {
        Self {
            level: 1.0,
            low_cut: 0.0,
            high_cut: 0.6,
            sample_rate: 48000.0,
            hp_filter: None,
            lp_filter: None,
            params_dirty: true,
            convolver: None,
            ir_path: None,
        }
    }

    fn recalc_filters(&mut self) {
        let sr = self.sample_rate;
        // Map low_cut 0..1 → 20..250 Hz
        let lc_hz = 20.0 + self.low_cut * 230.0;
        // Map high_cut 0..1 → 2000..8000 Hz
        let hc_hz = 8000.0 - (1.0 - self.high_cut) * 6000.0;

        self.hp_filter = Some(BiquadFilter::design_highpass(sr, lc_hz, 0.707));
        self.lp_filter = Some(BiquadFilter::design_lowpass(sr, hc_hz, 0.707));
    }

    /// Load an impulse response from raw float samples.
    ///
    /// `path` — display path for the IR file.
    /// `ir_data` — mono float samples of the impulse response.
    /// `ir_sample_rate` — sample rate of the IR (for metadata).
    pub fn load_ir(&mut self, path: String, ir_data: Vec<f32>, ir_sample_rate: f32) {
        let file_name = path.rsplit('/').next().unwrap_or(&path).to_string();
        self.convolver = Some(Convolver::new(&ir_data, ir_sample_rate));
        // Normalize and trim silence for best results
        if let Some(conv) = self.convolver.as_mut() {
            conv.trim(0.001);
            conv.normalize();
        }
        self.ir_path = Some(path);
        tracing::info!(
            "Cab: loaded IR '{}' ({} samples, {:.1} ms, {} Hz)",
            file_name,
            ir_data.len(),
            ir_data.len() as f32 / ir_sample_rate * 1000.0,
            ir_sample_rate
        );
    }

    /// Build a trimmed, normalised convolver. Heavy; call off the audio thread.
    pub fn prepare_ir(ir_data: &[f32], ir_sample_rate: f32) -> Convolver {
        let mut conv = Convolver::new(ir_data, ir_sample_rate);
        conv.trim(0.001);
        conv.normalize();
        conv
    }

    /// Install a convolver built by [`prepare_ir`](Self::prepare_ir).
    /// Returns the previous one so the caller can drop it off the audio thread.
    pub fn install_ir(&mut self, path: String, conv: Convolver) -> Option<Convolver> {
        self.ir_path = Some(path);
        self.convolver.replace(conv)
    }

    /// Clear the loaded IR, falling back to filter-only processing.
    pub fn clear_ir(&mut self) {
        self.convolver = None;
        self.ir_path = None;
        tracing::info!("Cab: IR cleared, using filter-only mode");
    }

    /// Get information about the currently loaded IR, if any.
    pub fn ir_info(&self) -> Option<IrInfo> {
        self.convolver.as_ref().map(|conv| {
            let path = self.ir_path.clone().unwrap_or_default();
            let file_name = path.rsplit('/').next().unwrap_or(&path).to_string();
            IrInfo {
                path,
                file_name,
                sample_rate: conv.sample_rate(),
                length_samples: conv.ir_length(),
                length_ms: conv.ir_length_ms(),
            }
        })
    }

    /// Whether an IR is currently loaded.
    pub fn has_ir(&self) -> bool {
        self.convolver.is_some()
    }
}

impl Plugin for Cab {
    fn name(&self) -> &str {
        "cab"
    }

    fn init(&mut self, sample_rate: f64) -> anyhow::Result<()> {
        self.sample_rate = sample_rate as f32;
        self.params_dirty = true;
        Ok(())
    }

    fn process(&mut self, input: &[f32], output: &mut [f32]) -> anyhow::Result<()> {
        let level = self.level;

        if let Some(conv) = self.convolver.as_mut() {
            // IR convolution mode: run through convolver + level
            conv.process(input, output);
            for sample in output.iter_mut() {
                *sample *= level;
            }
        } else {
            // Filter-only mode: high-pass + low-pass + level
            if self.params_dirty {
                self.recalc_filters();
                self.params_dirty = false;
            }

            let hp = self.hp_filter.as_mut().unwrap();
            let lp = self.lp_filter.as_mut().unwrap();

            for (i, sample) in input.iter().enumerate() {
                let mut s = *sample;
                s = hp.process(s);
                s = lp.process(s);
                s *= level;
                output[i] = s;
            }
        }
        Ok(())
    }

    fn get_parameter(&self, id: &str) -> Option<f32> {
        match id {
            "level" => Some(self.level),
            "low_cut" => Some(self.low_cut),
            "high_cut" => Some(self.high_cut),
            "ir_loaded" => Some(if self.convolver.is_some() { 1.0 } else { 0.0 }),
            _ => None,
        }
    }

    fn set_parameter(&mut self, id: &str, value: f32) {
        match id {
            "level" => self.level = value.clamp(0.0, 1.0),
            "low_cut" => {
                self.low_cut = value.clamp(0.0, 1.0);
                self.params_dirty = true;
            }
            "high_cut" => {
                self.high_cut = value.clamp(0.0, 1.0);
                self.params_dirty = true;
            }
            _ => {}
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Nam — Neural Amp Modeler
// ═══════════════════════════════════════════════════════════════════════════════

pub struct Nam {
    level: f32,
    sample_rate: f32,
    model: Option<NeuralModel>,
    model_path: Option<String>,
}

impl Default for Nam {
    fn default() -> Self {
        Self::new()
    }
}

impl Nam {
    pub fn new() -> Self {
        Self {
            level: 1.0,
            sample_rate: 48000.0,
            model: None,
            model_path: None,
        }
    }

    /// Load a NAM neural network model.
    pub fn load_nam_model(&mut self, path: String, neural_model: NeuralModel) {
        let file_name = path.rsplit('/').next().unwrap_or(&path).to_string();
        let arch = neural_model.architecture().to_string();
        let sr = neural_model.sample_rate();
        self.model = Some(neural_model);
        self.model_path = Some(path);
        tracing::info!(
            "Nam: loaded model '{}' (arch: {}, {} Hz, {} params)",
            file_name,
            arch,
            sr,
            0
        );
    }

    /// Clear the loaded NAM model, falling back to passthrough.
    pub fn clear_nam_model(&mut self) {
        self.model = None;
        self.model_path = None;
        tracing::info!("Nam: model cleared, using passthrough");
    }

    /// Get information about the currently loaded model, if any.
    pub fn nam_info(&self) -> Option<NamModelInfo> {
        self.model.as_ref().map(|m| {
            let path = self.model_path.clone().unwrap_or_default();
            let file_name = path.rsplit('/').next().unwrap_or(&path).to_string();
            NamModelInfo {
                path,
                file_name,
                architecture: m.architecture().to_string(),
                sample_rate: m.sample_rate(),
                num_parameters: 0,
            }
        })
    }

    /// Whether a model is currently loaded.
    pub fn has_nam(&self) -> bool {
        self.model.is_some()
    }
}

impl Plugin for Nam {
    fn name(&self) -> &str {
        "nam"
    }

    fn init(&mut self, sample_rate: f64) -> anyhow::Result<()> {
        self.sample_rate = sample_rate as f32;
        Ok(())
    }

    fn process(&mut self, input: &[f32], output: &mut [f32]) -> anyhow::Result<()> {
        let level = self.level;
        if input.len() != output.len() {
            anyhow::bail!(
                "Nam: input/output length mismatch ({} vs {})",
                input.len(),
                output.len()
            );
        }
        if let Some(ref mut model) = self.model {
            model.process(input, output);
        } else {
            output.copy_from_slice(input);
        }
        for sample in output.iter_mut() {
            *sample *= level;
        }
        Ok(())
    }

    fn get_parameter(&self, id: &str) -> Option<f32> {
        match id {
            "level" => Some(self.level),
            "model_loaded" => Some(if self.model.is_some() { 1.0 } else { 0.0 }),
            _ => None,
        }
    }

    fn set_parameter(&mut self, id: &str, value: f32) {
        if id == "level" {
            self.level = value.clamp(0.0, 1.0);
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Input — Passthrough for signal chain consistency
// ═══════════════════════════════════════════════════════════════════════════════

pub struct Input;

impl Plugin for Input {
    fn name(&self) -> &str {
        "input"
    }

    fn init(&mut self, _sample_rate: f64) -> anyhow::Result<()> {
        Ok(())
    }

    fn process(&mut self, input: &[f32], output: &mut [f32]) -> anyhow::Result<()> {
        output.copy_from_slice(input);
        Ok(())
    }

    fn get_parameter(&self, _id: &str) -> Option<f32> {
        None
    }

    fn set_parameter(&mut self, _id: &str, _value: f32) {}

    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Output — Master volume stage
// ═══════════════════════════════════════════════════════════════════════════════

pub struct Output {
    volume: f32,
}

impl Default for Output {
    fn default() -> Self {
        Self::new()
    }
}

impl Output {
    pub fn new() -> Self {
        Self { volume: 0.8 }
    }
}

impl Plugin for Output {
    fn name(&self) -> &str {
        "output"
    }

    fn init(&mut self, _sample_rate: f64) -> anyhow::Result<()> {
        Ok(())
    }

    fn process(&mut self, input: &[f32], output: &mut [f32]) -> anyhow::Result<()> {
        for (i, sample) in input.iter().enumerate() {
            output[i] = sample * self.volume;
        }
        Ok(())
    }

    fn get_parameter(&self, id: &str) -> Option<f32> {
        match id {
            "volume" => Some(self.volume),
            _ => None,
        }
    }

    fn set_parameter(&mut self, id: &str, value: f32) {
        if id == "volume" {
            self.volume = value.clamp(0.0, 1.0);
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Delay — Digital delay line with feedback
// ═══════════════════════════════════════════════════════════════════════════════

pub struct Delay {
    /// Delay time as fraction of max (0..1)
    time: f32,
    /// Feedback amount (0..0.99)
    feedback: f32,
    /// Wet/dry mix (0..1)
    mix: f32,
    sample_rate: f32,
    /// Maximum delay in samples (2 seconds at any sample rate)
    max_delay_samples: usize,
    /// The actual delay line
    line: Option<DelayLine>,
    /// Current delay in samples
    delay_samples: f32,
}

impl Default for Delay {
    fn default() -> Self {
        Self::new()
    }
}

impl Delay {
    pub fn new() -> Self {
        Self {
            time: 0.3,
            feedback: 0.4,
            mix: 0.3,
            sample_rate: 48000.0,
            max_delay_samples: 192000, // 2s at 96kHz
            line: None,
            delay_samples: 0.0,
        }
    }

    fn update_delay(&mut self) {
        // Map time 0..1 → 20ms..2000ms
        let delay_ms = 20.0 + self.time * 1980.0;
        self.delay_samples = (delay_ms / 1000.0) * self.sample_rate;
    }
}

impl Plugin for Delay {
    fn name(&self) -> &str {
        "delay"
    }

    fn init(&mut self, sample_rate: f64) -> anyhow::Result<()> {
        self.sample_rate = sample_rate as f32;
        self.max_delay_samples = (sample_rate as usize) * 2; // 2 seconds
        self.line = Some(DelayLine::new(self.max_delay_samples));
        self.update_delay();
        Ok(())
    }

    fn process(&mut self, input: &[f32], output: &mut [f32]) -> anyhow::Result<()> {
        let line = self.line.as_mut().unwrap();
        let mix = self.mix;
        let fb = self.feedback;
        let delay_s = self.delay_samples;

        for (i, sample) in input.iter().enumerate() {
            let wet = line.read(delay_s);
            let out = sample * (1.0 - mix) + wet * mix;
            let feedback_sample = *sample + wet * fb;
            line.write(feedback_sample);
            output[i] = out;
        }
        Ok(())
    }

    fn get_parameter(&self, id: &str) -> Option<f32> {
        match id {
            "time" => Some(self.time),
            "feedback" => Some(self.feedback),
            "mix" => Some(self.mix),
            _ => None,
        }
    }

    fn set_parameter(&mut self, id: &str, value: f32) {
        match id {
            "time" => {
                self.time = value.clamp(0.0, 1.0);
                self.update_delay();
            }
            "feedback" => self.feedback = value.clamp(0.0, 0.99),
            "mix" => self.mix = value.clamp(0.0, 1.0),
            _ => {}
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Reverb — Schroeder reverberator
// ═══════════════════════════════════════════════════════════════════════════════

/// A comb filter used in the Schroeder reverb.
struct CombFilter {
    buffer: Vec<f32>,
    write_pos: usize,
    delay: usize,
    feedback: f32,
    damping: f32,
    damp_state: f32,
}

impl CombFilter {
    fn new(max_delay: usize) -> Self {
        Self {
            buffer: vec![0.0; max_delay],
            write_pos: 0,
            delay: max_delay.saturating_sub(1),
            feedback: 0.5,
            damping: 0.5,
            damp_state: 0.0,
        }
    }

    fn process(&mut self, sample: f32) -> f32 {
        let read_pos = (self.write_pos + self.buffer.len() - self.delay) % self.buffer.len();
        let delayed = self.buffer[read_pos];
        // Lowpass filter on feedback path for damping
        self.damp_state = delayed * (1.0 - self.damping) + self.damp_state * self.damping;
        let out = sample + self.damp_state * self.feedback;
        self.buffer[self.write_pos] = out;
        self.write_pos = (self.write_pos + 1) % self.buffer.len();
        out
    }

    fn set_delay(&mut self, samples: usize) {
        self.delay = samples.min(self.buffer.len().saturating_sub(1));
    }
}

/// An allpass filter used in the Schroeder reverb.
struct AllpassFilter {
    buffer: Vec<f32>,
    write_pos: usize,
    delay: usize,
    gain: f32,
}

impl AllpassFilter {
    fn new(max_delay: usize) -> Self {
        Self {
            buffer: vec![0.0; max_delay],
            write_pos: 0,
            delay: max_delay.saturating_sub(1),
            gain: 0.5,
        }
    }

    fn process(&mut self, sample: f32) -> f32 {
        let read_pos = (self.write_pos + self.buffer.len() - self.delay) % self.buffer.len();
        let delayed = self.buffer[read_pos];
        let out = -self.gain * sample + delayed + self.gain * self.buffer[self.write_pos];
        self.buffer[self.write_pos] = sample + self.gain * delayed;
        self.write_pos = (self.write_pos + 1) % self.buffer.len();
        out
    }

    fn set_delay(&mut self, samples: usize) {
        self.delay = samples.min(self.buffer.len().saturating_sub(1));
    }
}

/// Base delay lengths for comb filters at 44100 Hz (scaled by sample rate).
const COMB_DELAYS: [usize; 4] = [1557, 1617, 1491, 1422];

/// Base delay lengths for allpass filters.
const ALLPASS_DELAYS: [usize; 2] = [225, 341];

pub struct Reverb {
    /// Room size (0..1) — scales delay times
    size: f32,
    /// Damping (0..1) — high frequency absorption
    damping: f32,
    /// Wet/dry mix (0..1)
    mix: f32,
    sample_rate: f32,
    combs: Vec<CombFilter>,
    allpasses: Vec<AllpassFilter>,
    params_dirty: bool,
}

impl Default for Reverb {
    fn default() -> Self {
        Self::new()
    }
}

impl Reverb {
    pub fn new() -> Self {
        Self {
            size: 0.5,
            damping: 0.5,
            mix: 0.3,
            sample_rate: 48000.0,
            combs: Vec::new(),
            allpasses: Vec::new(),
            params_dirty: true,
        }
    }

    fn rebuild(&mut self) {
        let sr_ratio = (self.sample_rate / 44100.0).clamp(0.5, 2.0);
        let size_scale = 0.5 + self.size * 1.5; // 0.5..2.0

        self.combs.clear();
        for &base_delay in &COMB_DELAYS {
            let delay = (base_delay as f32 * sr_ratio * size_scale).round() as usize;
            let mut comb = CombFilter::new(delay + 4);
            comb.set_delay(delay);
            comb.feedback = 0.84;
            comb.damping = self.damping;
            self.combs.push(comb);
        }

        self.allpasses.clear();
        for &base_delay in &ALLPASS_DELAYS {
            let delay = (base_delay as f32 * sr_ratio * size_scale).round() as usize;
            let mut ap = AllpassFilter::new(delay + 4);
            ap.set_delay(delay);
            ap.gain = 0.5;
            self.allpasses.push(ap);
        }
    }
}

impl Plugin for Reverb {
    fn name(&self) -> &str {
        "reverb"
    }

    fn init(&mut self, sample_rate: f64) -> anyhow::Result<()> {
        self.sample_rate = sample_rate as f32;
        self.params_dirty = true;
        Ok(())
    }

    fn process(&mut self, input: &[f32], output: &mut [f32]) -> anyhow::Result<()> {
        if self.params_dirty {
            self.rebuild();
            self.params_dirty = false;
        }

        let mix = self.mix;

        for (i, sample) in input.iter().enumerate() {
            let mut wet = 0.0_f32;

            // Parallel comb filters
            for comb in &mut self.combs {
                wet += comb.process(*sample);
            }
            wet /= self.combs.len() as f32;

            // Series allpass filters
            for ap in &mut self.allpasses {
                wet = ap.process(wet);
            }

            // Wet/dry mix
            output[i] = sample * (1.0 - mix) + wet * mix;
        }
        Ok(())
    }

    fn get_parameter(&self, id: &str) -> Option<f32> {
        match id {
            "size" => Some(self.size),
            "damping" => Some(self.damping),
            "mix" => Some(self.mix),
            _ => None,
        }
    }

    fn set_parameter(&mut self, id: &str, value: f32) {
        match id {
            "size" => {
                self.size = value.clamp(0.0, 1.0);
                self.params_dirty = true;
            }
            "damping" => {
                self.damping = value.clamp(0.0, 1.0);
                self.params_dirty = true;
            }
            "mix" => self.mix = value.clamp(0.0, 1.0),
            _ => {}
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Plugin Registry ── manages a chain of plugins
// ═══════════════════════════════════════════════════════════════════════════════

/// Registry for managing a chain of plugins.
/// Per-slot pseudo-parameter controlling the wet/dry mix (`"plugin/wet_dry"`).
pub const WET_DRY_PARAM: &str = "wet_dry";

pub struct PluginRegistry {
    plugins: Vec<Box<dyn Plugin>>,
    parameters: HashMap<String, f32>,
    /// RMS audio levels per plugin (updated each `process_all` cycle).
    /// Used by the frontend for real-time VU-meter visualisation.
    /// Length matches the number of plugins.
    levels: Vec<f32>,
    /// Pre-allocated working buffers to avoid allocation in the real-time path.
    /// We use double-buffering: buf_a and buf_b are swapped each plugin iteration.
    buf_a: Vec<f32>,
    buf_b: Vec<f32>,
    /// Whether each plugin is enabled (true) or bypassed (false).
    /// Length matches the number of plugins.
    enabled: Vec<bool>,
    /// Wet/dry mix per plugin (1.0 = fully processed, 0.0 = dry).
    /// Length matches the number of plugins.
    wet: Vec<f32>,
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
            parameters: HashMap::new(),
            levels: Vec::new(),
            buf_a: Vec::new(),
            buf_b: Vec::new(),
            enabled: Vec::new(),
            wet: Vec::new(),
        }
    }

    pub fn add_plugin(&mut self, plugin: Box<dyn Plugin>) {
        self.plugins.push(plugin);
        self.levels.push(0.0);
        self.enabled.push(true);
        self.wet.push(1.0);
    }

    /// Remove all plugins.
    pub fn clear(&mut self) {
        self.plugins.clear();
        self.parameters.clear();
        self.levels.clear();
        self.enabled.clear();
        self.wet.clear();
    }

    /// Number of plugins in the chain.
    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// Initialize all plugins at the given sample rate.
    pub fn init_all(&mut self, sample_rate: f64) -> anyhow::Result<()> {
        for plugin in &mut self.plugins {
            plugin.init(sample_rate)?;
        }
        // Pre-allocate working buffers for real-time processing
        // (max buffer size is typically 2048 samples)
        let max_buf = 4096;
        self.buf_a.resize(max_buf, 0.0);
        self.buf_b.resize(max_buf, 0.0);
        Ok(())
    }

    /// Process audio through the full plugin chain.
    ///
    /// After each plugin processes, its output RMS level is stored in
    /// `self.levels` for front-end VU-meter visualisation.  The levels
    /// are updated every audio cycle (~5 ms at 256 / 48 kHz).
    ///
    /// Uses pre-allocated double buffers to avoid heap allocation in the
    /// real-time audio callback.
    pub fn process_all(&mut self, input: &[f32], output: &mut [f32]) -> anyhow::Result<()> {
        let n = input.len().min(output.len());
        if self.plugins.is_empty() {
            output[..n].copy_from_slice(&input[..n]);
            return Ok(());
        }
        // Never grow buffers here: that would allocate on the audio thread.
        // Blocks larger than the pre-allocated size are processed in chunks.
        let chunk = self.buf_a.len().min(self.buf_b.len());
        if chunk == 0 {
            // `init_all` was never called; pass audio through untouched.
            output[..n].copy_from_slice(&input[..n]);
            return Ok(());
        }
        let mut start = 0;
        while start < n {
            let end = (start + chunk).min(n);
            self.process_chunk(&input[start..end], &mut output[start..end])?;
            start = end;
        }
        Ok(())
    }

    fn process_chunk(&mut self, input: &[f32], output: &mut [f32]) -> anyhow::Result<()> {
        let n = input.len();
        self.buf_a[..n].copy_from_slice(input);

        // read_from_a = true means read from buf_a, write to buf_b.
        let mut read_from_a = true;
        for (i, plugin) in self.plugins.iter_mut().enumerate() {
            let (src, dst) = if read_from_a {
                (&self.buf_a[..n], &mut self.buf_b[..n])
            } else {
                (&self.buf_b[..n], &mut self.buf_a[..n])
            };
            let level = if self.enabled.get(i).copied().unwrap_or(true) {
                plugin.process(src, dst)?;
                let w = self.wet.get(i).copied().unwrap_or(1.0);
                if w < 1.0 {
                    let d = 1.0 - w;
                    for (o, &x) in dst.iter_mut().zip(src.iter()) {
                        *o = *o * w + x * d;
                    }
                }
                compute_rms(dst)
            } else {
                dst.copy_from_slice(src);
                0.0
            };
            if let Some(l) = self.levels.get_mut(i) {
                *l = level;
            }
            read_from_a = !read_from_a;
        }
        let result = if read_from_a {
            &self.buf_a[..n]
        } else {
            &self.buf_b[..n]
        };
        output.copy_from_slice(result);
        Ok(())
    }

    /// Real-time-safe parameter update, used by the audio callback.
    ///
    /// Unlike [`set_parameter`](Self::set_parameter) it never touches the
    /// `parameters` cache (which would allocate a `String` key) and never
    /// formats strings. `"plugin/param"` ids target one plugin; `"plugin/enabled"`
    /// toggles bypass; bare ids go to every plugin that exposes them.
    pub fn set_parameter_rt(&mut self, id: &str, value: f32) {
        if let Some((plugin_name, param_id)) = id.split_once('/') {
            for (i, plugin) in self.plugins.iter_mut().enumerate() {
                if plugin.name() != plugin_name {
                    continue;
                }
                if param_id == "enabled" {
                    if let Some(e) = self.enabled.get_mut(i) {
                        *e = value > 0.5;
                    }
                } else if param_id == WET_DRY_PARAM {
                    if let Some(w) = self.wet.get_mut(i) {
                        *w = value.clamp(0.0, 1.0);
                    }
                } else if plugin.get_parameter(param_id).is_some() {
                    plugin.set_parameter(param_id, value);
                }
                return;
            }
            return;
        }
        for plugin in &mut self.plugins {
            if plugin.get_parameter(id).is_some() {
                plugin.set_parameter(id, value);
            }
        }
    }

    /// Reorder plugins to follow `order` (plugin names). Plugins not named
    /// keep their relative order and are appended after the named ones,
    /// except that `input` stays first and `output` stays last.
    ///
    /// Enabled flags, wet/dry and levels move with their plugin. This is a
    /// control-thread operation (it allocates small index vectors); it holds
    /// no plugin state copies and runs in O(n log n) for n plugins.
    /// Returns `true` if anything moved.
    pub fn reorder<S: AsRef<str>>(&mut self, order: &[S]) -> bool {
        let n = self.plugins.len();
        let rank = |name: &str| -> usize {
            if name == "input" {
                return 0;
            }
            if name == "output" {
                return usize::MAX;
            }
            order
                .iter()
                .position(|o| o.as_ref() == name)
                .map(|p| p + 1)
                .unwrap_or(usize::MAX - 1)
        };
        let mut idx: Vec<usize> = (0..n).collect();
        // Stable sort keeps unnamed plugins in their current relative order.
        idx.sort_by_key(|&i| rank(self.plugins[i].name()));
        if idx.iter().enumerate().all(|(a, &b)| a == b) {
            return false;
        }
        let mut plugins: Vec<Option<Box<dyn Plugin>>> = self.plugins.drain(..).map(Some).collect();
        let (enabled, wet, levels) = (self.enabled.clone(), self.wet.clone(), self.levels.clone());
        for (k, &i) in idx.iter().enumerate() {
            if let Some(p) = plugins[i].take() {
                self.plugins.push(p);
            }
            self.enabled[k] = enabled[i];
            self.wet[k] = wet[i];
            self.levels[k] = levels[i];
        }
        true
    }

    /// Plugin names in processing order.
    pub fn plugin_names(&self) -> Vec<&str> {
        self.plugins.iter().map(|p| p.name()).collect()
    }

    /// Return the current per-plugin RMS audio levels.
    ///
    /// These are updated every `process_all` cycle (~5 ms).  Each entry
    /// is in the 0..1 range.  The frontend polls this at ~20 Hz for
    /// real-time VU-meter visualisation.
    pub fn audio_levels(&self) -> Vec<f32> {
        self.levels.clone()
    }

    /// Set a parameter on a specific plugin by name.
    ///
    /// Returns `true` if the plugin was found and the parameter set.
    pub fn set_parameter_on_plugin(&mut self, plugin_name: &str, id: &str, value: f32) -> bool {
        self.parameters
            .insert(format!("{}.{}", plugin_name, id), value);
        for plugin in &mut self.plugins {
            if plugin.name() == plugin_name && plugin.get_parameter(id).is_some() {
                plugin.set_parameter(id, value);
                return true;
            }
        }
        false
    }

    /// Enable or disable a plugin by name.
    ///
    /// Returns `true` if the plugin was found.
    pub fn set_plugin_enabled(&mut self, plugin_name: &str, enabled: bool) -> bool {
        for (i, plugin) in self.plugins.iter().enumerate() {
            if plugin.name() == plugin_name {
                self.enabled[i] = enabled;
                self.parameters.insert(
                    format!("{}.__enabled", plugin_name),
                    if enabled { 1.0 } else { 0.0 },
                );
                return true;
            }
        }
        false
    }

    pub fn get_parameter(&self, id: &str) -> Option<f32> {
        // Composite "plugin/param" ids read the live plugin state first: the
        // audio thread updates plugins via `set_parameter_rt`, which does not
        // touch the `parameters` cache, so the cache can be stale for them.
        if let Some((plugin_name, param_id)) = id.split_once('/') {
            if let Some(i) = self.plugins.iter().position(|p| p.name() == plugin_name) {
                if param_id == "enabled" {
                    return self.enabled.get(i).map(|&e| if e { 1.0 } else { 0.0 });
                }
                if param_id == WET_DRY_PARAM {
                    return self.wet.get(i).copied();
                }
                if let Some(v) = self.plugins[i].get_parameter(param_id) {
                    return Some(v);
                }
            }
        }
        // Check local override first
        if let Some(&val) = self.parameters.get(id) {
            return Some(val);
        }
        if let Some((plugin_name, param_id)) = id.split_once('/') {
            let i = self.plugins.iter().position(|p| p.name() == plugin_name)?;
            if param_id == "enabled" {
                return self.enabled.get(i).map(|&e| if e { 1.0 } else { 0.0 });
            }
            if param_id == WET_DRY_PARAM {
                return self.wet.get(i).copied();
            }
            return self.plugins[i].get_parameter(param_id);
        }
        // Fall through to plugins
        for plugin in &self.plugins {
            if let Some(val) = plugin.get_parameter(id) {
                return Some(val);
            }
        }
        None
    }

    pub fn set_parameter(&mut self, id: &str, value: f32) {
        self.parameters.insert(id.to_string(), value);
        // Composite "plugin/param" ids (pushed by preset/scene loads through
        // the SPSC queue) route to one specific plugin instead of broadcasting.
        if let Some((plugin_name, param_id)) = id.split_once('/') {
            if param_id == "enabled" {
                self.set_plugin_enabled(plugin_name, value > 0.5);
            } else if param_id == WET_DRY_PARAM {
                self.set_parameter_rt(id, value);
            } else {
                self.set_parameter_on_plugin(plugin_name, param_id, value);
            }
            return;
        }
        // Only set on plugins that actually expose this parameter,
        // avoiding collisions when multiple plugins share parameter names.
        for plugin in &mut self.plugins {
            if plugin.get_parameter(id).is_some() {
                plugin.set_parameter(id, value);
            }
        }
    }

    /// Update only the parameter value HashMap without iterating plugins.
    ///
    /// Used by the main thread to keep `get_parameter` consistent while
    /// the actual plugin update goes through the lock-free SPSC queue.
    /// The audio callback drains the queue and calls `set_parameter`
    /// (which includes the plugin iteration) before each process cycle.
    pub fn set_parameter_value(&mut self, id: &str, value: f32) {
        self.parameters.insert(id.to_string(), value);
    }

    /// Build a default signal chain: Input → Tuner → Boost → Amp → Cab → Delay → Reverb → Output.
    ///
    /// The Tuner sits right after Input so it always analyzes the raw guitar
    /// signal (it passes audio through transparently unless muted).
    pub fn build_default_chain(&mut self) {
        use crate::effects::{
            chorus, flanger, Compressor, NoiseGate, Overdrive, ParametricEq, Phaser, Tremolo,
        };
        self.clear();
        self.add_plugin(Box::new(Input));
        self.add_plugin(Box::new(Tuner::new()));
        self.add_plugin_bypassed(Box::new(NoiseGate::new()));
        self.add_plugin_bypassed(Box::new(Compressor::new()));
        self.add_plugin_bypassed(Box::new(Overdrive::new()));
        self.add_plugin(Box::new(Boost::new()));
        self.add_plugin(Box::new(Amp::new()));
        self.add_plugin(Box::new(Cab::new()));
        self.add_plugin_bypassed(Box::new(ParametricEq::new()));
        self.add_plugin_bypassed(Box::new(chorus()));
        self.add_plugin_bypassed(Box::new(Phaser::new()));
        self.add_plugin_bypassed(Box::new(flanger()));
        self.add_plugin_bypassed(Box::new(Tremolo::new()));
        self.add_plugin(Box::new(Delay::new()));
        self.add_plugin(Box::new(Reverb::new()));
        self.add_plugin(Box::new(Output::new()));
    }

    /// Add a plugin that starts bypassed (e.g. optional pedals).
    pub fn add_plugin_bypassed(&mut self, plugin: Box<dyn Plugin>) {
        self.add_plugin(plugin);
        if let Some(e) = self.enabled.last_mut() {
            *e = false;
        }
    }

    /// Build a bass signal chain: Boost → BassAmp → Cab → Delay → Reverb.
    pub fn build_bass_chain(&mut self) {
        self.clear();
        self.add_plugin(Box::new(Boost::new()));
        self.add_plugin(Box::new(BassAmp::new()));
        self.add_plugin(Box::new(Cab::new()));
        self.add_plugin(Box::new(Delay::new()));
        self.add_plugin(Box::new(Reverb::new()));
    }

    /// Build a practice chain with tuner and metronome.
    pub fn build_practice_chain(&mut self) {
        self.clear();
        self.add_plugin(Box::new(Tuner::new()));
        self.add_plugin(Box::new(Metronome::new()));
        self.add_plugin(Box::new(Amp::new()));
        self.add_plugin(Box::new(Cab::new()));
    }

    /// Build a looper chain: Amp → Cab → Looper.
    pub fn build_looper_chain(&mut self) {
        self.clear();
        self.add_plugin(Box::new(Amp::new()));
        self.add_plugin(Box::new(Cab::new()));
        self.add_plugin(Box::new(Looper::new()));
    }

    /// Add a tuner plugin to the chain.
    pub fn add_tuner(&mut self) {
        self.add_plugin(Box::new(Tuner::new()));
    }

    /// Add a metronome plugin to the chain.
    pub fn add_metronome(&mut self) {
        self.add_plugin(Box::new(Metronome::new()));
    }

    /// Add a looper plugin to the chain.
    pub fn add_looper(&mut self) {
        self.add_plugin(Box::new(Looper::new()));
    }

    /// Add a bass amp plugin to the chain.
    pub fn add_bass_amp(&mut self) {
        self.add_plugin(Box::new(BassAmp::new()));
    }

    /// Find the Tuner plugin and get its current pitch info.
    pub fn tuner_info(&self) -> Option<(f32, String, f32, f32)> {
        for plugin in &self.plugins {
            if let Some(tuner) = plugin.as_any().downcast_ref::<Tuner>() {
                return Some((
                    tuner.frequency(),
                    tuner.note(),
                    tuner.cents(),
                    tuner.detection_confidence(),
                ));
            }
        }
        None
    }

    /// Find the Metronome plugin and get its current state.
    pub fn metronome_state(&self) -> Option<(f32, u8, bool)> {
        for plugin in &self.plugins {
            if let Some(metro) = plugin.as_any().downcast_ref::<Metronome>() {
                // Access internal fields via parameter getters
                let bpm = metro.get_parameter("bpm").unwrap_or(120.0);
                let beats = (metro.get_parameter("beats_per_bar").unwrap_or(4.0) as u8).max(1);
                let running = metro.get_parameter("running").unwrap_or(0.0) > 0.5;
                return Some((bpm, beats, running));
            }
        }
        None
    }

    /// Find the Looper plugin and get its current state.
    pub fn looper_state(&self) -> Option<(String, f32, bool)> {
        for plugin in &self.plugins {
            if let Some(looper) = plugin.as_any().downcast_ref::<Looper>() {
                let mode_str = match looper.get_parameter("mode").unwrap_or(0.0) as i32 {
                    0 => "idle",
                    1 => "record",
                    2 => "overdub",
                    3 => "play",
                    4 => "stop",
                    _ => "unknown",
                };
                let time = looper.get_parameter("loop_time").unwrap_or(0.0);
                let has_loop = looper.get_parameter("has_loop").unwrap_or(0.0) > 0.5;
                return Some((mode_str.to_string(), time, has_loop));
            }
        }
        None
    }

    /// Trigger a mode change on the Looper plugin.
    pub fn trigger_looper_mode(&mut self, mode_value: f32) -> bool {
        for plugin in &mut self.plugins {
            if let Some(looper) = plugin.as_any_mut().downcast_mut::<Looper>() {
                looper.set_parameter("mode", mode_value);
                return true;
            }
        }
        false
    }

    /// Undo the last overdub on the Looper plugin.
    pub fn looper_undo(&mut self) -> bool {
        for plugin in &mut self.plugins {
            if let Some(looper) = plugin.as_any_mut().downcast_mut::<Looper>() {
                looper.undo();
                return true;
            }
        }
        false
    }

    /// Clear the Looper plugin's buffer.
    pub fn looper_clear(&mut self) -> bool {
        for plugin in &mut self.plugins {
            if let Some(looper) = plugin.as_any_mut().downcast_mut::<Looper>() {
                looper.clear();
                return true;
            }
        }
        false
    }

    /// Get a reference to a plugin by index (for direct access).
    pub fn get_plugin(&self, index: usize) -> Option<&dyn Plugin> {
        self.plugins.get(index).map(|p| p.as_ref())
    }

    /// Get a mutable reference to a plugin by index.
    pub fn get_plugin_mut(&mut self, index: usize) -> Option<&mut dyn Plugin> {
        // Bypass lifetime issue by using an index-based approach
        if index < self.plugins.len() {
            Some(self.plugins[index].as_mut())
        } else {
            None
        }
    }

    /// Find the Cab plugin in the chain and load an impulse response into it.
    ///
    /// Returns `true` if the Cab was found and IR loaded, `false` otherwise.
    pub fn load_ir_to_cab(&mut self, path: String, ir_data: Vec<f32>, ir_sample_rate: f32) -> bool {
        for plugin in &mut self.plugins {
            if let Some(cab) = plugin.as_any_mut().downcast_mut::<Cab>() {
                cab.load_ir(path, ir_data, ir_sample_rate);
                return true;
            }
        }
        false
    }

    /// Swap a prepared convolver into the Cab. `Err(conv)` if there is no Cab.
    /// On success returns the old convolver (drop it outside the engine lock).
    pub fn install_cab_ir(
        &mut self, path: String, conv: Convolver,
    ) -> Result<Option<Convolver>, Convolver> {
        for plugin in &mut self.plugins {
            if let Some(cab) = plugin.as_any_mut().downcast_mut::<Cab>() {
                return Ok(cab.install_ir(path, conv));
            }
        }
        Err(conv)
    }

    /// Get IR info from the Cab plugin, if loaded.
    pub fn cab_ir_info(&self) -> Option<IrInfo> {
        for plugin in &self.plugins {
            if let Some(cab) = plugin.as_any().downcast_ref::<Cab>() {
                return cab.ir_info();
            }
        }
        None
    }

    /// Clear the IR from the Cab plugin.
    pub fn clear_cab_ir(&mut self) -> bool {
        for plugin in &mut self.plugins {
            if let Some(cab) = plugin.as_any_mut().downcast_mut::<Cab>() {
                cab.clear_ir();
                return true;
            }
        }
        false
    }

    /// Find the Nam plugin in the chain and load a neural model into it.
    pub fn load_nam_to_plugin(&mut self, path: String, neural_model: NeuralModel) -> bool {
        for plugin in &mut self.plugins {
            if let Some(nam) = plugin.as_any_mut().downcast_mut::<Nam>() {
                nam.load_nam_model(path, neural_model);
                return true;
            }
        }
        false
    }

    /// Get NAM model info from the Nam plugin, if loaded.
    pub fn nam_model_info(&self) -> Option<NamModelInfo> {
        for plugin in &self.plugins {
            if let Some(nam) = plugin.as_any().downcast_ref::<Nam>() {
                return nam.nam_info();
            }
        }
        None
    }

    /// Clear the NAM model from the Nam plugin.
    pub fn clear_nam_model(&mut self) -> bool {
        for plugin in &mut self.plugins {
            if let Some(nam) = plugin.as_any_mut().downcast_mut::<Nam>() {
                nam.clear_nam_model();
                return true;
            }
        }
        false
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {

    #[test]
    fn delay_line_read_never_indexes_past_end() {
        // f32 rem_euclid of a tiny negative offset rounds to exactly
        // `capacity`; this used to panic on the audio thread.
        let mut d = DelayLine::new(96_000);
        d.write(1.0);
        d.write_pos = 0;
        for delay in [1e-5f32, 1e-3, 0.5, 95_999.0] {
            assert!(d.read(delay).is_finite());
        }
    }

    #[test]
    fn process_all_handles_blocks_larger_than_buffers() {
        let mut r = PluginRegistry::new();
        r.build_default_chain();
        r.init_all(48_000.0).unwrap();
        let input = vec![0.1f32; 10_000];
        let mut output = vec![0.0f32; 10_000];
        r.process_all(&input, &mut output).unwrap();
        assert!(output.iter().all(|s| s.is_finite()));
        assert!(output[9_999] != 0.0 || output.iter().any(|&s| s != 0.0));
    }

    #[test]
    fn composite_get_reflects_rt_updates_after_cached_set() {
        let mut r = PluginRegistry::new();
        r.build_default_chain();
        r.init_all(48_000.0).unwrap();
        r.set_parameter("amp/drive", 0.1); // control thread, fills the cache
        r.set_parameter_rt("amp/drive", 0.9); // audio thread, no cache
        assert_eq!(r.get_parameter("amp/drive"), Some(0.9));
        r.set_parameter("delay/enabled", 1.0);
        r.set_parameter_rt("delay/enabled", 0.0);
        assert_eq!(r.get_parameter("delay/enabled"), Some(0.0));
    }

    #[test]
    fn reorder_moves_flags_with_plugins() {
        let mut r = PluginRegistry::new();
        r.build_default_chain();
        r.init_all(48_000.0).unwrap();
        r.set_parameter_rt("delay/enabled", 0.0);
        r.set_parameter_rt("delay/wet_dry", 0.3);
        assert!(r.reorder(&["tuner", "delay", "boost", "amp", "cab", "reverb"]));
        assert_eq!(
            r.plugin_names(),
            [
                "input", "tuner", "delay", "boost", "amp", "cab", "reverb", "gate", "comp",
                "drive", "eq", "chorus", "phaser", "flanger", "tremolo", "output"
            ]
        );
        assert_eq!(r.get_parameter("delay/enabled"), Some(0.0));
        assert_eq!(r.get_parameter("delay/wet_dry"), Some(0.3));
        assert_eq!(r.get_parameter("boost/enabled"), Some(1.0));
        // Same order again: nothing moves.
        assert!(!r.reorder(&["tuner", "delay", "boost", "amp", "cab", "reverb"]));
        // input/output are pinned even if named elsewhere.
        r.reorder(&["output", "reverb", "input"]);
        let names = r.plugin_names();
        assert_eq!(names.first(), Some(&"input"));
        assert_eq!(names.last(), Some(&"output"));
        assert_eq!(names[1], "reverb");
    }

    #[test]
    fn reorder_changes_the_sound() {
        // Boost before vs after amp must differ (non-linear amp).
        let render = |order: &[&str]| {
            let mut r = PluginRegistry::new();
            r.build_default_chain();
            r.init_all(48_000.0).unwrap();
            r.set_parameter_rt("delay/enabled", 0.0);
            r.set_parameter_rt("reverb/enabled", 0.0);
            r.reorder(order);
            let input: Vec<f32> = (0..1024).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
            let mut out = vec![0.0; 1024];
            r.process_all(&input, &mut out).unwrap();
            out
        };
        let a = render(&["tuner", "boost", "amp", "cab"]);
        let b = render(&["tuner", "amp", "boost", "cab"]);
        let diff: f32 = a.iter().zip(&b).map(|(x, y)| (x - y).abs()).sum();
        assert!(diff > 1e-3, "reordering had no audible effect ({diff})");
    }

    #[test]
    fn wet_dry_mixes_processed_and_dry() {
        let mut r = PluginRegistry::new();
        r.add_plugin(Box::new(Boost::new()));
        r.init_all(48_000.0).unwrap();
        let input = vec![0.1f32; 256];
        let mut wet = vec![0.0; 256];
        r.process_all(&input, &mut wet).unwrap();
        assert!((wet[100] - 0.1).abs() > 1e-4, "boost should change level");

        r.set_parameter_rt("boost/wet_dry", 0.0);
        let mut dry = vec![0.0; 256];
        r.process_all(&input, &mut dry).unwrap();
        assert!(dry.iter().all(|&x| (x - 0.1).abs() < 1e-6));

        r.set_parameter_rt("boost/wet_dry", 0.5);
        let mut half = vec![0.0; 256];
        r.process_all(&input, &mut half).unwrap();
        let expect = 0.5 * wet[100] + 0.5 * 0.1;
        assert!(
            (half[100] - expect).abs() < 1e-4,
            "{} vs {}",
            half[100],
            expect
        );
    }

    #[test]
    fn set_parameter_rt_routes_composite_ids() {
        let mut r = PluginRegistry::new();
        r.build_default_chain();
        r.init_all(48_000.0).unwrap();
        r.set_parameter_rt("boost/enabled", 0.0);
        assert_eq!(r.get_parameter("boost/enabled"), Some(0.0));
        r.set_parameter_rt("amp/drive", 0.25);
        assert_eq!(r.get_parameter("amp/drive"), Some(0.25));
    }

    use super::*;

    fn test_buffer(size: usize) -> Vec<f32> {
        (0..size)
            .map(|i| (i as f32 / size as f32) * 0.5 - 0.25)
            .collect()
    }

    #[test]
    fn test_passthrough() {
        let mut p = Passthrough;
        let input = test_buffer(64);
        let mut output = vec![0.0; 64];
        p.process(&input, &mut output).unwrap();
        assert_eq!(input, output);
    }

    #[test]
    fn test_boost() {
        let mut boost = Boost::new();
        boost.init(48000.0).unwrap();
        let input = test_buffer(64);
        let mut output = vec![0.0; 64];
        boost.process(&input, &mut output).unwrap();
        // Boost default gain is 1.5
        for i in 0..64 {
            assert!((output[i] - input[i] * 1.5).abs() < 1e-6);
        }
    }

    #[test]
    fn test_boost_parameter() {
        let mut boost = Boost::new();
        boost.set_parameter("gain", 0.5); // 0.5 → gain = 1.0
        let input = test_buffer(64);
        let mut output = vec![0.0; 64];
        boost.process(&input, &mut output).unwrap();
        for i in 0..64 {
            assert!((output[i] - input[i]).abs() < 1e-6);
        }
    }

    #[test]
    fn test_amp_processes() {
        let mut amp = Amp::new();
        amp.init(48000.0).unwrap();
        let input = test_buffer(64);
        let mut output = vec![0.0; 64];
        amp.process(&input, &mut output).unwrap();
        // Output should not be all zeros or identical to input
        assert!(output.iter().any(|&x| x != 0.0));
    }

    #[test]
    fn test_amp_parameters() {
        let mut amp = Amp::new();
        amp.set_parameter("gain", 1.0);
        assert!((amp.get_parameter("gain").unwrap() - 1.0).abs() < 1e-6);
        amp.set_parameter("bass", 0.7);
        assert!((amp.get_parameter("bass").unwrap() - 0.7).abs() < 1e-6);
        amp.set_parameter("master", 0.5);
        assert!((amp.get_parameter("master").unwrap() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn test_cab_processes() {
        let mut cab = Cab::new();
        cab.init(48000.0).unwrap();
        let input = test_buffer(64);
        let mut output = vec![0.0; 64];
        cab.process(&input, &mut output).unwrap();
        assert!(output.iter().any(|&x| x != 0.0));
    }

    #[test]
    fn test_delay_processes() {
        let mut delay = Delay::new();
        delay.init(48000.0).unwrap();
        let input = test_buffer(64);
        let mut output = vec![0.0; 64];
        delay.process(&input, &mut output).unwrap();
        assert!(output.iter().any(|&x| x != 0.0));
    }

    #[test]
    fn test_delay_line() {
        let mut line = DelayLine::new(10);
        line.write(1.0);
        line.write(2.0);
        line.write(3.0);
        // Read 2 samples back → should be sample from (write_pos - 2) = 1 → buffer[1] = 2.0
        let val = line.read(2.0);
        assert!((val - 2.0).abs() < 1e-6);
    }

    #[test]
    fn test_reverb_processes() {
        let mut reverb = Reverb::new();
        reverb.init(44100.0).unwrap();
        let input = test_buffer(256);
        let mut output = vec![0.0; 256];
        reverb.process(&input, &mut output).unwrap();
        assert!(output.iter().any(|&x| x != 0.0));
    }

    #[test]
    fn test_reverb_parameters() {
        let mut reverb = Reverb::new();
        reverb.set_parameter("size", 0.8);
        assert!((reverb.get_parameter("size").unwrap() - 0.8).abs() < 1e-6);
        reverb.set_parameter("mix", 0.5);
        assert!((reverb.get_parameter("mix").unwrap() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn test_registry_default_chain() {
        let mut registry = PluginRegistry::new();
        registry.build_default_chain();
        assert_eq!(registry.len(), 16);
        // Optional pedals start bypassed so the default sound is unchanged.
        for id in [
            "gate", "comp", "drive", "eq", "chorus", "phaser", "flanger", "tremolo",
        ] {
            assert_eq!(
                registry.get_parameter(&format!("{id}/enabled")),
                Some(0.0),
                "{id}"
            );
        }
        for id in ["tuner", "boost", "amp", "cab", "delay", "reverb"] {
            assert_eq!(
                registry.get_parameter(&format!("{id}/enabled")),
                Some(1.0),
                "{id}"
            );
        }
    }

    #[test]
    fn test_composite_param_id_routing() {
        let mut registry = PluginRegistry::new();
        registry.build_default_chain();
        registry.init_all(48000.0).unwrap();

        registry.set_parameter("boost/gain", 0.25);
        assert_eq!(registry.get_parameter("boost.gain"), Some(0.25));

        registry.set_parameter("boost/enabled", 0.0);
        assert_eq!(registry.get_parameter("boost.__enabled"), Some(0.0));
    }

    #[test]
    fn test_registry_process_chain() {
        let mut registry = PluginRegistry::new();
        registry.build_default_chain();
        registry.init_all(48000.0).unwrap();
        let input = test_buffer(128);
        let mut output = vec![0.0; 128];
        registry.process_all(&input, &mut output).unwrap();
        assert!(output.iter().any(|&x| x != 0.0));
        assert_ne!(input, output);
    }

    #[test]
    fn test_biquad_lowpass() {
        let mut lp = BiquadFilter::design_lowpass(48000.0, 500.0, 0.707);
        // DC should pass through (IIR needs ~5 time constants to settle; at 500Hz ~15 samples each)
        for _ in 0..200 {
            lp.process(1.0);
        }
        let dc_out = lp.process(1.0);
        assert!((dc_out - 1.0).abs() < 1e-3);
    }

    #[test]
    fn test_biquad_highpass() {
        let mut hp = BiquadFilter::design_highpass(48000.0, 100.0, 0.707);
        // DC should be blocked (highpass time constant ~1.6ms, need ~500 samples to settle)
        for _ in 0..500 {
            hp.process(1.0);
        }
        let steady = hp.process(1.0);
        assert!(steady.abs() < 0.1);
    }

    #[test]
    fn test_waveshape_asymmetric() {
        let result = waveshape_asymmetric(0.5, 5.0);
        assert!(result > 0.0);
        assert!(result < 1.0);
    }
}
