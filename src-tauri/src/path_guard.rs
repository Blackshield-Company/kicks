//! Scoping for commands that read user files by path.
//!
//! The webview can call any registered command with any string, so file
//! commands must not trust the path they are given. A path is accepted only
//! if, after resolving symlinks and `..`, it is
//! - inside a directory picked in a native dialog this session, or
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

/// Files and directories the user picked through native dialogs this session.
#[derive(Default)]
pub struct PathGrants {
    picked: Mutex<HashSet<PathBuf>>,
    roots: Mutex<HashSet<PathBuf>>,
}

impl PathGrants {
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
        self.roots.lock_or_recover().insert(canonical.clone());
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
