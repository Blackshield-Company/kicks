// Kicks — Audio I/O Backend
//
// Abstracts real-time audio I/O behind CPAL (cross-platform) or JACK
// (pro-audio, Linux/PipeWire). JACK is the recommended backend for
// guitar work because it exposes named ports in qpwgraph / Catia.
//
// Real-time safety:
//   - Parameter changes are sent via a lock-free SPSC ring buffer so the main
//     thread never holds the engine mutex for `set_parameter`.
//   - The audio callback still uses `try_lock` on the engine, but contention
//     is now extremely rare (only non-parameter ops like IR loading or engine
//     start/stop hold the lock).

use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};

use crate::engine::{AudioEngine, KicksEngine};

// ═══════════════════════════════════════════════════════════════════════════════
// AudioConfig
// ═══════════════════════════════════════════════════════════════════════════════

/// Configuration for CPAL audio I/O.
#[derive(Debug, Clone)]
pub struct AudioConfig {
    /// Sample rate in Hz (default: 48000).
    pub sample_rate: f64,
    /// Buffer size in frames (default: 256).
    pub buffer_size: u32,
    /// Optional output device name. If None, uses default.
    pub output_device: Option<String>,
    /// Optional input device name. If None, uses default.
    pub input_device: Option<String>,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48000.0,
            buffer_size: 256,
            output_device: None,
            input_device: None,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// CpalAudioIO — Cross-platform audio via CPAL
// ═══════════════════════════════════════════════════════════════════════════════

/// CPAL-based audio I/O backend.
///
/// Opens full-duplex input+output streams on the calling thread.
/// The engine is shared via `Arc<Mutex<KicksEngine>>` so the main thread
/// can control the engine while audio callbacks process in real-time.
///
/// Parameter changes are lock-free: `start()` accepts the consumer end of an
/// SPSC channel (`ParamConsumer`), which the output callback drains before
/// each `process_all` call.  The producer lives in `AppState` so the main
/// thread never needs to lock the engine mutex for parameter changes.
///
/// Audio flow: Input callback → ring buffer → Output callback → KicksEngine → speakers
///
/// Requires CPAL 0.17+ (Stream is Send+Sync across all platforms).
#[derive(Default)]
pub struct CpalAudioIO {
    #[cfg(feature = "cpal-backend")]
    _input_stream: Option<cpal::Stream>,
    #[cfg(feature = "cpal-backend")]
    _output_stream: Option<cpal::Stream>,
    config: AudioConfig,
    active: bool,
}

impl CpalAudioIO {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start full-duplex audio I/O with the given engine and config.
    ///
    /// Opens input+output CPAL streams on the current default devices.
    ///
    /// `param_rx` is the consumer end of a lock-free SPSC channel. The output
    /// callback drains it before each `process_all` cycle, applying pending
    /// parameter changes to the engine.  Because the main thread pushes to
    /// the producer side (never locking the engine mutex), `try_lock` in the
    /// callback almost always succeeds.
    ///
    /// `cpu_load` is an optional atomic counter for DSP CPU usage monitoring.
    /// When provided, the callback stores (percentage * 1000) on each cycle.
    pub fn start(
        &mut self, engine: Arc<Mutex<KicksEngine>>, config: AudioConfig,
        param_rx: crate::param::ParamConsumer, cpu_load: Option<Arc<std::sync::atomic::AtomicU64>>,
    ) -> Result<()> {
        #[cfg(feature = "cpal-backend")]
        {
            self.stop();
            self.config = config.clone();

            use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
            use ringbuf::traits::{Consumer, Producer, Split};
            use ringbuf::HeapRb;

            let host = cpal::default_host();

            let output_device = config
                .output_device
                .as_ref()
                .and_then(|name| {
                    #[allow(deprecated)]
                    host.output_devices()
                        .ok()?
                        .find(|d| d.name().map(|n| n == *name).unwrap_or(false))
                })
                .or_else(|| host.default_output_device())
                .context("No audio output device found")?;

            let input_device = config
                .input_device
                .as_ref()
                .and_then(|name| {
                    #[allow(deprecated)]
                    host.input_devices()
                        .ok()?
                        .find(|d| d.name().map(|n| n == *name).unwrap_or(false))
                })
                .or_else(|| host.default_input_device());

            let sr = config.sample_rate as u32;
            let bs = config.buffer_size;

            let stream_config = cpal::StreamConfig {
                channels: 2,
                sample_rate: sr,
                buffer_size: cpal::BufferSize::Fixed(bs),
            };

            let err_fn = |err| tracing::error!("CPAL stream error: {}", err);

            // Ring buffer: input callback → output callback (mono)
            let ring_capacity = bs as usize * 8;
            let ring = HeapRb::<f32>::new(ring_capacity);
            let (producer, mut consumer) = ring.split();

            // ── Input stream (optional — only if device exists) ──
            let input_stream = input_device.and_then(|dev| {
                let mut prod = producer;
                dev.build_input_stream::<f32, _, _>(
                    &stream_config,
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        // CPAL gives us interleaved stereo; convert to mono by taking left channel
                        for frame in data.chunks(2) {
                            let mono = frame.first().copied().unwrap_or(0.0);
                            let _ = prod.try_push(mono);
                        }
                    },
                    err_fn,
                    None,
                )
                .ok()
            });

            if let Some(ref stream) = input_stream {
                if let Err(e) = stream.play() {
                    tracing::warn!("Failed to start input stream: {}", e);
                }
            }

            // ── Output stream ──
            // Pre-allocated input buffer avoids allocation in audio callback
            let mut input_buf = vec![0.0f32; bs as usize];
            let mut output_buf = vec![0.0f32; bs as usize];
            let eng = engine;
            let mut param_rx = param_rx;

            let output_stream = output_device
                .build_output_stream::<f32, _, _>(
                    &stream_config,
                    move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                        // Drain ring buffer into mono input_buf
                        for input_slot in input_buf.iter_mut() {
                            *input_slot = consumer.try_pop().unwrap_or(0.0);
                        }

                        let start = std::time::Instant::now();

                        // The main thread only holds the engine lock for O(1)
                        // swaps (chains/IRs are built off-lock), so try_lock
                        // almost always succeeds. If it fails, the previous
                        // block is repeated at reduced level rather than a hard
                        // dropout to silence.
                        if let Ok(mut eng) = eng.try_lock() {
                            eng.process_block(&mut param_rx, &input_buf, &mut output_buf);
                        } else {
                            for s in output_buf.iter_mut() {
                                *s *= 0.5;
                            }
                        }

                        let elapsed = start.elapsed().as_secs_f64();
                        let budget = bs as f64 / sr as f64;
                        let pct = ((elapsed / budget) * 1000.0).min(100_000.0) as u64;
                        if let Some(ref atomic) = cpu_load {
                            atomic.store(pct, std::sync::atomic::Ordering::Relaxed);
                        }

                        // Duplicate mono output to stereo interleaved
                        for (frame, &mono) in data.chunks_mut(2).zip(output_buf.iter()) {
                            frame[0] = mono;
                            if let Some(right) = frame.get_mut(1) {
                                *right = mono;
                            }
                        }
                    },
                    |err| tracing::error!("CPAL output stream error: {}", err),
                    None,
                )
                .context("Failed to build output audio stream")?;

            output_stream
                .play()
                .context("Failed to start output audio stream")?;

            self._input_stream = input_stream;
            self._output_stream = Some(output_stream);
            self.active = true;

            tracing::info!(
                "CPAL audio started: {} Hz, buffer {} frames, stereo",
                sr,
                bs
            );
        }

        #[cfg(not(feature = "cpal-backend"))]
        {
            let _ = (engine, config, param_rx);
            tracing::warn!("CPAL backend not compiled in; audio I/O disabled");
        }

        Ok(())
    }

    /// Stop audio I/O by dropping stream handles.
    pub fn stop(&mut self) {
        #[cfg(feature = "cpal-backend")]
        {
            self._output_stream = None;
            self._input_stream = None;
            tracing::info!("CPAL audio stopped");
        }
        self.active = false;
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn config(&self) -> &AudioConfig {
        &self.config
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Device enumeration
// ═══════════════════════════════════════════════════════════════════════════════

/// Information about a single audio device discovered on the system.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DeviceInfo {
    pub name: String,
    pub is_input: bool,
    pub is_output: bool,
}

/// Enumerate available audio devices via CPAL.
///
/// Returns a list of `DeviceInfo` entries.  When the `cpal-backend` feature
/// is disabled, returns an empty vec.
pub fn list_audio_devices() -> Vec<DeviceInfo> {
    #[cfg(feature = "cpal-backend")]
    {
        use cpal::traits::{DeviceTrait, HostTrait};
        let host = cpal::default_host();
        let mut devices = Vec::new();
        if let Ok(devs) = host.devices() {
            for dev in devs {
                let name = dev
                    .description()
                    .map(|d| d.to_string())
                    .unwrap_or_else(|_| String::new());
                let is_input = dev
                    .supported_input_configs()
                    .map(|c| c.count() > 0)
                    .unwrap_or(false);
                let is_output = dev
                    .supported_output_configs()
                    .map(|c| c.count() > 0)
                    .unwrap_or(false);
                devices.push(DeviceInfo {
                    name,
                    is_input,
                    is_output,
                });
            }
        }
        devices
    }
    #[cfg(not(feature = "cpal-backend"))]
    {
        vec![]
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// JackAudioIO — JACK backend (feature-gated)
// ═══════════════════════════════════════════════════════════════════════════════

/// Configuration for JACK audio I/O.
#[derive(Debug, Clone)]
pub struct JackConfig {
    pub client_name: String,
}

impl Default for JackConfig {
    fn default() -> Self {
        Self {
            client_name: "Kicks".to_string(),
        }
    }
}

/// Custom ProcessHandler for Kicks JACK backend.
/// Holds the engine Arc, CPU load atomic, and port references so the process
/// callback can read/write audio buffers.
#[cfg(feature = "jack-backend")]
pub struct JackProcessHandler {
    engine: Arc<Mutex<KicksEngine>>,
    cpu_load: Option<Arc<std::sync::atomic::AtomicU64>>,
    sample_rate: f64,
    in_l: jack::Port<jack::AudioIn>,
    in_r: jack::Port<jack::AudioIn>,
    out_l: jack::Port<jack::AudioOut>,
    out_r: jack::Port<jack::AudioOut>,
    param_rx: Option<crate::param::ParamConsumer>,
    mono_in: Vec<f32>,
    mono_out: Vec<f32>,
}

#[cfg(feature = "jack-backend")]
impl jack::ProcessHandler for JackProcessHandler {
    fn process(&mut self, _client: &jack::Client, ps: &jack::ProcessScope) -> jack::Control {
        let start = std::time::Instant::now();

        let in_l_buf = self.in_l.as_slice(ps);
        let in_r_buf = self.in_r.as_slice(ps);
        let out_l_buf = self.out_l.as_mut_slice(ps);
        let out_r_buf = self.out_r.as_mut_slice(ps);

        let n = ps.n_frames() as usize;

        // Reuse the activation-time scratch buffers even if JACK grows its period.
        process_jack_period(
            [in_l_buf, in_r_buf],
            [out_l_buf, out_r_buf],
            [&mut self.mono_in, &mut self.mono_out],
            |mono_in, mono_out| {
                if let Ok(mut eng) = self.engine.try_lock() {
                    match self.param_rx.as_mut() {
                        Some(rx) => eng.process_block(rx, mono_in, mono_out),
                        None => {
                            if eng.process(mono_in, mono_out).is_err() {
                                mono_out.fill(0.0);
                            }
                        }
                    }
                } else {
                    mono_out.fill(0.0);
                }
            },
        );

        // CPU load tracking
        let elapsed = start.elapsed().as_secs_f64();
        let budget = n as f64 / self.sample_rate;
        let pct = ((elapsed / budget) * 1000.0).min(100_000.0) as u64;
        if let Some(ref atomic) = self.cpu_load {
            atomic.store(pct, std::sync::atomic::Ordering::Relaxed);
        }

        jack::Control::Continue
    }
}

/// Process every frame using bounded, preallocated mono scratch buffers.
#[cfg(feature = "jack-backend")]
fn process_jack_period(
    input: [&[f32]; 2], output: [&mut [f32]; 2], scratch: [&mut [f32]; 2],
    mut process: impl FnMut(&[f32], &mut [f32]),
) {
    let [mono_in, mono_out] = scratch;
    let capacity = mono_in.len().min(mono_out.len());
    let [out_l, out_r] = output;
    if capacity == 0 {
        out_l.fill(0.0);
        out_r.fill(0.0);
        return;
    }
    for start in (0..input[0].len()).step_by(capacity) {
        let end = (start + capacity).min(input[0].len());
        let n = end - start;
        for (i, sample) in mono_in[..n].iter_mut().enumerate() {
            *sample = (input[0][start + i] + input[1][start + i]) * 0.5;
        }
        process(&mono_in[..n], &mut mono_out[..n]);
        out_l[start..end].copy_from_slice(&mono_out[..n]);
        out_r[start..end].copy_from_slice(&mono_out[..n]);
    }
}

#[cfg(all(test, feature = "jack-backend"))]
mod tests {
    use super::process_jack_period;

    #[test]
    fn jack_period_processes_all_frames_including_partial_chunk() {
        let input_l = [1.0, 2.0, 3.0, 4.0, 5.0];
        let input_r = [3.0, 4.0, 5.0, 6.0, 7.0];
        let mut out_l = [f32::NAN; 5];
        let mut out_r = [f32::NAN; 5];
        let mut mono_in = [0.0; 2];
        let mut mono_out = [0.0; 2];
        let mut frames = 0;
        process_jack_period(
            [&input_l, &input_r],
            [&mut out_l, &mut out_r],
            [&mut mono_in, &mut mono_out],
            |input, output| {
                frames += input.len();
                for (src, dst) in input.iter().zip(output) {
                    *dst = src * 2.0;
                }
            },
        );
        assert_eq!(frames, 5);
        assert_eq!(out_l, [4.0, 6.0, 8.0, 10.0, 12.0]);
        assert_eq!(out_r, out_l);
    }
}

/// A real JACK audio client.
///
/// Registers stereo input ports (`in_l`, `in_r`) and stereo output ports
/// (`out_l`, `out_r`) so the app appears as "Kicks" in qpwgraph / Catia.
/// The process callback runs the DSP engine on every JACK cycle.
///
/// Usage:
/// ```ignore
/// let mut jack = JackAudioIO::new(JackConfig::default());
/// jack.start(engine, cpu_load)?;
/// // ... app runs ...
/// jack.stop()?;
/// ```
pub struct JackAudioIO {
    #[cfg(feature = "jack-backend")]
    active_client: Option<jack::AsyncClient<(), JackProcessHandler>>,
    #[cfg(feature = "jack-backend")]
    _input_l: Option<jack::Port<jack::AudioIn>>,
    #[cfg(feature = "jack-backend")]
    _input_r: Option<jack::Port<jack::AudioIn>>,
    #[cfg(feature = "jack-backend")]
    _output_l: Option<jack::Port<jack::AudioOut>>,
    #[cfg(feature = "jack-backend")]
    _output_r: Option<jack::Port<jack::AudioOut>>,

    config: JackConfig,
    active: bool,
}

impl JackAudioIO {
    pub fn new(config: JackConfig) -> Self {
        Self {
            #[cfg(feature = "jack-backend")]
            active_client: None,
            #[cfg(feature = "jack-backend")]
            _input_l: None,
            #[cfg(feature = "jack-backend")]
            _input_r: None,
            #[cfg(feature = "jack-backend")]
            _output_l: None,
            #[cfg(feature = "jack-backend")]
            _output_r: None,
            config,
            active: false,
        }
    }

    /// Start the JACK client: register ports, activate, and begin processing.
    ///
    /// `engine` is the DSP engine shared with the process thread.
    /// `cpu_load` is an optional atomic for monitoring DSP load.
    pub fn start(
        &mut self, engine: Arc<Mutex<KicksEngine>>, param_rx: crate::param::ParamConsumer,
        cpu_load: Option<Arc<std::sync::atomic::AtomicU64>>,
    ) -> Result<()> {
        #[cfg(feature = "jack-backend")]
        {
            self.stop();

            let (client, _status) = jack::Client::new(
                &self.config.client_name,
                jack::ClientOptions::NO_START_SERVER,
            )
            .map_err(|e| anyhow::anyhow!("Failed to create JACK client: {:?}", e))?;

            let sample_rate = client.sample_rate() as f64;
            let buffer_size = client.buffer_size();

            // Register stereo input ports
            let in_l = client
                .register_port("in_l", jack::AudioIn)
                .context("Failed to register JACK input port in_l")?;
            let in_r = client
                .register_port("in_r", jack::AudioIn)
                .context("Failed to register JACK input port in_r")?;

            // Register stereo output ports
            let out_l = client
                .register_port("out_l", jack::AudioOut)
                .context("Failed to register JACK output port out_l")?;
            let out_r = client
                .register_port("out_r", jack::AudioOut)
                .context("Failed to register JACK output port out_r")?;

            // Initialize engine at JACK's sample rate / buffer size
            {
                let mut eng = engine.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
                eng.init(sample_rate, buffer_size)
                    .map_err(|e| anyhow::anyhow!("Engine init failed: {}", e))?;
            }

            let eng_process = engine.clone();

            let process = JackProcessHandler {
                engine: eng_process,
                cpu_load,
                sample_rate,
                in_l,
                in_r,
                out_l,
                out_r,
                param_rx: Some(param_rx),
                // Generous headroom: JACK periods rarely exceed 8192 frames.
                mono_in: vec![0.0; (buffer_size as usize).max(8192)],
                mono_out: vec![0.0; (buffer_size as usize).max(8192)],
            };

            let active_client = client
                .activate_async((), process)
                .map_err(|e| anyhow::anyhow!("Failed to activate JACK client: {:?}", e))?;

            self.active_client = Some(active_client);
            // Ports are owned by the JackProcessHandler inside AsyncClient now.
            // We don't store them separately.
            self.active = true;

            tracing::info!(
                "JACK client '{}' active: {} Hz, {} frames, ports: in_l, in_r, out_l, out_r",
                self.config.client_name,
                sample_rate,
                buffer_size
            );
        }

        #[cfg(not(feature = "jack-backend"))]
        {
            let _ = (engine, param_rx, cpu_load);
            tracing::warn!("JACK backend not compiled in; audio I/O disabled");
        }

        Ok(())
    }

    /// Stop the JACK client by deactivating and dropping it.
    pub fn stop(&mut self) {
        #[cfg(feature = "jack-backend")]
        {
            if let Some(client) = self.active_client.take() {
                let _ = client.deactivate();
                tracing::info!("JACK client '{}' deactivated", self.config.client_name);
            }
            self._input_l = None;
            self._input_r = None;
            self._output_l = None;
            self._output_r = None;
        }
        self.active = false;
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn config(&self) -> &JackConfig {
        &self.config
    }
}
