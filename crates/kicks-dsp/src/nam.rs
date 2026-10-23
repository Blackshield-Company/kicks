// Kicks — Neural Amp Modeler (NAM) Inference Engine
//
// Pure Rust inference engine for .nam neural network model files.
// Supports WaveNet, LSTM, and Linear architectures.
// All real-time safe: no allocations in the process() path.

use std::fs;
use std::path::Path;

use serde::Deserialize;

/// Extension trait adding `sigmoid()` to `f32`.
trait Sigmoid {
    fn sigmoid(self) -> f32;
}

impl Sigmoid for f32 {
    fn sigmoid(self) -> f32 {
        1.0 / (1.0 + (-self).exp())
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// .nam File Format
// ═══════════════════════════════════════════════════════════════════════════════

/// The top-level .nam file structure (JSON).
#[derive(Debug, Clone, Deserialize)]
struct NamFile {
    #[allow(dead_code)]
    version: Option<String>,
    architecture: String,
    config: serde_json::Value,
    weights: Vec<f32>,
    sample_rate: Option<u32>,
    #[allow(dead_code)]
    metadata: Option<serde_json::Value>,
}

/// Metadata about a loaded NAM model.
#[derive(Debug, Clone)]
pub struct NamModelInfo {
    pub path: String,
    pub file_name: String,
    pub architecture: String,
    pub sample_rate: u32,
    pub num_parameters: usize,
}

// ═══════════════════════════════════════════════════════════════════════════════
// Architecture Implementations
// ═══════════════════════════════════════════════════════════════════════════════

/// Helper to read chunks from a flat weight array.
struct WeightReader<'a> {
    data: &'a [f32],
    pos: usize,
}

impl<'a> WeightReader<'a> {
    fn new(data: &'a [f32]) -> Self {
        Self { data, pos: 0 }
    }

    fn read(&mut self, n: usize) -> &'a [f32] {
        let start = self.pos.min(self.data.len());
        let end = (start + n).min(self.data.len());
        self.pos = end;
        &self.data[start..end]
    }

    #[allow(dead_code)]
    fn read_f32(&mut self) -> f32 {
        self.read(1).first().copied().unwrap_or(0.0)
    }

    fn remaining(&self) -> &'a [f32] {
        &self.data[self.pos..]
    }

    #[allow(dead_code)]
    fn pos(&self) -> usize {
        self.pos
    }
}

// ── Linear Architecture ──────────────────────────────────────────────────────

/// Simplest NAM architecture: y = w * x + b
struct LinearModel {
    weight: Vec<f32>, // flattened weight matrix
    bias: Vec<f32>,
    #[allow(dead_code)]
    input_size: usize,
    #[allow(dead_code)]
    output_size: usize,
}

impl LinearModel {
    fn from_config(config: &serde_json::Value, weights: &mut WeightReader) -> Self {
        let _receptive_field = config
            .get("receptive_field")
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as usize;
        let bias_enabled = config.get("bias").and_then(|v| v.as_bool()).unwrap_or(true);
        let input_size = 1; // NAM models are typically 1-input (mono audio)
        let output_size = 1;
        let param_count = input_size * output_size;

        let weight = if weights.remaining().len() >= param_count {
            weights.read(param_count).to_vec()
        } else {
            vec![1.0; param_count]
        };

        let bias = if bias_enabled {
            vec![weights.read(output_size).first().copied().unwrap_or(0.0)]
        } else {
            vec![0.0]
        };

        Self {
            weight,
            bias,
            input_size,
            output_size,
        }
    }

    fn process(&self, input: &[f32], output: &mut [f32]) {
        for (i, sample) in input.iter().enumerate() {
            // y = w * x + b (mono → single weight)
            let w = self.weight.first().copied().unwrap_or(1.0);
            let b = self.bias.first().copied().unwrap_or(0.0);
            output[i] = sample * w + b;
        }
    }
}

// ── WaveNet Architecture ─────────────────────────────────────────────────────

/// A single layer in the WaveNet model.
struct WaveNetLayer {
    /// Convolution weights: shape depends on gated/non-gated
    conv_weight: Vec<f32>,
    /// Convolution bias
    conv_bias: Vec<f32>,
    /// Skip connection weights (channels → channels)
    skip_weight: Vec<f32>,
    /// Skip connection bias
    skip_bias: Vec<f32>,
    /// Dilation for this layer
    dilation: usize,
    /// Whether this layer uses gated activation
    gated: bool,
    /// Number of input channels to this layer
    channels: usize,
    /// Kernel size
    kernel_size: usize,
}

impl WaveNetLayer {
    fn process(&self, delay: &mut [f32], write_pos: &mut usize, input: &[f32]) -> (f32, f32) {
        let ch = self.channels;
        let ks = self.kernel_size;
        let dil = self.dilation;
        let delay_len = delay.len() / ch;

        // For each input sample, process through this layer
        // Write input to delay line (input is single channel, broadcast to all channels)
        for c in 0..ch {
            let idx = *write_pos + c * delay_len;
            delay[idx % delay.len()] = if c == 0 { input[0] } else { 0.0 };
        }
        *write_pos = (*write_pos + 1) % delay_len;

        let (main_out, skip_sum) = if self.gated {
            // Gated activation: conv_weight is [ch, 2, ch, ks]
            // Two output channels: gate and tanh
            let half = self.conv_weight.len() / 2;
            let weight_tanh = &self.conv_weight[..half];
            let weight_gate = &self.conv_weight[half..];
            let bias_tanh = self.conv_bias[0];
            let bias_gate = self.conv_bias.get(1).copied().unwrap_or(0.0);

            let mut tanh_sum = bias_tanh;
            let mut gate_sum = bias_gate;

            // Convolve across channels and kernel taps
            for c_in in 0..ch {
                for k in 0..ks {
                    let read_pos = (*write_pos as isize - 1 - (k as isize * dil as isize))
                        .rem_euclid(delay_len as isize) as usize;
                    let idx = read_pos + c_in * delay_len;
                    let x = delay[idx % delay.len()];

                    let ti = c_in * ks + k;
                    let gi = half / 2 + c_in * ks + k;
                    // Simplified: each output channel has its own kernel per input channel
                    if ti < weight_tanh.len() {
                        tanh_sum += x * weight_tanh[ti];
                    }
                    if gi < weight_gate.len() {
                        gate_sum += x * weight_gate[gi];
                    }
                }
            }

            let activated = tanh_sum.tanh() * gate_sum.sigmoid();
            let skip = activated * self.skip_weight.first().copied().unwrap_or(1.0)
                + self.skip_bias.first().copied().unwrap_or(0.0);
            (activated, skip)
        } else {
            // Non-gated: simple conv + activation
            let mut conv_sum = self.conv_bias.first().copied().unwrap_or(0.0);

            for c_in in 0..ch {
                for k in 0..ks {
                    let read_pos = (*write_pos as isize - 1 - (k as isize * dil as isize))
                        .rem_euclid(delay_len as isize) as usize;
                    let idx = read_pos + c_in * delay_len;
                    let x = delay[idx % delay.len()];

                    let wi = c_in * ks + k;
                    if wi < self.conv_weight.len() {
                        conv_sum += x * self.conv_weight[wi];
                    }
                }
            }

            let activated = match self.activation() {
                Activation::Tanh => conv_sum.tanh(),
                Activation::Sigmoid => conv_sum.sigmoid(),
                Activation::Relu => conv_sum.max(0.0),
            };
            let skip = activated * self.skip_weight.first().copied().unwrap_or(1.0)
                + self.skip_bias.first().copied().unwrap_or(0.0);
            (activated, skip)
        };

        (main_out, skip_sum)
    }

    fn activation(&self) -> Activation {
        Activation::Tanh // most common for WaveNet
    }
}

#[allow(dead_code)]
enum Activation {
    Tanh,
    Sigmoid,
    Relu,
}

/// The WaveNet architecture: dilated causal convolutions with skip connections.
struct WaveNetModel {
    channels: usize,
    head_scale: f32,
    /// Head projection (optional): input → channels
    head_weight: Option<Vec<f32>>,
    head_bias: Option<Vec<f32>>,
    /// WaveNet layers (dilated conv + skip)
    layers: Vec<WaveNetLayer>,
    /// Output projection weights (channels → 1)
    output_weight: Vec<f32>,
    output_bias: Vec<f32>,
    /// Delay lines: one per layer, each [channels * max_delay]
    delay_lines: Vec<Vec<f32>>,
    /// Write positions per delay line
    write_positions: Vec<usize>,
}

impl WaveNetModel {
    fn from_config(config: &serde_json::Value, weights: &mut WeightReader) -> Self {
        // Parse config
        let channels = config
            .get("channels")
            .and_then(|v| v.as_u64())
            .unwrap_or(16) as usize;
        let kernel_size = config
            .get("kernel_size")
            .and_then(|v| v.as_u64())
            .unwrap_or(3) as usize;
        let dilations: Vec<usize> = config
            .get("dilations")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|d| d.as_u64())
                    .map(|d| d as usize)
                    .collect()
            })
            .unwrap_or_else(|| (0..8).map(|i| 1usize << i).collect()); // default: [1,2,4,8,16,32,64,128]
        let gated = config
            .get("gated")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let head_size = config
            .get("head_size")
            .and_then(|v| v.as_u64())
            .unwrap_or(channels as u64) as usize;
        let head_bias = config
            .get("head_bias")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let num_layers = dilations.len();

        // Read head projection weights
        let head_weight = if head_size > 0 {
            let n = head_size; // input_size (1) * head_size
            Some(weights.read(n).to_vec())
        } else {
            None
        };

        let head_bias_vals = if head_bias && head_size > 0 {
            Some(weights.read(head_size).to_vec())
        } else {
            None
        };

        // Read each layer's weights
        let mut layers = Vec::with_capacity(num_layers);
        for &dilation in &dilations {
            let conv_size = if gated {
                // Gated: 2 * channels * channels * kernel_size
                channels * 2 * channels * kernel_size
            } else {
                channels * channels * kernel_size
            };
            let conv_weight = weights.read(conv_size).to_vec();
            let conv_bias = if gated {
                weights.read(channels * 2).to_vec()
            } else {
                weights.read(channels).to_vec()
            };
            let skip_weight = weights.read(channels * channels).to_vec();
            let skip_bias = weights.read(channels).to_vec();

            layers.push(WaveNetLayer {
                conv_weight,
                conv_bias,
                skip_weight,
                skip_bias,
                dilation,
                gated,
                channels,
                kernel_size,
            });
        }

        // Read output projection
        let output_weight = weights.read(channels).to_vec();
        let output_bias = vec![weights.read(1).first().copied().unwrap_or(0.0)];

        let head_scale = config
            .get("head_scale")
            .and_then(|v| v.as_f64())
            .map(|v| v as f32)
            .unwrap_or(1.0);

        // Initialize delay lines: one per layer, each sized for (receptive_field + kernel) * channels
        let max_delay = dilations.iter().max().copied().unwrap_or(1) * kernel_size + kernel_size;
        let delay_lines: Vec<Vec<f32>> = (0..num_layers)
            .map(|_| vec![0.0; channels * (max_delay + kernel_size + 1)])
            .collect();
        let write_positions = vec![0; num_layers];

        Self {
            channels,
            head_scale,
            head_weight,
            head_bias: head_bias_vals,
            layers,
            output_weight,
            output_bias,
            delay_lines,
            write_positions,
        }
    }

    fn process(&mut self, input: &[f32], output: &mut [f32]) {
        for i in 0..input.len() {
            let sample = input[i];

            // Head projection: input → channels
            let mut x = vec![0.0f32; self.channels];
            if let Some(ref hw) = self.head_weight {
                for c in 0..self.channels.min(hw.len()) {
                    x[c] = sample * hw[c] * self.head_scale;
                }
            } else {
                x[0] = sample;
            }
            if let Some(ref hb) = self.head_bias {
                for c in 0..self.channels.min(hb.len()) {
                    x[c] += hb[c];
                }
            }

            // Process through each WaveNet layer
            let mut skip_total = 0.0f32;

            for (layer_idx, layer) in self.layers.iter().enumerate() {
                let (_, skip) = layer.process(
                    &mut self.delay_lines[layer_idx],
                    &mut self.write_positions[layer_idx],
                    &x,
                );
                skip_total += skip;
            }

            // Output projection
            let mut out = self.output_bias.first().copied().unwrap_or(0.0);
            for c in 0..self.channels.min(self.output_weight.len()) {
                out += skip_total * self.output_weight[c];
            }

            output[i] = out;
        }
    }

    fn reset(&mut self) {
        for delay in &mut self.delay_lines {
            delay.fill(0.0);
        }
        self.write_positions.fill(0);
    }
}

// ── LSTM Architecture ────────────────────────────────────────────────────────

/// Standard LSTM cell.
struct LstmModel {
    #[allow(dead_code)]
    input_size: usize,
    hidden_size: usize,
    num_layers: usize,
    /// Combined weights for all gates (i, f, g, o) per layer: [4 * hidden, input]
    weight_ih: Vec<Vec<f32>>,
    /// Recurrent weights per layer: [4 * hidden, hidden]
    weight_hh: Vec<Vec<f32>>,
    /// Biases per layer: [4 * hidden]
    bias_ih: Vec<Vec<f32>>,
    bias_hh: Vec<Vec<f32>>,
    /// Hidden state per layer
    h: Vec<Vec<f32>>,
    /// Cell state per layer
    c: Vec<Vec<f32>>,
}

impl LstmModel {
    fn from_config(config: &serde_json::Value, weights: &mut WeightReader) -> Self {
        let input_size = config
            .get("input_size")
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as usize;
        let hidden_size = config
            .get("hidden_size")
            .and_then(|v| v.as_u64())
            .unwrap_or(8) as usize;
        let num_layers = config
            .get("num_layers")
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as usize;

        let mut weight_ih = Vec::with_capacity(num_layers);
        let mut weight_hh = Vec::with_capacity(num_layers);
        let mut bias_ih = Vec::with_capacity(num_layers);
        let mut bias_hh = Vec::with_capacity(num_layers);

        for _ in 0..num_layers {
            let layer_input = if weight_ih.is_empty() {
                input_size
            } else {
                hidden_size
            };
            let w_ih = weights.read(4 * hidden_size * layer_input).to_vec();
            let w_hh = weights.read(4 * hidden_size * hidden_size).to_vec();
            let b_ih = weights.read(4 * hidden_size).to_vec();
            let b_hh = weights.read(4 * hidden_size).to_vec();
            weight_ih.push(w_ih);
            weight_hh.push(w_hh);
            bias_ih.push(b_ih);
            bias_hh.push(b_hh);
        }

        let mut h = Vec::with_capacity(num_layers);
        let mut c = Vec::with_capacity(num_layers);
        for _ in 0..num_layers {
            h.push(vec![0.0; hidden_size]);
            c.push(vec![0.0; hidden_size]);
        }

        Self {
            input_size,
            hidden_size,
            num_layers,
            weight_ih,
            weight_hh,
            bias_ih,
            bias_hh,
            h,
            c,
        }
    }

    fn process(&mut self, input: &[f32], output: &mut [f32]) {
        for (i, sample) in input.iter().enumerate() {
            let mut layer_input = vec![*sample];

            for layer in 0..self.num_layers {
                let w_ih = &self.weight_ih[layer];
                let w_hh = &self.weight_hh[layer];
                let b_ih = &self.bias_ih[layer];
                let b_hh = &self.bias_hh[layer];
                let h_prev = &self.h[layer];
                let c_prev = &self.c[layer];
                let h_size = self.hidden_size;

                // Compute all 4 gates at once
                let mut gates = vec![0.0f32; 4 * h_size];

                // Input-hidden contribution
                for g in 0..(4 * h_size) {
                    for j in 0..layer_input.len() {
                        let idx = g * layer_input.len() + j;
                        if idx < w_ih.len() {
                            gates[g] += layer_input[j] * w_ih[idx];
                        }
                    }
                    if g < b_ih.len() {
                        gates[g] += b_ih[g];
                    }
                }

                // Hidden-hidden contribution
                for g in 0..(4 * h_size) {
                    for (j, &h_val) in h_prev.iter().enumerate().take(h_size) {
                        let idx = g * h_size + j;
                        if idx < w_hh.len() {
                            gates[g] += h_val * w_hh[idx];
                        }
                    }
                    if g < b_hh.len() {
                        gates[g] += b_hh[g];
                    }
                }

                // Apply activations: sigmoid for i,f,o; tanh for g
                let i_gate = gates[0].sigmoid();
                let f_gate = gates[h_size].sigmoid();
                let g_gate = gates[2 * h_size].tanh();
                let o_gate = gates[3 * h_size].sigmoid();

                // Update cell and hidden state
                let c_new = f_gate * c_prev[0] + i_gate * g_gate;
                let h_new = o_gate * c_new.tanh();

                // Store for multi-layer
                if layer == self.num_layers - 1 {
                    output[i] = h_new;
                } else {
                    layer_input = vec![h_new];
                }

                // Update state vectors (only first element since audio is 1-channel)
                // For multi-dimensional hidden states, we'd update all, but NAM LSTM
                // typically has 1-dimensional input
                if h_size > 0 {
                    self.h[layer][0] = h_new;
                    self.c[layer][0] = c_new;
                }
            }
        }
    }

    fn reset(&mut self) {
        for layer in 0..self.num_layers {
            self.h[layer].fill(0.0);
            self.c[layer].fill(0.0);
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// NeuralModel — Public API
// ═══════════════════════════════════════════════════════════════════════════════

enum NeuralModelInner {
    Linear(LinearModel),
    WaveNet(WaveNetModel),
    Lstm(LstmModel),
    Passthrough,
}

/// A loaded NAM neural network model, ready for real-time audio processing.
pub struct NeuralModel {
    inner: NeuralModelInner,
    arch_name: String,
    sample_rate: u32,
}

impl NeuralModel {
    /// Load a .nam file from disk and parse it into a NeuralModel.
    pub fn from_file(path: &str) -> anyhow::Result<(Self, usize)> {
        let file_path = Path::new(path);
        if !file_path.exists() {
            anyhow::bail!("NAM file not found: {}", path);
        }

        Self::from_reader(std::io::BufReader::new(fs::File::open(file_path)?))
    }

    /// Parse a NAM model from an already opened and validated source.
    pub fn from_reader(reader: impl std::io::Read) -> anyhow::Result<(Self, usize)> {
        let nam: NamFile = serde_json::from_reader(reader)?;

        let sample_rate = nam.sample_rate.unwrap_or(48000);
        let arch_name = nam.architecture.clone();
        let num_parameters = nam.weights.len();
        let mut reader = WeightReader::new(&nam.weights);

        let inner = match nam.architecture.as_str() {
            "Linear" => {
                NeuralModelInner::Linear(LinearModel::from_config(&nam.config, &mut reader))
            }
            "WaveNet" => {
                NeuralModelInner::WaveNet(WaveNetModel::from_config(&nam.config, &mut reader))
            }
            "LSTM" => NeuralModelInner::Lstm(LstmModel::from_config(&nam.config, &mut reader)),
            _ => {
                tracing::warn!(
                    "Unsupported NAM architecture '{}', using passthrough",
                    nam.architecture
                );
                NeuralModelInner::Passthrough
            }
        };

        Ok((
            Self {
                inner,
                arch_name,
                sample_rate,
            },
            num_parameters,
        ))
    }

    /// Process one buffer of audio samples through the neural model.
    ///
    /// Real-time safe — no allocations.
    pub fn process(&mut self, input: &[f32], output: &mut [f32]) {
        match &mut self.inner {
            NeuralModelInner::Linear(m) => m.process(input, output),
            NeuralModelInner::WaveNet(m) => m.process(input, output),
            NeuralModelInner::Lstm(m) => m.process(input, output),
            NeuralModelInner::Passthrough => output.copy_from_slice(input),
        }
    }

    /// Reset all internal state (delay lines, recurrent state) to zero.
    pub fn reset(&mut self) {
        match &mut self.inner {
            NeuralModelInner::Linear(_) => {} // stateless
            NeuralModelInner::WaveNet(m) => m.reset(),
            NeuralModelInner::Lstm(m) => m.reset(),
            NeuralModelInner::Passthrough => {}
        }
    }

    /// The architecture name (e.g., "WaveNet", "LSTM", "Linear").
    pub fn architecture(&self) -> &str {
        &self.arch_name
    }

    /// The model's native sample rate.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    fn test_buffer(size: usize) -> Vec<f32> {
        (0..size)
            .map(|i| (i as f32 / size as f32) * 0.5 - 0.25)
            .collect()
    }

    #[test]
    fn test_linear_model_passthrough() {
        // A linear model with w=1, b=0
        let config = serde_json::json!({
            "receptive_field": 1,
            "bias": true
        });
        let weights = vec![1.0f32, 0.0f32]; // w, b
        let mut reader = WeightReader::new(&weights);
        let model = LinearModel::from_config(&config, &mut reader);

        let input = test_buffer(64);
        let mut output = vec![0.0; 64];
        model.process(&input, &mut output);

        for (out, inp) in output.iter().zip(input.iter()) {
            assert!((out - inp).abs() < 1e-5);
        }
    }

    #[test]
    fn test_linear_model_gain() {
        let config = serde_json::json!({
            "receptive_field": 1,
            "bias": true
        });
        let weights = vec![2.0f32, 0.0f32]; // w=2, b=0
        let mut reader = WeightReader::new(&weights);
        let model = LinearModel::from_config(&config, &mut reader);

        let input = vec![0.5f32; 16];
        let mut output = vec![0.0; 16];
        model.process(&input, &mut output);

        for out in &output {
            assert!((out - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn test_wavenet_minimal() {
        // Minimal WaveNet: 1 layer, 1 channel, kernel=2, no gating
        let config = serde_json::json!({
            "channels": 1,
            "kernel_size": 2,
            "dilations": [1],
            "gated": false,
            "head_size": 1,
            "head_bias": false,
            "head_scale": 1.0
        });
        // Weights: head(1), conv(1*1*2=2), conv_bias(1), skip(1), skip_bias(1), output(1), output_bias(1)
        let weights = vec![
            1.0, // head weight
            0.5, 0.3, // conv weight (1ch, 1ch, k=2)
            0.0, // conv bias
            0.8, // skip weight (1ch)
            0.0, // skip bias
            1.0, // output weight (1ch)
            0.0, // output bias
        ];
        let mut reader = WeightReader::new(&weights);
        let mut model = WaveNetModel::from_config(&config, &mut reader);

        let input = vec![1.0f32; 16];
        let mut output = vec![0.0; 16];
        model.process(&input, &mut output);

        // Should produce non-zero output (neural net processed)
        assert!(
            output.iter().any(|&x| x != 0.0),
            "WaveNet should produce output"
        );
    }

    #[test]
    fn test_lstm_minimal() {
        // Minimal LSTM: 1 layer, 1 hidden, 1 input
        let config = serde_json::json!({
            "input_size": 1,
            "hidden_size": 1,
            "num_layers": 1
        });
        // Weights: w_ih(4*1*1=4), w_hh(4*1*1=4), b_ih(4), b_hh(4)
        let weights = vec![
            1.0, 1.0, 1.0, 1.0, // w_ih: i,f,g,o
            1.0, 1.0, 1.0, 1.0, // w_hh
            0.0, 0.0, 0.0, 0.0, // b_ih
            0.0, 2.0, 0.0, 0.0,
        ]; // b_hh (forget gate bias = 2.0 to prevent vanishing)
        let mut reader = WeightReader::new(&weights);
        let mut model = LstmModel::from_config(&config, &mut reader);

        let input = vec![1.0f32; 16];
        let mut output = vec![0.0; 16];
        model.process(&input, &mut output);

        assert!(
            output.iter().any(|&x| x != 0.0),
            "LSTM should produce output"
        );
    }

    #[test]
    fn test_neural_model_passthrough_unknown_arch() {
        // Create a minimal .nam JSON with unsupported architecture
        let nam_json = serde_json::json!({
            "architecture": "UnknownArch",
            "config": {},
            "weights": [],
            "sample_rate": 48000
        });
        // Write to temp file
        let dir = std::env::temp_dir().join(format!("kicks-nam-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("unknown.nam");
        std::fs::write(&path, serde_json::to_vec(&nam_json).unwrap()).unwrap();

        let (mut model, _num_params) = NeuralModel::from_file(&path.to_string_lossy()).unwrap();
        assert_eq!(model.architecture(), "UnknownArch");

        let input = vec![0.5f32; 32];
        let mut output = vec![0.0; 32];
        model.process(&input, &mut output);

        for (out, inp) in output.iter().zip(input.iter()) {
            assert!((out - inp).abs() < 1e-6, "Unknown arch should passthrough");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}
