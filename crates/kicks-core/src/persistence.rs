// Kicks — Disk Persistence
//
// JSON save/load utilities for presets, config, and other persistent state.
// All files live under $XDG_CONFIG_HOME/kicks/ (default ~/.config/kicks/).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::config::KicksConfig;
use crate::midi::MidiConfig;
use crate::preset::PresetCollection;
use crate::scene::SceneCollection;

// ── Paths ────────────────────────────────────────────────────────────────────

/// Get the Kicks config directory (XDG-compliant).
pub fn config_dir() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
            PathBuf::from(home).join(".config")
        });
    base.join("kicks")
}

/// Path to the presets JSON file.
pub fn presets_path() -> PathBuf {
    config_dir().join("presets.json")
}

/// Path to the config JSON file.
pub fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

/// Path to the signal chain JSON file (auto-saved on changes).
pub fn signal_chain_path() -> PathBuf {
    config_dir().join("signal_chain.json")
}

/// Ensure the config directory exists, creating it if necessary.
pub fn ensure_config_dir() -> Result<()> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create config directory: {}", dir.display()))
}

// ── Generic JSON helpers ─────────────────────────────────────────────────────

/// Write `bytes` to `path`, readable only by the current user on Unix
/// (mode 0600). Config files can hold the AI API key, so they must not be
/// world-readable.
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path)?;
    // `mode` only applies on create; tighten a pre-existing file too.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    f.write_all(bytes)?;
    f.sync_all()
}

/// Save `data` as pretty-printed JSON to `path`.
pub fn save_json<T: Serialize>(path: &Path, data: &T) -> Result<()> {
    ensure_config_dir()?;
    let bytes = serde_json::to_vec_pretty(data)
        .with_context(|| format!("Failed to serialize data to {}", path.display()))?;
    // Atomic write: write to temp file, then rename
    let tmp = path.with_extension("tmp");
    write_private(&tmp, &bytes)
        .with_context(|| format!("Failed to write temp file {}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("Failed to rename {} to {}", tmp.display(), path.display()))?;
    Ok(())
}

/// Load and deserialize `T` from a JSON file at `path`.
/// Returns `None` if the file doesn't exist (caller decides default behaviour).
pub fn load_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes =
        std::fs::read(path).with_context(|| format!("Failed to read {}", path.display()))?;
    let value = serde_json::from_slice(&bytes)
        .with_context(|| format!("Failed to parse JSON from {}", path.display()))?;
    Ok(Some(value))
}

// ── Presets ──────────────────────────────────────────────────────────────────

/// Save a preset collection to disk.
pub fn save_presets(presets: &PresetCollection) -> Result<()> {
    let path = presets_path();
    save_json(&path, presets)?;
    tracing::info!(
        "Saved {} presets across {} banks to {}",
        presets.banks.iter().map(|b| b.presets.len()).sum::<usize>(),
        presets.banks.len(),
        path.display()
    );
    Ok(())
}

/// Load a preset collection from disk.
/// Returns the default (empty) collection if no file exists yet.
pub fn load_presets() -> PresetCollection {
    let path = presets_path();
    match load_json::<PresetCollection>(&path) {
        Ok(Some(collection)) => {
            tracing::info!(
                "Loaded {} banks from {}",
                collection.banks.len(),
                path.display()
            );
            collection
        }
        Ok(None) => {
            tracing::info!("No presets file at {}, starting fresh", path.display());
            PresetCollection { banks: vec![] }
        }
        Err(e) => {
            tracing::warn!("Failed to load presets from {}: {}", path.display(), e);
            PresetCollection { banks: vec![] }
        }
    }
}

// ── Config ───────────────────────────────────────────────────────────────────

/// Save app config to disk.
pub fn save_config(config: &KicksConfig) -> Result<()> {
    let path = config_path();
    save_json(&path, config)?;
    tracing::info!("Saved config to {}", path.display());
    Ok(())
}

/// Load app config from disk.
/// Returns the default config if no file exists yet.
pub fn load_config() -> KicksConfig {
    let path = config_path();
    match load_json::<KicksConfig>(&path) {
        Ok(Some(config)) => {
            tracing::info!("Loaded config from {}", path.display());
            config
        }
        Ok(None) => {
            tracing::info!("No config file at {}, using defaults", path.display());
            KicksConfig::default()
        }
        Err(e) => {
            tracing::warn!("Failed to load config from {}: {}", path.display(), e);
            KicksConfig::default()
        }
    }
}

// ── MIDI Config ──────────────────────────────────────────────────────────────

/// Path to the MIDI config JSON file.
pub fn midi_config_path() -> PathBuf {
    config_dir().join("midi_config.json")
}

/// Save MIDI config to disk.
pub fn save_midi_config(config: &MidiConfig) -> Result<()> {
    let path = midi_config_path();
    save_json(&path, config)?;
    tracing::info!("Saved MIDI config to {}", path.display());
    Ok(())
}

/// Load MIDI config from disk.
/// Returns the default config if no file exists yet.
pub fn load_midi_config() -> MidiConfig {
    let path = midi_config_path();
    match load_json::<MidiConfig>(&path) {
        Ok(Some(config)) => {
            tracing::info!("Loaded MIDI config from {}", path.display());
            config
        }
        Ok(None) => {
            tracing::info!("No MIDI config at {}, using defaults", path.display());
            MidiConfig::default()
        }
        Err(e) => {
            tracing::warn!("Failed to load MIDI config from {}: {}", path.display(), e);
            MidiConfig::default()
        }
    }
}

// ── Scenes (live mode) ───────────────────────────────────────────────────────

/// Path to the scenes JSON file.
pub fn scenes_path() -> PathBuf {
    config_dir().join("scenes.json")
}

/// Save scenes to disk.
pub fn save_scenes(scenes: &SceneCollection) -> Result<()> {
    let path = scenes_path();
    save_json(&path, scenes)?;
    tracing::info!("Saved {} scenes to {}", scenes.len(), path.display());
    Ok(())
}

/// Load scenes from disk.
/// Returns default (empty) collection if no file exists.
pub fn load_scenes() -> SceneCollection {
    let path = scenes_path();
    match load_json::<SceneCollection>(&path) {
        Ok(Some(collection)) => {
            tracing::info!("Loaded {} scenes from {}", collection.len(), path.display());
            collection
        }
        Ok(None) => {
            tracing::info!("No scenes file at {}, starting fresh", path.display());
            SceneCollection::new()
        }
        Err(e) => {
            tracing::warn!("Failed to load scenes from {}: {}", path.display(), e);
            SceneCollection::new()
        }
    }
}

// ── Signal Chain (auto-save) ─────────────────────────────────────────────────

/// Save a signal chain to disk (auto-saved on every change).
pub fn save_signal_chain(chain: &crate::signal_chain::SignalChain) -> Result<()> {
    let path = signal_chain_path();
    save_json(&path, chain)?;
    Ok(())
}

/// Load signal chain from disk.
/// Falls back to the default chain if no file exists.
pub fn load_signal_chain() -> crate::signal_chain::SignalChain {
    let path = signal_chain_path();
    match load_json::<crate::signal_chain::SignalChain>(&path) {
        Ok(Some(chain)) => {
            tracing::info!("Loaded signal chain from {}", path.display());
            chain
        }
        Ok(None) => {
            tracing::info!("No signal chain file at {}, using defaults", path.display());
            crate::signal_chain::SignalChain::default()
        }
        Err(e) => {
            tracing::warn!("Failed to load signal chain from {}: {}", path.display(), e);
            crate::signal_chain::SignalChain::default()
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {

    #[cfg(unix)]
    #[test]
    fn write_private_is_owner_only_even_over_existing_file() {
        use std::os::unix::fs::PermissionsExt;
        let p = std::env::temp_dir().join(format!("kicks-priv-{}.json", std::process::id()));
        std::fs::write(&p, b"old").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        super::write_private(&p, b"{\"ai_api_key\":\"k\"}").unwrap();
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(std::fs::read(&p).unwrap(), b"{\"ai_api_key\":\"k\"}");
        let _ = std::fs::remove_file(&p);
    }

    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::preset::{Bank, Preset};
    use crate::signal_chain::SignalChain;

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn test_dir() -> PathBuf {
        let n = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("kicks-test-{}-{}", std::process::id(), n))
    }

    #[test]
    fn test_save_json_roundtrip() {
        let dir = test_dir();
        let path = dir.join("test.json");
        std::fs::create_dir_all(&dir).unwrap();

        let data = vec![1, 2, 3];
        save_json(&path, &data).unwrap();

        let loaded: Vec<i32> = load_json(&path).unwrap().unwrap();
        assert_eq!(loaded, vec![1, 2, 3]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_json_nonexistent() {
        let dir = test_dir();
        let path = dir.join("nonexistent.json");

        let result: Result<Option<Vec<i32>>> = load_json(&path);
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn test_save_load_presets_roundtrip() {
        let dir = test_dir();
        let path = dir.join("presets.json");
        std::fs::create_dir_all(&dir).unwrap();

        let mut collection = PresetCollection::default();
        collection.banks.push(Bank {
            name: "Test Bank".into(),
            presets: vec![Preset {
                name: "Test Preset".into(),
                description: "A test".into(),
                tags: vec!["test".into()],
                signal_chain: SignalChain::default(),
                created: chrono::Utc::now(),
                modified: chrono::Utc::now(),
            }],
        });

        save_json(&path, &collection).unwrap();
        let loaded: PresetCollection = load_json(&path).unwrap().unwrap();
        assert_eq!(loaded.banks.len(), 1);
        assert_eq!(loaded.banks[0].name, "Test Bank");
        assert_eq!(loaded.banks[0].presets[0].name, "Test Preset");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_save_load_config_roundtrip() {
        let dir = test_dir();
        let path = dir.join("config.json");
        std::fs::create_dir_all(&dir).unwrap();

        let config = KicksConfig {
            jack_client_name: "kicks-test".into(),
            ..KicksConfig::default()
        };

        save_json(&path, &config).unwrap();
        let loaded: KicksConfig = load_json(&path).unwrap().unwrap();
        assert_eq!(loaded.jack_client_name, "kicks-test");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_atomic_write() {
        let dir = test_dir();
        let path = dir.join("config.json");
        std::fs::create_dir_all(&dir).unwrap();

        let mut config = KicksConfig {
            jack_client_name: "first".into(),
            ..KicksConfig::default()
        };
        save_json(&path, &config).unwrap();

        config.jack_client_name = "second".into();
        save_json(&path, &config).unwrap();

        let loaded: KicksConfig = load_json(&path).unwrap().unwrap();
        assert_eq!(loaded.jack_client_name, "second");
        assert!(!path.with_extension("tmp").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_nonexistent_returns_none() {
        let dir = test_dir();
        // Don't create the file — should return None, not error
        let result: Result<Option<PresetCollection>> = load_json(&dir.join("no_such_file.json"));
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn test_path_defaults() {
        // These should return sensible paths without panicking
        let dir = config_dir();
        assert!(dir.ends_with("kicks"));

        let p = presets_path();
        assert!(p.to_string_lossy().contains("presets"));

        let c = config_path();
        assert!(c.to_string_lossy().contains("config"));

        let s = signal_chain_path();
        assert!(s.to_string_lossy().contains("signal_chain"));

        let m = midi_config_path();
        assert!(m.to_string_lossy().contains("midi_config"));
    }
}
