//! Workstation effects: noise gate, compressor, 3-band parametric EQ,
//! overdrive, chorus, flanger, phaser and tremolo.
//!
//! All parameters are normalised to `0..=1` like the rest of the chain and
//! mapped to musical ranges internally. Every buffer is allocated in `init`
//! (control thread); `process` and `set_parameter` never allocate.

use crate::plugins::Plugin;
use std::any::Any;
use std::f32::consts::PI;

#[inline]
fn db_to_lin(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

#[inline]
fn lin_to_db(x: f32) -> f32 {
    20.0 * x.max(1e-9).log10()
}

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t.clamp(0.0, 1.0)
}

/// Exponential map of `t` in 0..1 to `lo..hi` (for frequencies and times).
#[inline]
fn exp_map(lo: f32, hi: f32, t: f32) -> f32 {
    lo * (hi / lo).powf(t.clamp(0.0, 1.0))
}

/// One-pole smoothing coefficient for a time constant in milliseconds.
#[inline]
fn coef_ms(ms: f32, sr: f32) -> f32 {
    (-1.0 / (ms.max(0.01) * 0.001 * sr)).exp()
}

macro_rules! any_impl {
    () => {
        fn as_any(&self) -> &dyn Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }
    };
}

// ── Biquad (RBJ cookbook) ────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Default for Biquad {
    fn default() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            z1: 0.0,
            z2: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BiquadKind {
    LowShelf,
    Peak,
    HighShelf,
}

impl Biquad {
    pub fn set(&mut self, kind: BiquadKind, sr: f32, freq: f32, q: f32, gain_db: f32) {
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * (freq / sr).clamp(1e-5, 0.49);
        let (sw, cw) = w0.sin_cos();
        let alpha = sw / (2.0 * q.max(0.05));
        let (b0, b1, b2, a0, a1, a2) = match kind {
            BiquadKind::Peak => (
                1.0 + alpha * a,
                -2.0 * cw,
                1.0 - alpha * a,
                1.0 + alpha / a,
                -2.0 * cw,
                1.0 - alpha / a,
            ),
            BiquadKind::LowShelf => {
                let s = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) - (a - 1.0) * cw + s),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * cw),
                    a * ((a + 1.0) - (a - 1.0) * cw - s),
                    (a + 1.0) + (a - 1.0) * cw + s,
                    -2.0 * ((a - 1.0) + (a + 1.0) * cw),
                    (a + 1.0) + (a - 1.0) * cw - s,
                )
            }
            BiquadKind::HighShelf => {
                let s = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) + (a - 1.0) * cw + s),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cw),
                    a * ((a + 1.0) + (a - 1.0) * cw - s),
                    (a + 1.0) - (a - 1.0) * cw + s,
                    2.0 * ((a - 1.0) - (a + 1.0) * cw),
                    (a + 1.0) - (a - 1.0) * cw - s,
                )
            }
        };
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
    }

    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        // Transposed direct form II.
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

// ── Modulated delay line ─────────────────────────────────────────────────────

/// Circular delay with fractional (linear-interpolated) reads.
#[derive(Default)]
struct ModDelay {
    buf: Vec<f32>,
    pos: usize,
}

impl ModDelay {
    fn allocate(&mut self, len: usize) {
        self.buf = vec![0.0; len.max(4)];
        self.pos = 0;
    }

    #[inline]
    fn push(&mut self, x: f32) {
        if self.buf.is_empty() {
            return;
        }
        self.buf[self.pos] = x;
        self.pos = (self.pos + 1) % self.buf.len();
    }

    /// Sample written `delay` samples before the most recent one.
    #[inline]
    fn read(&self, delay: f32) -> f32 {
        let len = self.buf.len();
        if len == 0 {
            return 0.0;
        }
        let d = delay.clamp(0.0, (len - 2) as f32);
        let di = d.floor() as usize;
        let frac = d - di as f32;
        let newest = (self.pos + len - 1) % len;
        let i0 = (newest + len - di) % len;
        let i1 = (i0 + len - 1) % len;
        self.buf[i0] * (1.0 - frac) + self.buf[i1] * frac
    }
}

/// Sine LFO.
#[derive(Default, Clone, Copy)]
struct Lfo {
    phase: f32,
}

impl Lfo {
    #[inline]
    fn next(&mut self, hz: f32, sr: f32) -> f32 {
        let v = (self.phase * 2.0 * PI).sin();
        self.phase += hz / sr;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        v
    }
}

// ── Noise gate ───────────────────────────────────────────────────────────────

/// Downward gate with hysteresis (closes 6 dB below the open threshold).
pub struct NoiseGate {
    sr: f32,
    threshold: f32,
    release: f32,
    env: f32,
    gain: f32,
    open: bool,
}

impl Default for NoiseGate {
    fn default() -> Self {
        Self::new()
    }
}

impl NoiseGate {
    pub fn new() -> Self {
        Self {
            sr: 48_000.0,
            threshold: 0.3,
            release: 0.3,
            env: 0.0,
            gain: 0.0,
            open: false,
        }
    }
    fn threshold_db(&self) -> f32 {
        lerp(-80.0, -20.0, self.threshold)
    }
}

impl Plugin for NoiseGate {
    fn name(&self) -> &str {
        "gate"
    }
    fn init(&mut self, sample_rate: f64) -> anyhow::Result<()> {
        self.sr = sample_rate as f32;
        self.env = 0.0;
        self.gain = 0.0;
        self.open = false;
        Ok(())
    }
    fn process(&mut self, input: &[f32], output: &mut [f32]) -> anyhow::Result<()> {
        let env_rel = coef_ms(50.0, self.sr);
        let open_db = self.threshold_db();
        let close_db = open_db - 6.0;
        let att = coef_ms(1.0, self.sr);
        let rel = coef_ms(exp_map(20.0, 500.0, self.release), self.sr);
        for (o, &x) in output.iter_mut().zip(input) {
            let a = x.abs();
            self.env = if a > self.env {
                a
            } else {
                a + (self.env - a) * env_rel
            };
            let lvl = lin_to_db(self.env);
            if self.open && lvl < close_db {
                self.open = false;
            } else if !self.open && lvl > open_db {
                self.open = true;
            }
            let target = if self.open { 1.0 } else { 0.0 };
            let c = if target > self.gain { att } else { rel };
            self.gain = target + (self.gain - target) * c;
            *o = x * self.gain;
        }
        Ok(())
    }
    fn get_parameter(&self, id: &str) -> Option<f32> {
        match id {
            "threshold" => Some(self.threshold),
            "release" => Some(self.release),
            _ => None,
        }
    }
    fn set_parameter(&mut self, id: &str, v: f32) {
        let v = v.clamp(0.0, 1.0);
        match id {
            "threshold" => self.threshold = v,
            "release" => self.release = v,
            _ => {}
        }
    }
    any_impl!();
}

// ── Compressor ───────────────────────────────────────────────────────────────

/// Feed-forward peak compressor with soft knee and makeup gain.
pub struct Compressor {
    sr: f32,
    threshold: f32,
    ratio: f32,
    attack: f32,
    release: f32,
    makeup: f32,
    env_db: f32,
}

impl Default for Compressor {
    fn default() -> Self {
        Self::new()
    }
}

impl Compressor {
    pub fn new() -> Self {
        Self {
            sr: 48_000.0,
            threshold: 0.6,
            ratio: 0.3,
            attack: 0.3,
            release: 0.4,
            makeup: 0.2,
            env_db: 0.0,
        }
    }
    /// Static gain reduction in dB for an input level (soft knee 6 dB).
    pub fn gain_reduction_db(&self, level_db: f32) -> f32 {
        let t = lerp(-50.0, 0.0, self.threshold);
        let r = lerp(1.0, 20.0, self.ratio);
        let knee = 6.0;
        let over = level_db - t;
        if 2.0 * over < -knee {
            0.0
        } else if 2.0 * over > knee {
            over * (1.0 / r - 1.0)
        } else {
            let k = over + knee / 2.0;
            (1.0 / r - 1.0) * k * k / (2.0 * knee)
        }
    }
}

impl Plugin for Compressor {
    fn name(&self) -> &str {
        "comp"
    }
    fn init(&mut self, sample_rate: f64) -> anyhow::Result<()> {
        self.sr = sample_rate as f32;
        self.env_db = 0.0;
        Ok(())
    }
    fn process(&mut self, input: &[f32], output: &mut [f32]) -> anyhow::Result<()> {
        let att = coef_ms(exp_map(0.1, 50.0, self.attack), self.sr);
        let rel = coef_ms(exp_map(20.0, 500.0, self.release), self.sr);
        let makeup = db_to_lin(lerp(0.0, 24.0, self.makeup));
        for (o, &x) in output.iter_mut().zip(input) {
            let gr = self.gain_reduction_db(lin_to_db(x.abs()));
            // gr <= 0; a more negative value means more reduction (attack).
            let c = if gr < self.env_db { att } else { rel };
            self.env_db = gr + (self.env_db - gr) * c;
            *o = x * db_to_lin(self.env_db) * makeup;
        }
        Ok(())
    }
    fn get_parameter(&self, id: &str) -> Option<f32> {
        match id {
            "threshold" => Some(self.threshold),
            "ratio" => Some(self.ratio),
            "attack" => Some(self.attack),
            "release" => Some(self.release),
            "makeup" => Some(self.makeup),
            _ => None,
        }
    }
    fn set_parameter(&mut self, id: &str, v: f32) {
        let v = v.clamp(0.0, 1.0);
        match id {
            "threshold" => self.threshold = v,
            "ratio" => self.ratio = v,
            "attack" => self.attack = v,
            "release" => self.release = v,
            "makeup" => self.makeup = v,
            _ => {}
        }
    }
    any_impl!();
}

// ── Parametric EQ ────────────────────────────────────────────────────────────

/// Low shelf (100 Hz), sweepable peak (200 Hz–5 kHz) and high shelf (4 kHz).
/// Gains map 0..1 to −12..+12 dB; 0.5 is flat.
pub struct ParametricEq {
    sr: f32,
    low: f32,
    mid: f32,
    mid_freq: f32,
    q: f32,
    high: f32,
    bands: [Biquad; 3],
}

impl Default for ParametricEq {
    fn default() -> Self {
        Self::new()
    }
}

impl ParametricEq {
    pub fn new() -> Self {
        let mut eq = Self {
            sr: 48_000.0,
            low: 0.5,
            mid: 0.5,
            mid_freq: 0.5,
            q: 0.3,
            high: 0.5,
            bands: [Biquad::default(); 3],
        };
        eq.update();
        eq
    }
    fn update(&mut self) {
        let g = |v: f32| lerp(-12.0, 12.0, v);
        let q = exp_map(0.3, 6.0, self.q);
        self.bands[0].set(BiquadKind::LowShelf, self.sr, 100.0, 0.707, g(self.low));
        self.bands[1].set(
            BiquadKind::Peak,
            self.sr,
            exp_map(200.0, 5000.0, self.mid_freq),
            q,
            g(self.mid),
        );
        self.bands[2].set(BiquadKind::HighShelf, self.sr, 4000.0, 0.707, g(self.high));
    }
}

impl Plugin for ParametricEq {
    fn name(&self) -> &str {
        "eq"
    }
    fn init(&mut self, sample_rate: f64) -> anyhow::Result<()> {
        self.sr = sample_rate as f32;
        self.bands.iter_mut().for_each(Biquad::reset);
        self.update();
        Ok(())
    }
    fn process(&mut self, input: &[f32], output: &mut [f32]) -> anyhow::Result<()> {
        for (o, &x) in output.iter_mut().zip(input) {
            let mut y = x;
            for b in &mut self.bands {
                y = b.tick(y);
            }
            *o = y;
        }
        Ok(())
    }
    fn get_parameter(&self, id: &str) -> Option<f32> {
        match id {
            "low" => Some(self.low),
            "mid" => Some(self.mid),
            "mid_freq" => Some(self.mid_freq),
            "q" => Some(self.q),
            "high" => Some(self.high),
            _ => None,
        }
    }
    fn set_parameter(&mut self, id: &str, v: f32) {
        let v = v.clamp(0.0, 1.0);
        match id {
            "low" => self.low = v,
            "mid" => self.mid = v,
            "mid_freq" => self.mid_freq = v,
            "q" => self.q = v,
            "high" => self.high = v,
            _ => return,
        }
        self.update();
    }
    any_impl!();
}

// ── Overdrive ────────────────────────────────────────────────────────────────

/// Tube-screamer-flavoured drive: mid-hump pre-emphasis, asymmetric soft
/// clipping, one-pole tone control and output level.
pub struct Overdrive {
    sr: f32,
    drive: f32,
    tone: f32,
    level: f32,
    hp_x: f32,
    hp_y: f32,
    lp: f32,
}

impl Default for Overdrive {
    fn default() -> Self {
        Self::new()
    }
}

impl Overdrive {
    pub fn new() -> Self {
        Self {
            sr: 48_000.0,
            drive: 0.4,
            tone: 0.5,
            level: 0.5,
            hp_x: 0.0,
            hp_y: 0.0,
            lp: 0.0,
        }
    }
}

impl Plugin for Overdrive {
    fn name(&self) -> &str {
        "drive"
    }
    fn init(&mut self, sample_rate: f64) -> anyhow::Result<()> {
        self.sr = sample_rate as f32;
        self.hp_x = 0.0;
        self.hp_y = 0.0;
        self.lp = 0.0;
        Ok(())
    }
    fn process(&mut self, input: &[f32], output: &mut [f32]) -> anyhow::Result<()> {
        let gain = exp_map(1.0, 60.0, self.drive);
        let hp = (-2.0 * PI * 720.0 / self.sr).exp();
        let lp = 1.0 - (-2.0 * PI * exp_map(800.0, 8000.0, self.tone) / self.sr).exp();
        let out = db_to_lin(lerp(-24.0, 6.0, self.level));
        for (o, &x) in output.iter_mut().zip(input) {
            // Pre-emphasis high-pass, blended with dry lows (the "mid hump").
            let h = hp * (self.hp_y + x - self.hp_x);
            self.hp_x = x;
            self.hp_y = h;
            let pre = (h + 0.3 * x) * gain;
            // Asymmetric soft clip.
            let clipped = if pre >= 0.0 {
                pre.tanh()
            } else {
                (pre * 0.8).tanh() / 0.8
            };
            self.lp += lp * (clipped - self.lp);
            *o = self.lp * out;
        }
        Ok(())
    }
    fn get_parameter(&self, id: &str) -> Option<f32> {
        match id {
            "drive" => Some(self.drive),
            "tone" => Some(self.tone),
            "level" => Some(self.level),
            _ => None,
        }
    }
    fn set_parameter(&mut self, id: &str, v: f32) {
        let v = v.clamp(0.0, 1.0);
        match id {
            "drive" => self.drive = v,
            "tone" => self.tone = v,
            "level" => self.level = v,
            _ => {}
        }
    }
    any_impl!();
}

// ── Chorus / Flanger ─────────────────────────────────────────────────────────

/// Shared modulated-delay core for chorus and flanger.
pub struct ModulatedDelay {
    name: &'static str,
    sr: f32,
    base_ms: f32,
    max_depth_ms: f32,
    rate_range: (f32, f32),
    rate: f32,
    depth: f32,
    feedback: f32,
    mix: f32,
    lfo: Lfo,
    line: ModDelay,
    fb: f32,
}

impl ModulatedDelay {
    fn new(
        name: &'static str, base_ms: f32, max_depth_ms: f32, rate_range: (f32, f32), feedback: f32,
    ) -> Self {
        let mut s = Self {
            name,
            sr: 48_000.0,
            base_ms,
            max_depth_ms,
            rate_range,
            rate: 0.3,
            depth: 0.5,
            feedback,
            mix: 0.5,
            lfo: Lfo::default(),
            line: ModDelay::default(),
            fb: 0.0,
        };
        s.line.allocate(s.len_for(48_000.0));
        s
    }
    fn len_for(&self, sr: f32) -> usize {
        ((self.base_ms + self.max_depth_ms + 2.0) * 0.001 * sr) as usize + 4
    }
}

/// Chorus: 15 ms base delay, ±5 ms modulation, no feedback.
pub fn chorus() -> ModulatedDelay {
    ModulatedDelay::new("chorus", 15.0, 5.0, (0.1, 5.0), 0.0)
}

/// Flanger: 1 ms base delay, ±3 ms modulation, with feedback.
pub fn flanger() -> ModulatedDelay {
    ModulatedDelay::new("flanger", 1.0, 3.0, (0.05, 2.0), 0.5)
}

impl Plugin for ModulatedDelay {
    fn name(&self) -> &str {
        self.name
    }
    fn init(&mut self, sample_rate: f64) -> anyhow::Result<()> {
        self.sr = sample_rate as f32;
        self.line.allocate(self.len_for(self.sr));
        self.lfo = Lfo::default();
        self.fb = 0.0;
        Ok(())
    }
    fn process(&mut self, input: &[f32], output: &mut [f32]) -> anyhow::Result<()> {
        let hz = exp_map(self.rate_range.0, self.rate_range.1, self.rate);
        let ms_to_s = 0.001 * self.sr;
        let fb = self.feedback * 0.9;
        for (o, &x) in output.iter_mut().zip(input) {
            let m = self.lfo.next(hz, self.sr) * 0.5 + 0.5;
            let d = (self.base_ms + m * self.depth * self.max_depth_ms) * ms_to_s;
            self.line.push(x + self.fb * fb);
            let wet = self.line.read(d);
            self.fb = wet;
            *o = x * (1.0 - self.mix * 0.5) + wet * self.mix;
        }
        Ok(())
    }
    fn get_parameter(&self, id: &str) -> Option<f32> {
        match id {
            "rate" => Some(self.rate),
            "depth" => Some(self.depth),
            "mix" => Some(self.mix),
            "feedback" if self.name == "flanger" => Some(self.feedback),
            _ => None,
        }
    }
    fn set_parameter(&mut self, id: &str, v: f32) {
        let v = v.clamp(0.0, 1.0);
        match id {
            "rate" => self.rate = v,
            "depth" => self.depth = v,
            "mix" => self.mix = v,
            "feedback" if self.name == "flanger" => self.feedback = v,
            _ => {}
        }
    }
    any_impl!();
}

// ── Phaser ───────────────────────────────────────────────────────────────────

/// Four first-order all-pass stages swept 200 Hz–2 kHz, with feedback.
pub struct Phaser {
    sr: f32,
    rate: f32,
    depth: f32,
    feedback: f32,
    mix: f32,
    lfo: Lfo,
    x1: [f32; 4],
    y1: [f32; 4],
    fb: f32,
}

impl Default for Phaser {
    fn default() -> Self {
        Self::new()
    }
}

impl Phaser {
    pub fn new() -> Self {
        Self {
            sr: 48_000.0,
            rate: 0.3,
            depth: 0.7,
            feedback: 0.4,
            mix: 0.5,
            lfo: Lfo::default(),
            x1: [0.0; 4],
            y1: [0.0; 4],
            fb: 0.0,
        }
    }
}

impl Plugin for Phaser {
    fn name(&self) -> &str {
        "phaser"
    }
    fn init(&mut self, sample_rate: f64) -> anyhow::Result<()> {
        self.sr = sample_rate as f32;
        self.x1 = [0.0; 4];
        self.y1 = [0.0; 4];
        self.fb = 0.0;
        Ok(())
    }
    fn process(&mut self, input: &[f32], output: &mut [f32]) -> anyhow::Result<()> {
        let hz = exp_map(0.05, 5.0, self.rate);
        for (o, &x) in output.iter_mut().zip(input) {
            let m = self.lfo.next(hz, self.sr) * 0.5 + 0.5;
            let f = exp_map(200.0, 2000.0, m * self.depth);
            let t = (PI * f / self.sr).tan();
            let a = (t - 1.0) / (t + 1.0);
            let mut y = x + self.fb * self.feedback * 0.9;
            for k in 0..4 {
                let out = a * y + self.x1[k] - a * self.y1[k];
                self.x1[k] = y;
                self.y1[k] = out;
                y = out;
            }
            self.fb = y;
            *o = x * (1.0 - self.mix * 0.5) + y * self.mix * 0.5;
        }
        Ok(())
    }
    fn get_parameter(&self, id: &str) -> Option<f32> {
        match id {
            "rate" => Some(self.rate),
            "depth" => Some(self.depth),
            "feedback" => Some(self.feedback),
            "mix" => Some(self.mix),
            _ => None,
        }
    }
    fn set_parameter(&mut self, id: &str, v: f32) {
        let v = v.clamp(0.0, 1.0);
        match id {
            "rate" => self.rate = v,
            "depth" => self.depth = v,
            "feedback" => self.feedback = v,
            "mix" => self.mix = v,
            _ => {}
        }
    }
    any_impl!();
}

// ── Tremolo ──────────────────────────────────────────────────────────────────

/// Amplitude modulation, 1–15 Hz.
pub struct Tremolo {
    sr: f32,
    rate: f32,
    depth: f32,
    lfo: Lfo,
}

impl Default for Tremolo {
    fn default() -> Self {
        Self::new()
    }
}

impl Tremolo {
    pub fn new() -> Self {
        Self {
            sr: 48_000.0,
            rate: 0.4,
            depth: 0.5,
            lfo: Lfo::default(),
        }
    }
}

impl Plugin for Tremolo {
    fn name(&self) -> &str {
        "tremolo"
    }
    fn init(&mut self, sample_rate: f64) -> anyhow::Result<()> {
        self.sr = sample_rate as f32;
        self.lfo = Lfo::default();
        Ok(())
    }
    fn process(&mut self, input: &[f32], output: &mut [f32]) -> anyhow::Result<()> {
        let hz = exp_map(1.0, 15.0, self.rate);
        for (o, &x) in output.iter_mut().zip(input) {
            let m = self.lfo.next(hz, self.sr) * 0.5 + 0.5;
            *o = x * (1.0 - self.depth * m);
        }
        Ok(())
    }
    fn get_parameter(&self, id: &str) -> Option<f32> {
        match id {
            "rate" => Some(self.rate),
            "depth" => Some(self.depth),
            _ => None,
        }
    }
    fn set_parameter(&mut self, id: &str, v: f32) {
        let v = v.clamp(0.0, 1.0);
        match id {
            "rate" => self.rate = v,
            "depth" => self.depth = v,
            _ => {}
        }
    }
    any_impl!();
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f64 = 48_000.0;

    fn sine(freq: f32, amp: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| amp * (2.0 * PI * freq * i as f32 / SR as f32).sin())
            .collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    fn run(p: &mut dyn Plugin, input: &[f32]) -> Vec<f32> {
        p.init(SR).unwrap();
        let mut out = vec![0.0; input.len()];
        // Odd block sizes on purpose.
        let mut i = 0;
        while i < input.len() {
            let end = (i + 77).min(input.len());
            p.process(&input[i..end], &mut out[i..end]).unwrap();
            i = end;
        }
        out
    }

    #[test]
    fn gate_mutes_noise_and_passes_playing() {
        let mut g = NoiseGate::new();
        g.set_parameter("threshold", 0.5); // −50 dB
        let quiet = run(&mut g, &sine(200.0, 0.0005, 24_000)); // ≈ −66 dB
        assert!(rms(&quiet[12_000..]) < 1e-5);
        let loud = run(&mut g, &sine(200.0, 0.3, 24_000));
        let ratio = rms(&loud[12_000..]) / rms(&sine(200.0, 0.3, 24_000)[12_000..]);
        assert!((ratio - 1.0).abs() < 0.02, "{ratio}");
    }

    #[test]
    fn compressor_reduces_loud_more_than_quiet() {
        let mut c = Compressor::new();
        c.set_parameter("threshold", 0.6); // −20 dB
        c.set_parameter("ratio", 1.0); // 20:1
        c.set_parameter("makeup", 0.0);
        let loud_in = sine(220.0, 0.9, 48_000);
        let loud = run(&mut c, &loud_in);
        let quiet_in = sine(220.0, 0.01, 48_000);
        let quiet = run(&mut c, &quiet_in);
        let loud_gain = rms(&loud[24_000..]) / rms(&loud_in[24_000..]);
        let quiet_gain = rms(&quiet[24_000..]) / rms(&quiet_in[24_000..]);
        assert!((quiet_gain - 1.0).abs() < 0.05, "quiet {quiet_gain}");
        assert!(loud_gain < 0.25, "loud {loud_gain}");
        // Static curve: 20 dB over at 20:1 → ≈19 dB reduction.
        assert!((c.gain_reduction_db(0.0) + 19.0).abs() < 0.1);
    }

    #[test]
    fn eq_is_flat_at_noon_and_boosts_its_band() {
        let input = sine(1000.0, 0.5, 48_000);
        let mut eq = ParametricEq::new();
        let flat = run(&mut eq, &input);
        assert!((rms(&flat[4800..]) / rms(&input[4800..]) - 1.0).abs() < 0.01);

        let mut eq = ParametricEq::new();
        // mid_freq 0.5 → ~1 kHz; +12 dB at the centre.
        eq.set_parameter("mid_freq", (1000f32 / 200.0).ln() / (25f32).ln());
        eq.set_parameter("mid", 1.0);
        let boosted = run(&mut eq, &input);
        let db = lin_to_db(rms(&boosted[4800..]) / rms(&input[4800..]));
        assert!((db - 12.0).abs() < 0.5, "{db} dB");

        let mut eq = ParametricEq::new();
        eq.set_parameter("low", 0.0);
        let low = sine(40.0, 0.5, 48_000);
        let cut = run(&mut eq, &low);
        let db = lin_to_db(rms(&cut[9600..]) / rms(&low[9600..]));
        assert!(db < -9.0, "{db} dB");
    }

    #[test]
    fn overdrive_adds_harmonics_and_stays_bounded() {
        let mut d = Overdrive::new();
        d.set_parameter("drive", 1.0);
        d.set_parameter("level", 1.0);
        let out = run(&mut d, &sine(220.0, 0.5, 48_000));
        assert!(out.iter().all(|v| v.is_finite() && v.abs() <= 2.6));
        // Clipped sine has a crest factor below a pure sine's √2.
        let s = &out[4800..];
        let peak = s.iter().fold(0f32, |m, v| m.max(v.abs()));
        assert!(peak / rms(s) < 1.38, "crest {}", peak / rms(s));
    }

    #[test]
    fn modulation_effects_are_stable_and_audible() {
        let input = sine(330.0, 0.5, 96_000);
        let mut fx: Vec<Box<dyn Plugin>> = vec![
            Box::new(chorus()),
            Box::new(flanger()),
            Box::new(Phaser::new()),
            Box::new(Tremolo::new()),
        ];
        for p in fx.iter_mut() {
            p.set_parameter("depth", 1.0);
            p.set_parameter("feedback", 1.0);
            let out = run(p.as_mut(), &input);
            assert!(
                out.iter().all(|v| v.is_finite() && v.abs() < 4.0),
                "{}",
                p.name()
            );
            let diff: f32 = out.iter().zip(&input).map(|(a, b)| (a - b).abs()).sum();
            assert!(diff / input.len() as f32 > 1e-3, "{} did nothing", p.name());
        }
    }

    #[test]
    fn tremolo_depth_zero_is_transparent() {
        let mut t = Tremolo::new();
        t.set_parameter("depth", 0.0);
        let input = sine(330.0, 0.5, 4800);
        assert_eq!(run(&mut t, &input), input);
    }

    #[test]
    fn mod_delay_reads_exact_and_fractional() {
        let mut d = ModDelay::default();
        d.allocate(16);
        for i in 0..10 {
            d.push(i as f32);
        }
        assert_eq!(d.read(0.0), 9.0);
        assert_eq!(d.read(3.0), 6.0);
        assert!((d.read(2.5) - 6.5).abs() < 1e-6);
        // Out-of-range delays clamp instead of panicking.
        assert!(d.read(1e9).is_finite());
        let empty = ModDelay::default();
        assert_eq!(empty.read(1.0), 0.0);
    }

    #[test]
    fn parameters_round_trip_and_clamp() {
        let mut fx: Vec<Box<dyn Plugin>> = vec![
            Box::new(NoiseGate::new()),
            Box::new(Compressor::new()),
            Box::new(ParametricEq::new()),
            Box::new(Overdrive::new()),
            Box::new(chorus()),
            Box::new(flanger()),
            Box::new(Phaser::new()),
            Box::new(Tremolo::new()),
        ];
        for p in fx.iter_mut() {
            for id in ["threshold", "ratio", "low", "drive", "rate", "depth", "mix"] {
                if p.get_parameter(id).is_some() {
                    p.set_parameter(id, 0.25);
                    assert_eq!(p.get_parameter(id), Some(0.25), "{}/{id}", p.name());
                    p.set_parameter(id, 7.0);
                    assert_eq!(p.get_parameter(id), Some(1.0), "{}/{id}", p.name());
                }
            }
            assert_eq!(p.get_parameter("nope"), None);
        }
    }
}
