//! Scoping for commands that read user files by path.
//!
//! The webview can call any registered command with any string, so file
//! commands must not trust the path they are given. A path is accepted only
//! if, after resolving symlinks and `..`, it is
//! - inside a directory picked in a native dialog (persisted across restarts), or
//! - a file the user picked in a native dialog this session (or a directory
//!   containing such a file, for scans),
//!
//! and, for files, has an expected extension and is under a size cap.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::sync::LockOrRecover;

/// Largest IR file we will read (a 10 s stereo 96 kHz f32 IR is ~7 MiB).
pub const MAX_IR_BYTES: u64 = 64 * 1024 * 1024;
/// Largest NAM model we will read.
pub const MAX_NAM_BYTES: u64 = 256 * 1024 * 1024;

/// Native picker grants, independent of frontend-writable settings.
/// Default is memory-only; production startup explicitly opens the native store.
#[derive(Default)]
pub struct PathGrants {
    picked: Mutex<HashSet<PathBuf>>,
    roots: Mutex<HashSet<PathBuf>>,
    store: Option<PathBuf>,
}

/// Read the native grant store and keep only entries that still exist, are
/// directories, and are already canonical (i.e. exactly what was granted).
fn load_valid_roots(store: &Path) -> HashSet<PathBuf> {
    let stored = match std::fs::read(store) {
        Ok(bytes) => match serde_json::from_slice::<Vec<PathBuf>>(&bytes) {
            Ok(paths) => paths,
            Err(e) => {
                tracing::warn!("Invalid directory grant store: {}", e);
                Vec::new()
            }
        },
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!("Could not read directory grant store: {}", e);
            }
            Vec::new()
        }
    };
    let mut roots = HashSet::new();
    for path in stored {
        match path.canonicalize() {
            Ok(canonical) if canonical.as_os_str() == path.as_os_str() && canonical.is_dir() => {
                roots.insert(canonical);
            }
            _ => tracing::warn!("Dropping invalid directory grant: {}", path.display()),
        }
    }
    roots
}

impl PathGrants {
    /// Load only unchanged canonical directory roots and rewrite the pruned store.
    /// This path is supplied by native startup code, never by an IPC argument.
    pub fn with_store(store: PathBuf) -> Self {
        let roots = load_valid_roots(&store);
        let grants = Self {
            picked: Mutex::default(),
            roots: Mutex::new(roots),
            store: Some(store),
        };
        if let Err(e) = grants.persist(&grants.roots.lock_or_recover()) {
            tracing::warn!("Could not rewrite directory grant store: {}", e);
        }
        grants
    }

    fn persist(&self, roots: &HashSet<PathBuf>) -> Result<(), String> {
        let Some(store) = &self.store else {
            return Ok(());
        };
        // Merge with roots another app instance may have written meanwhile,
        // re-validating them exactly like startup does.
        let mut merged = load_valid_roots(store);
        merged.extend(roots.iter().cloned());
        let mut paths: Vec<_> = merged.iter().collect();
        paths.sort();
        let bytes = serde_json::to_vec_pretty(&paths).map_err(|e| e.to_string())?;
        if let Some(parent) = store.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tmp = store.with_extension(format!("{}.{}.tmp", std::process::id(), n));
        kicks_core::persistence::write_private(&tmp, &bytes).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, store).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            e.to_string()
        })
    }

    /// Record a dialog-picked file. Ignored if it cannot be resolved.
    pub fn grant(&self, path: &Path) {
        if let Ok(p) = path.canonicalize() {
            self.picked.lock_or_recover().insert(p);
        }
    }

    /// Record a native directory-picker result, never a settings value.
    pub fn grant_directory(&self, path: &Path) -> Result<PathBuf, String> {
        let canonical = path.canonicalize().map_err(|e| e.to_string())?;
        if !canonical.is_dir() {
            return Err("Not a directory".to_string());
        }
        // Serialize mutation and persistence so concurrent picks cannot lose roots.
        let mut roots = self.roots.lock_or_recover();
        let mut updated = roots.clone();
        updated.insert(canonical.clone());
        self.persist(&updated)?;
        *roots = updated;
        Ok(canonical)
    }

    fn under_root(&self, path: &Path) -> bool {
        self.roots
            .lock_or_recover()
            .iter()
            .any(|root| path.starts_with(root))
    }

    fn has_file(&self, canonical: &Path) -> bool {
        self.picked.lock_or_recover().contains(canonical)
    }

    fn has_file_in(&self, canonical_dir: &Path) -> bool {
        self.picked
            .lock_or_recover()
            .iter()
            .any(|p| p.parent() == Some(canonical_dir))
    }
}

/// Validate a file path for reading. Returns the resolved path to open.
pub fn check_file(
    path: &str, grants: &PathGrants, exts: &[&str], max_bytes: u64,
) -> Result<PathBuf, String> {
    let canonical = Path::new(path)
        .canonicalize()
        .map_err(|_| format!("File not found: {path}"))?;
    let meta = std::fs::metadata(&canonical).map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err(format!("Not a file: {path}"));
    }
    let ext = canonical
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if !exts.contains(&ext.as_str()) {
        return Err(format!(
            "Unsupported file type (expected {}): {path}",
            exts.join(", ")
        ));
    }
    if meta.len() > max_bytes {
        return Err(format!(
            "File too large ({} bytes, limit {max_bytes}): {path}",
            meta.len()
        ));
    }
    if grants.has_file(&canonical) || grants.under_root(&canonical) {
        Ok(canonical)
    } else {
        Err(format!(
            "Access denied: {path} is outside the approved library folders. \
             Pick it with the file dialog or pick its folder with the native directory dialog."
        ))
    }
}

/// Validate a directory path for listing. Returns the resolved directory.
pub fn check_dir(dir: &str, grants: &PathGrants) -> Result<PathBuf, String> {
    let canonical = Path::new(dir)
        .canonicalize()
        .map_err(|_| format!("Directory '{dir}' does not exist"))?;
    if !canonical.is_dir() {
        return Err(format!("Not a directory: {dir}"));
    }
    if grants.under_root(&canonical) || grants.has_file_in(&canonical) {
        Ok(canonical)
    } else {
        Err(format!(
            "Access denied: {dir} is outside the approved library folders"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tmp(PathBuf);
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn tmp(name: &str) -> Tmp {
        let d = std::env::temp_dir().join(format!("kicks-pg-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("lib")).unwrap();
        std::fs::create_dir_all(d.join("outside")).unwrap();
        std::fs::write(d.join("lib/a.wav"), b"x").unwrap();
        std::fs::write(d.join("lib/notes.txt"), b"x").unwrap();
        std::fs::write(d.join("outside/b.wav"), b"x").unwrap();
        std::fs::write(d.join("outside/secret.wav"), b"x").unwrap();
        Tmp(d)
    }

    fn s(p: PathBuf) -> String {
        p.to_string_lossy().into_owned()
    }

    fn stored_roots(store: &Path) -> Vec<PathBuf> {
        serde_json::from_slice(&std::fs::read(store).unwrap()).unwrap()
    }

    #[test]
    fn picked_directory_survives_restart_but_picked_files_do_not() {
        let t = tmp("persist");
        let store = t.0.join("config/granted_dirs.json");
        let g = PathGrants::with_store(store.clone());
        let root = g.grant_directory(&t.0.join("lib")).unwrap();
        g.grant(&t.0.join("outside/b.wav"));
        assert_eq!(stored_roots(&store), vec![root]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&store).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        drop(g);
        let reloaded = PathGrants::with_store(store);
        assert!(check_dir(&s(t.0.join("lib")), &reloaded).is_ok());
        assert!(check_file(&s(t.0.join("lib/a.wav")), &reloaded, &["wav"], 10).is_ok());
        assert!(check_file(&s(t.0.join("outside/b.wav")), &reloaded, &["wav"], 10).is_err());
    }

    #[test]
    fn reload_prunes_deleted_and_replaced_directories() {
        let t = tmp("prune");
        let store = t.0.join("granted_dirs.json");
        let g = PathGrants::with_store(store.clone());
        g.grant_directory(&t.0.join("lib")).unwrap();
        g.grant_directory(&t.0.join("outside")).unwrap();
        std::fs::remove_dir_all(t.0.join("lib")).unwrap();
        std::fs::remove_dir_all(t.0.join("outside")).unwrap();
        std::fs::write(t.0.join("outside"), b"file").unwrap();
        let reloaded = PathGrants::with_store(store.clone());
        assert!(reloaded.roots.lock_or_recover().is_empty());
        assert!(stored_roots(&store).is_empty());
    }

    #[test]
    fn reload_rejects_noncanonical_entries_and_malformed_json() {
        let t = tmp("invalid-store");
        let store = t.0.join("granted_dirs.json");
        let noncanonical = t.0.join("lib/../outside");
        std::fs::write(&store, serde_json::to_vec(&vec![noncanonical]).unwrap()).unwrap();
        let g = PathGrants::with_store(store.clone());
        assert!(check_dir(&s(t.0.join("outside")), &g).is_err());
        assert!(stored_roots(&store).is_empty());
        for bytes in [b"not json".as_slice(), b"{}", b"[42]"] {
            std::fs::write(&store, bytes).unwrap();
            let g = PathGrants::with_store(store.clone());
            assert!(g.roots.lock_or_recover().is_empty());
            assert!(stored_roots(&store).is_empty());
        }
    }

    #[cfg(unix)]
    #[test]
    fn reload_rejects_a_granted_directory_replaced_by_a_symlink() {
        let t = tmp("retarget");
        let store = t.0.join("granted_dirs.json");
        let g = PathGrants::with_store(store.clone());
        g.grant_directory(&t.0.join("lib")).unwrap();
        std::fs::remove_dir_all(t.0.join("lib")).unwrap();
        std::os::unix::fs::symlink(t.0.join("outside"), t.0.join("lib")).unwrap();
        let g = PathGrants::with_store(store.clone());
        assert!(check_dir(&s(t.0.join("lib")), &g).is_err());
        assert!(check_dir(&s(t.0.join("outside")), &g).is_err());
        assert!(stored_roots(&store).is_empty());
    }

    #[test]
    fn second_instance_does_not_drop_roots_granted_by_first() {
        let t = tmp("two-instances");
        let store = t.0.join("granted_dirs.json");
        let a = PathGrants::with_store(store.clone());
        let b = PathGrants::with_store(store.clone());
        let lib = a.grant_directory(&t.0.join("lib")).unwrap();
        let out = b.grant_directory(&t.0.join("outside")).unwrap();
        let mut want = vec![lib, out];
        want.sort();
        assert_eq!(stored_roots(&store), want);
        let leftovers = std::fs::read_dir(&t.0)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .path()
                    .to_string_lossy()
                    .ends_with(".tmp")
            })
            .count();
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn failed_store_write_does_not_grant_access() {
        let t = tmp("write-failure");
        let g = PathGrants::with_store(t.0.join("lib/a.wav/granted_dirs.json"));
        assert!(g.grant_directory(&t.0.join("outside")).is_err());
        assert!(check_dir(&s(t.0.join("outside")), &g).is_err());
    }

    #[test]
    fn file_inside_library_is_allowed() {
        let t = tmp("in");
        let g = PathGrants::default();
        g.grant_directory(&t.0.join("lib")).unwrap();
        assert!(check_file(&s(t.0.join("lib/a.wav")), &g, &["wav"], 10).is_ok());
    }

    #[test]
    fn file_outside_library_is_denied_until_picked() {
        let t = tmp("out");
        let g = PathGrants::default();
        g.grant_directory(&t.0.join("lib")).unwrap();
        let p = s(t.0.join("outside/b.wav"));
        assert!(check_file(&p, &g, &["wav"], 10)
            .unwrap_err()
            .contains("Access denied"));
        g.grant(&t.0.join("outside/b.wav"));
        assert!(check_file(&p, &g, &["wav"], 10).is_ok());
        // Granting one file does not grant its siblings.
        let sib = s(t.0.join("outside/secret.wav"));
        assert!(check_file(&sib, &g, &["wav"], 10).is_err());
    }

    #[test]
    fn dot_dot_escape_is_denied() {
        let t = tmp("dotdot");
        let p = s(t.0.join("lib/../outside/b.wav"));
        let g = PathGrants::default();
        g.grant_directory(&t.0.join("lib")).unwrap();
        assert!(check_file(&p, &g, &["wav"], 10).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_out_of_library_is_denied() {
        let t = tmp("link");
        std::os::unix::fs::symlink(t.0.join("outside/b.wav"), t.0.join("lib/link.wav")).unwrap();
        let p = s(t.0.join("lib/link.wav"));
        let g = PathGrants::default();
        g.grant_directory(&t.0.join("lib")).unwrap();
        assert!(check_file(&p, &g, &["wav"], 10).is_err());
    }

    #[test]
    fn wrong_extension_and_oversize_are_rejected() {
        let t = tmp("ext");
        let g = PathGrants::default();
        g.grant_directory(&t.0.join("lib")).unwrap();
        let txt = s(t.0.join("lib/notes.txt"));
        assert!(check_file(&txt, &g, &["wav"], 10)
            .unwrap_err()
            .contains("Unsupported"));
        let wav = s(t.0.join("lib/a.wav"));
        assert!(check_file(&wav, &g, &["wav"], 0)
            .unwrap_err()
            .contains("too large"));
    }

    #[test]
    fn writable_settings_do_not_grant_access() {
        let t = tmp("settings");
        let mut config = kicks_core::config::KicksConfig::default();
        let g = PathGrants::default();
        config.ir_directories = vec!["/".into(), s(t.0.clone())];
        config.nam_directories = config.ir_directories.clone();
        let p = s(t.0.join("lib/a.wav"));
        assert!(check_file(&p, &g, &["wav"], 10).is_err());
        assert!(check_dir(&s(t.0.join("lib")), &g).is_err());
        g.grant_directory(&t.0.join("lib")).unwrap();
        assert!(check_file(&p, &g, &["wav"], 10).is_ok());
        // A new session never derives grants from the persisted settings.
        assert!(check_file(&p, &PathGrants::default(), &["wav"], 10).is_err());
    }

    #[test]
    fn directory_grants_reject_files_and_cover_descendants() {
        let t = tmp("nested");
        let g = PathGrants::default();
        assert!(g.grant_directory(&t.0.join("outside/b.wav")).is_err());
        assert!(check_file(&s(t.0.join("outside/b.wav")), &g, &["wav"], 10).is_err());
        std::fs::create_dir(t.0.join("lib/nested")).unwrap();
        std::fs::write(t.0.join("lib/nested/a.nam"), b"x").unwrap();
        g.grant_directory(&t.0.join("lib")).unwrap();
        assert!(check_dir(&s(t.0.join("lib/nested")), &g).is_ok());
        assert!(check_file(&s(t.0.join("lib/nested/a.nam")), &g, &["nam"], 10).is_ok());
    }

    #[test]
    fn dir_scan_scope() {
        let t = tmp("dir");
        let g = PathGrants::default();
        g.grant_directory(&t.0.join("lib")).unwrap();
        assert!(check_dir(&s(t.0.join("lib")), &g).is_ok());
        assert!(check_dir(&s(t.0.join("outside")), &g).is_err());
        assert!(check_dir("/etc", &g).is_err());
        g.grant(&t.0.join("outside/b.wav"));
        assert!(check_dir(&s(t.0.join("outside")), &g).is_ok());
    }
}
