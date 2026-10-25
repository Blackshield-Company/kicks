# Changelog

All notable changes to Kicks Guitar Workstation will be documented in this file.

## [Unreleased]

### Changed

- The minimum supported Rust version is now 1.90; the CI minimum-version check job builds with Rust 1.90.

## [0.2.0] — 2026-10-23

This release makes the real-time audio path safe, adds a full set of workstation pedals and DI recording, and tightens file access and the webview security model.

### Highlights

- Real-time-safe engine: no heap allocation on the audio thread, verified by a dedicated test.
- Audio I/O that works with real interfaces: device channel counts, input channel selection, buffer negotiation, xrun counters and latency reporting.
- Pedalboard edits, reordering and wet/dry now reach the running DSP.
- New pedals: noise gate, compressor, overdrive, parametric EQ, chorus, flanger, phaser and tremolo.
- Two-channel WAV recording (dry DI + processed) for re-amping.
- File access limited to folders approved in the native folder picker, which are now remembered across restarts.

### Added

- **Pedals** — Gate, Comp, Drive, EQ, Chorus, Flanger, Phaser and Tremolo. New pedals are added to existing saved chains bypassed, without changing the existing order or settings. (#33)
- **Recording** — Record DI + processed audio to a 32-bit float WAV in the Kicks config directory. Lock-free on the audio thread; dropped frames are counted rather than blocking audio. (#33)
- **Audio Routing panel** in Settings — backend (System or JACK/PipeWire-JACK), input device, input channel, output device and a live status line showing latency, xruns and input errors. (#31)
- **Input channel selection** for interfaces where the guitar is not on input 1. (#31)
- **`get_audio_status`** command reporting backend, estimated latency, frames per callback, xruns and input health. (#31)
- **Persistent library folders** — IR and NAM folders chosen in the native folder picker are remembered across restarts. (#34)

### Changed

- **Real-time audio path** — parameter changes use a fixed-size, allocation-free key; chain switches and IR loads are prepared off the engine lock and swapped in O(1); oversized callbacks are processed in chunks; a missed lock repeats the last block at reduced level instead of dropping to silence. (#30)
- **Audio I/O** — streams open with the device's channel count, buffer size is clamped to the device's supported range, output is pre-filled to avoid start-up underruns, and input failures are reported in the UI instead of producing silence. JACK now reports latency. (#31)
- **Pedalboard** — parameter, bypass and mix edits apply even while audio is running; reordering changes the running chain; undo, redo and reset resync the engine; wet/dry is implemented in the DSP. (#32)
- **MIDI** — CC messages are applied directly from the MIDI input thread to the parameter queue, so MIDI control no longer depends on the UI polling. MIDI remains CC-only. (#34)
- **Engine start** applies the saved slot order and wet/dry settings. (#33)
- **Locks** — a poisoned mutex is logged and recovered instead of making every later command panic. (#29)
- **README** updated to match current behaviour: Linux is the tested platform; macOS and Windows bundles are experimental. (#34)

### Fixed

- Settings failed to save in the desktop app because the frontend did not send audio backend and device fields. (#31)
- Several commands failed in the desktop app because arguments were sent in snake_case instead of the camelCase names Tauri 2 expects. (#32)
- JACK ignored parameter changes from the UI. (#30)
- `DelayLine::read` could index past the end of its buffer and panic in the audio callback. (#30)
- A deadlock in MIDI learn mode caused by re-locking the MIDI configuration. (#34)
- `get_parameter` could return a stale cached value instead of the live plugin value. (#33)

### Removed

- `guitarix-rpc` crate and the Guitarix engine mode / connection settings. They were never wired into the app. Saved configs using the Guitarix mode load as Internal.

### Security

- **File access scoping** — IR and NAM commands accept only files inside folders approved through the native folder picker, or files picked in the native file dialog. Paths are canonicalized, checked before opening, opened once without following symlinks, and validated on the open handle for type, size and location. Reads are capped (64 MiB IR, 256 MiB NAM). Values in Settings never grant access. (#34)
- **Folder grants** are stored in an owner-only file written only by native code, re-checked on every start, and protected by a cross-process lock. The filesystem root, the home folder and broad system folders cannot be granted. (#34)
- **Content Security Policy** — `'unsafe-eval'` removed; network access limited to Tauri IPC; `object-src 'none'`, `base-uri 'self'`, `form-action 'none'` and `frame-ancestors 'none'` added. (#34)
- **Config files** containing the AI API key are written owner-only (0600 on Unix). The key is not yet stored in the OS keyring. (#34)
- **Devtools** disabled in release builds. (#29)

### Build and CI

- `rust-toolchain.toml` pins the stable toolchain with rustfmt and clippy. (#29)
- CI runs on pull requests against any base branch. (#29)
- New Rust 1.89 minimum-version check job; the minimum supported Rust version is now 1.89. (#34)

### Known limitations

- Linux is the tested platform; macOS and Windows builds are experimental.
- The Flatpak/Snap (portal) folder picker has not been verified with persistent folder grants.
- No native PipeWire backend; PipeWire is supported through ALSA or pipewire-jack. JACK xrun notifications are not yet reported.
- Latency figures are estimates, not loopback measurements.

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

