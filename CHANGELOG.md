# Changelog

All notable changes to Kicks Guitar Workstation will be documented in this file.

## [0.1.0] — 2025-05-31

### ✨ Features

- **Amp Modeling** — Built-in amp & cabinet simulation with boost, drive, EQ (bass/mid/treble), and master volume
- **IR Loader** — Load custom impulse responses (WAV) into the Cab slot for realistic cabinet simulation
- **NAM Models** — Load Neural Amp Modeler models for state-of-the-art neural amp emulation
- **Effects** — Delay and reverb with wet/dry mix and parameter control
- **AI Assistant** — Generate tones from text descriptions using Claude (Anthropic API) or OpenAI
- **MIDI Control** — Map CC controllers to any parameter, with learn mode for easy assignment
- **Live Scenes** — Save and switch between full signal chains instantly for gigging and recording
- **Presets** — Organize tones into banks with tags, descriptions, and searchable metadata
- **Tuner** — Built-in chromatic tuner with mute/passthrough
- **Metronome** — Tempo-synced click generator
- **Looper** — Record, overdub, undo, reverse, and half-speed looping
- **CPU Meter** — Real-time DSP load monitoring

### 🏗️ Architecture

- **Tauri 2** desktop shell with Rust backend and React frontend
- **Workspace crates** for clean separation:
  - `kicks-core` — Domain models, config, persistence, presets
  - `kicks-dsp` — Real-time audio DSP engine (CPAL + JACK backends)
  - `guitarix-rpc` — Guitarix integration via JSON-RPC 2.0
- **Plugin-based DSP** — Boost, Amp, Cab, BassAmp, Delay, Reverb, Output
- **FFT convolution** — Efficient IR loading with overlap-add
- **Ring-buffer I/O** — Lock-free real-time audio processing

### 🔧 Technical

- 50 Rust unit tests across all crates
- 44 frontend unit tests (Vitest)
- CI/CD with GitHub Actions: fmt, clippy, test, security audit, cross-platform releases
- `cargo-deny` license and dependency auditing
- Cross-platform targets: Linux (AppImage/deb), macOS (dmg), Windows (msi/exe)

### 📦 Dependencies

- Rust 1.77.2+
- Node.js 22+
- Tauri CLI 2.x
- JACK / PipeWire (Linux audio)

---

