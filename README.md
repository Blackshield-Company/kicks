![kicks](.github/preview.png)

# 🎸 Kicks Guitar Workstation

[![CI](https://github.com/synthalorian/kicks/actions/workflows/ci.yml/badge.svg)](https://github.com/synthalorian/kicks/actions/workflows/ci.yml)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE)

A modern, open-source guitar amp simulator and effects workstation built with **Tauri** (Rust) and **React**. Kicks combines real-time DSP, AI-powered tone generation, impulse response loading, NAM model support, MIDI control, and a scene-based live mode — all in a fast, native desktop app.

---

## ✨ Features

| Category | Features |
|----------|----------|
| **Amp Modeling** | Built-in amp & cabinet simulation with boost, drive, EQ (bass/mid/treble), and master volume |
| **IR Loader** | Load custom impulse responses (WAV) into the Cab slot for realistic cabinet simulation |
| **NAM Models** | Load [Neural Amp Modeler](https://github.com/sdatkinson/neural-amp-modeler) models for state-of-the-art neural amp emulation |
| **Effects** | Noise gate, compressor, parametric EQ, overdrive, chorus, flanger, phaser, tremolo, delay and reverb, each with bypass and wet/dry mix |
| **Recording** | Record a stereo WAV with the dry DI on the left channel and the processed signal on the right, for later re-amping |
| **AI Assistant** | Generate tones from text descriptions using Claude (Anthropic API) — "Give me a SRV-style Texas blues tone" |
| **MIDI Control** | Map CC controllers to any parameter, with learn mode. CCs are applied on the MIDI thread through the lock-free parameter queue, so control keeps working while the UI is busy. CC only (no program change or notes yet) |
| **Live Scenes** | Save and switch between full signal chains instantly for gigging and recording |
| **Presets** | Organize tones into banks with tags, descriptions, and searchable metadata |
| **Platforms** | Linux is the supported, tested platform (AppImage/deb; JACK or PipeWire/ALSA). The release job also produces macOS and Windows bundles, but they are untested and should be treated as experimental |

### Security notes

- File commands only read IR/NAM files inside the library folders set in Settings, or files you pick in the file dialog. Symlinks and `..` are resolved before the check.
- The webview runs under a strict Content Security Policy (no `eval`, no inline scripts, network limited to the Tauri IPC).
- The AI API key is stored in `config.json` in the Kicks config directory, which is written owner-only (mode 0600). It is not yet in the OS keyring.

---

## 🏗️ Architecture

```
kicks/
├── crates/
│   ├── kicks-core/      # Domain models, config, persistence, presets
│   ├── kicks-dsp/       # Real-time audio DSP engine (plugins, audio I/O, NAM, convolution)
├── src-tauri/           # Tauri application shell (commands, state management, menus)
├── frontend/            # React + Vite + Tailwind CSS + Zustand UI
└── .github/workflows/   # CI/CD: clippy, tests, security audit, cross-platform releases
```

---

## 🚀 Quick Start

### Prerequisites

- **Rust** (latest stable) — [rustup.rs](https://rustup.rs)
- **Node.js** (v20+) & **npm**
- **Tauri CLI** — `cargo install tauri-cli --version "^2"`
- **cargo-deny** (optional, for local security checks) — `cargo install cargo-deny --locked`
- **System dependencies**
  - Linux: `jackd`, `libjack-jackd2-dev`, `libwebkit2gtk-4.1-dev`
  - macOS: Xcode Command Line Tools
  - Windows: MSVC Build Tools

### Development

```bash
# 1. Clone
git clone https://github.com/synthalorian/kicks.git
cd kicks

# 2. Install frontend dependencies
cd frontend && npm install && cd ..

# 3. Run the Tauri dev server
cargo tauri dev
```

The app will open automatically. The frontend dev server runs on `http://localhost:5173`.

### Building

```bash
# Frontend only
cd frontend && npm run build

# Full Tauri app (native binary + bundles)
cargo tauri build
```

Built artifacts:
- **Binary:** `src-tauri/target/release/app`
- **Linux:** `src-tauri/target/release/bundle/appimage/*.AppImage`, `*.deb`
- **macOS:** `src-tauri/target/release/bundle/dmg/*.dmg`
- **Windows:** `src-tauri/target/release/bundle/msi/*.msi`, `*.exe`

---

## 🧪 Testing

```bash
# Rust unit tests
cargo test --all-features

# Clippy (zero warnings policy)
cargo clippy --all-targets --all-features -- -D warnings

# Frontend unit tests (Vitest)
cd frontend && npm run test

# Frontend E2E tests (Playwright)
cd frontend && npx playwright install  # one-time browser install
cd frontend && npm run test:e2e        # headless
cd frontend && npm run test:e2e:ui     # interactive UI mode

# Security audit
cargo audit
cargo deny check
```

---

## 📦 CI / CD

The [`.github/workflows/ci.yml`](.github/workflows/ci.yml) pipeline runs on every push and PR:

| Job | What it does |
|-----|-------------|
| **Quality** | `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test --all-features` |
| **Frontend** | `npm run lint`, `npm run test`, `npm run test:e2e`, `npm run build` |
| **Security** | `cargo audit` (vulnerability scanning), `cargo deny check` (license/duplicate scanning) |
| **Release** | Cross-platform matrix build: Linux, macOS (universal), Windows. Uploads artifacts to GitHub Releases. |

### Automated Dependency Updates

Dependabot is configured to open weekly grouped PRs for:
- **Cargo** crates (`.github/dependabot.yml`)
- **npm** packages (`frontend/`)

---

## 🎛️ DSP Engine

The real-time audio pipeline is built on **JACK** and **CPAL** with a plugin-based architecture:

| Plugin | Description |
|--------|-------------|
| `Input` | Audio input with pass-through level control |
| `Boost` | Clean gain boost stage |
| `Amp` | Tube amp simulation with preamp gain, master volume, 3-band EQ, drive |
| `Cab` | Cabinet simulation with IR convolution, low/high cut filters |
| `BassAmp` | Dedicated bass amp with extended low-end response |
| `Delay` | Digital delay with time, feedback, and mix controls |
| `Reverb` | Room reverb with size, damping, and mix controls |
| `Output` | Master volume and output level |

**Audio I/O:** JACK (Linux, pro-audio) and CPAL (PipeWire/ALSA on Linux; CoreAudio/WASAPI backends compile but are untested)

---

## 🧠 AI Tone Assistant

Kicks integrates with the **Anthropic Claude** API to generate complete signal chains from natural language descriptions:

1. Enter a tone description in the **AI Assistant** panel
2. The backend sends it to Claude with a structured prompt
3. Claude returns parameter values for each slot in the signal chain
4. Apply the generated preset with one click

Configure your API key in **Settings → AI Provider**.

---

## 🤝 Contributing

We welcome contributions! Please:

1. **Open an issue** first for significant changes or new features
2. **Fork & branch** — `git checkout -b feature/your-feature-name`
3. **Write tests** for new DSP plugins, UI components, and Tauri commands
4. **Run the full validation suite** before submitting:
   ```bash
   cargo clippy --all-targets --all-features -- -D warnings
   cargo test --all-features
   cd frontend && npm run lint && npm run test && npm run test:e2e
   ```
5. **Submit a PR** with a clear description and linked issue

### Code Style

- **Rust:** `cargo fmt` + `cargo clippy -- -D warnings`
- **TypeScript:** ESLint + Prettier (via `npm run lint`)
- **Commits:** Clear, imperative mood (`Add delay plugin`, not `Added delay plugin`)

---

## 📄 License

Apache License 2.0 — see [LICENSE](LICENSE) for details.

---

## 🙏 Acknowledgments

- [Tauri](https://tauri.app/) — Secure, lightweight desktop apps with web tech
- [Neural Amp Modeler](https://github.com/sdatkinson/neural-amp-modeler) — Open-source neural amp modeling
- [JACK Audio Connection Kit](https://jackaudio.org/) — Professional low-latency audio
- [Tailwind CSS](https://tailwindcss.com/) — Utility-first CSS framework

---

## 🚀 Releasing

To create a new release and trigger the cross-platform build pipeline:

```bash
# Create an annotated tag
git tag -a v0.2.0 -m "Kicks v0.2.0"

# Push the tag to GitHub
git push origin v0.2.0
```

The CI `release` job will automatically build and upload `.AppImage`, `.deb`, `.dmg`, `.msi`, and `.exe` bundles to the GitHub Release page.

---


