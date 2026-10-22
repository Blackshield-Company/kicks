# Kicks Architecture

## Crate Hierarchy

```
┌─────────────────────────────────────────────┐
│                 Tauri Shell                  │
│               src-tauri (app)                │
│   Window management, IPC commands, menu      │
└──────────────────────┬──────────────────────┘
                       ▼
        ┌──────────────────────────────┐
        │          kicks-dsp            │
        │  Real-time audio engine       │
        │  CPAL / JACK I/O, plugins,    │
        │  NAM, convolution, recorder   │
        └──────────────┬───────────────┘
                       ▼
        ┌──────────────────────────────┐
        │          kicks-core           │
        │  SignalChain, presets, MIDI   │
        │  config, app config           │
        └──────────────────────────────┘
```

## Engine

Kicks processes audio natively with the kicks-dsp crate: CPAL audio I/O
(ALSA/PipeWire on Linux) with embedded DSP plugins, and a JACK client as a
feature-gated alternative. The `Auto` and `Internal` engine modes both select
this engine. (An earlier Guitarix remote-control mode was never wired up and
has been removed.)

## Data Flow

```
User Input (UI)
     │
     ▼
Tauri IPC Commands (src-tauri/src/commands/)
     │
     └── kicks-dsp (CpalAudioIO / JackAudioIO)
                             │
                    Input callback → ringbuf
                             │
                    Output callback → KicksEngine.process()
                             │
                    Speakers (ALSA/PipeWire/CoreAudio)
```

## Frontend Architecture

The frontend is a React + TypeScript SPA rendered in a Tauri webview.

```
frontend/src/
├── components/       # Reusable UI components
│   ├── SignalChain/  # Visual chain editor
│   ├── Presets/      # Preset browser
│   ├── IRBrowser/    # File browser for IRs/NAMs
│   ├── MIDI/         # MIDI configuration
│   ├── Live/         # Performance mode
│   ├── AI/           # Tone assistant panel
│   └── Common/       # Shared UI (knobs, sliders, etc.)
├── hooks/            # React hooks for Tauri invoke()
├── stores/           # State management
├── types/            # TypeScript type definitions
├── App.tsx           # Main app layout
└── main.tsx          # Entry point
```

## Key Design Decisions

- **Rust + Tauri** over Electron for memory efficiency and audio performance
- **Workspace crates** for clear separation between concerns
- **GPLv3** to align with Guitarix and the audio FOSS ecosystem
- **CPAL** as primary audio backend (ALSA/PipeWire on Linux, CoreAudio on macOS, WASAPI on Windows); JACK still available as feature-gated alternative
- **Demo-depth MVP** — prove all features work, deepen incrementally
