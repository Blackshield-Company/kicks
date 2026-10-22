# Settings

The Settings page configures audio, engine, AI, and file directory preferences.
Changes are saved to `~/.config/kicks/config.json` and applied immediately.

## Audio Engine

| Setting | Options | Description |
|---------|---------|-------------|
| **Engine Mode** | Auto / Internal | Both use the native kicks-dsp engine |
| **JACK Client Name** | text | Client name used when connecting to JACK (JACK backend) |

## Audio Device

| Setting | Options | Description |
|---------|---------|-------------|
| **Sample Rate** | 44100 / 48000 / 96000 Hz | Audio processing sample rate. Higher = better quality, more CPU |
| **Buffer Size** | 64 / 128 / 256 / 512 / 1024 | Lower = less latency, more CPU. Start at 256 and adjust |
| **Device** | System Default or specific device | Audio output device from CPAL enumeration |

### Config Hot-Reload

Changing any audio setting while the engine is running **automatically restarts
the CPAL audio stream** with the new configuration. The engine state (current
parameters, IR, etc.) is preserved — no manual stop/start needed.

### Choosing Buffer Size

| Buffer | Latency | CPU Use | Best For |
|--------|---------|---------|----------|
| 64 | ~1.3ms | High | Recording, low-latency monitoring |
| 128 | ~2.7ms | Medium | Real-time playing |
| 256 | ~5.3ms | Low | General use (default) |
| 512 | ~10.7ms | Very Low | Heavy plugin chains |
| 1024 | ~21ms | Minimal | Mixing, high track counts |

## AI Tone Assistant

See the [AI Assistant guide](ai-assistant.md) for full details.

| Setting | Description |
|---------|-------------|
| **Provider** | Anthropic or OpenAI-compatible |
| **Endpoint URL** | API endpoint for the selected provider |
| **API Key** | Your API key (stored in config file) |
| **Model** | Model name string |

## File Directories

| Directory | Purpose |
|-----------|---------|
| **IR Directory** | Scanned for WAV impulse response files |
| **NAM Directory** | Scanned for Neural Amp Model captures |
| **Preset Directory** | Saved presets location (default: `~/.config/kicks/presets/`) |

## Persistence

All settings are stored in `~/.config/kicks/config.json`. The file is atomic
write — a crash during save won't corrupt it.
