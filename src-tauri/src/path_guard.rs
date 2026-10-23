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
        if is_valid_root(&path) {
            roots.insert(path);
        } else {
            tracing::warn!("Dropping invalid directory grant: {}", path.display());
        }
    }
    roots
}

/// Under an exclusive cross-process lock, re-validate the stored roots, add
/// `new_root` (already canonical) if given, and atomically rewrite the store.
/// Returns the roots that were written.
fn update_store(store: &Path, new_root: Option<&Path>) -> Result<HashSet<PathBuf>, String> {
    let err = |e: std::io::Error| e.to_string();
    if let Some(parent) = store.parent() {
        std::fs::create_dir_all(parent).map_err(err)?;
    }
    let lock = open_private(&store.with_extension("lock"), false).map_err(err)?;
    lock.lock().map_err(err)?;
    let mut roots = load_valid_roots(store);
    if let Some(root) = new_root {
        if !is_valid_root(root) {
            return Err("Directory changed before it could be granted".to_string());
        }
        roots.insert(root.to_path_buf());
    }
    let mut paths: Vec<_> = roots.iter().collect();
    paths.sort();
    let bytes = serde_json::to_vec_pretty(&paths).map_err(|e| e.to_string())?;
    write_atomic(store, &bytes).map_err(err)?;
    Ok(roots)
    // `lock` is released when dropped here.
}

fn paths_equal(a: &Path, b: &Path) -> bool {
    #[cfg(windows)]
    {
        a.as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&b.as_os_str().to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}

fn is_valid_root(path: &Path) -> bool {
    let Ok(canonical) = path.canonicalize() else {
        return false;
    };
    #[cfg(not(windows))]
    let broad = ["/usr", "/etc", "/var", "/tmp", "/home", "/Users"];
    #[cfg(windows)]
    let broad = [r"C:\", r"C:\Users", r"C:\Windows"];
    if path.parent().is_none()
        || dirs::home_dir()
            .and_then(|home| home.canonicalize().ok())
            .is_some_and(|home| home.ancestors().any(|p| paths_equal(p, &canonical)))
        || broad
            .iter()
            .filter_map(|p| Path::new(p).canonicalize().ok())
            // Reject a denylisted folder and anything containing one
            // (e.g. macOS `/private`, which holds `/private/tmp`).
            .any(|p| p.ancestors().any(|a| paths_equal(a, &canonical)))
    {
        return false;
    }
    paths_equal(&canonical, path) && canonical.is_dir()
}

fn open_private(path: &Path, create_new: bool) -> std::io::Result<std::fs::File> {
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true).write(true);
    if create_new {
        opts.create_new(true);
    } else {
        opts.create(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // O_NOFOLLOW: never follow a planted symlink at the lock/temp path.
        opts.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    opts.open(path)
}

/// Write to a fresh, exclusively created temp file, then rename over `store`.
/// The temp file is removed on every failure path.
fn write_atomic(store: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    struct Cleanup<'a>(&'a Path, bool);
    impl Drop for Cleanup<'_> {
        fn drop(&mut self) {
            if self.1 {
                let _ = std::fs::remove_file(self.0);
            }
        }
    }
    loop {
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).map_err(|e| std::io::Error::other(e.to_string()))?;
        let n = u128::from_ne_bytes(random);
        let tmp = store.with_extension(format!("{}.{}.tmp", std::process::id(), n));
        let mut f = match open_private(&tmp, true) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };
        let mut guard = Cleanup(&tmp, true);
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, store)?;
        guard.1 = false;
        // Directory fsync is Unix-only; Windows does not support this directory open/sync pattern.
        #[cfg(unix)]
        std::fs::File::open(store.parent().unwrap_or(Path::new(".")))?.sync_all()?;
        return Ok(());
    }
}

impl PathGrants {
    /// Load only unchanged canonical directory roots and rewrite the pruned store.
    /// This path is supplied by native startup code, never by an IPC argument.
    pub fn with_store(store: PathBuf) -> Self {
        let roots = match update_store(&store, None) {
            Ok(roots) => roots,
            Err(e) => {
                tracing::warn!("Could not rewrite directory grant store: {}", e);
                load_valid_roots(&store)
            }
        };
        Self {
            picked: Mutex::default(),
            roots: Mutex::new(roots),
            store: Some(store),
        }
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
        if !is_valid_root(&canonical) {
            return Err("Directory is too broad or is not a valid library directory".to_string());
        }
        let mut roots = self.roots.lock_or_recover();
        match &self.store {
            // The store is the source of truth: write (re-validated disk roots
            // + this one) under a cross-process lock, then adopt exactly that.
            // Stale in-memory roots are never written back.
            Some(store) => *roots = update_store(store, Some(&canonical))?,
            None => {
                roots.insert(canonical.clone());
            }
        }
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

/// Open once and validate that handle against the canonical, approved path.
pub fn check_file(
    path: &str, grants: &PathGrants, exts: &[&str], max_bytes: u64,
) -> Result<(PathBuf, std::fs::File), String> {
    let canonical = Path::new(path)
        .canonicalize()
        .map_err(|_| format!("File not found: {path}"))?;
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
    if !(grants.has_file(&canonical) || grants.under_root(&canonical)) {
        return Err(format!(
            "Access denied: {path} is outside the approved library folders. \
             Pick it with the file dialog or pick its folder with the native directory dialog."
        ));
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Avoid following a replaced leaf or blocking on a planted FIFO.
        // Regular files ignore O_NONBLOCK.
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(&canonical).map_err(|e| e.to_string())?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err(format!("Not a file: {path}"));
    }
    if meta.len() > max_bytes {
        return Err(format!(
            "File too large ({} bytes, limit {max_bytes}): {path}",
            meta.len()
        ));
    }
    validate_handle(&file, &canonical)?;
    // Recheck scope using the path proven to belong to the open handle.
    if !(grants.has_file(&canonical) || grants.under_root(&canonical)) {
        return Err("Access denied: file is outside the approved library folders".into());
    }
    Ok((canonical, file))
}

/// Reject replacement of the canonical path between resolution and opening.
fn validate_handle(file: &std::fs::File, canonical: &Path) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    let actual = {
        use std::os::fd::AsRawFd;
        std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd()))
            .map_err(|e| e.to_string())?
    };
    #[cfg(target_os = "macos")]
    let actual = {
        use std::os::{fd::AsRawFd, unix::ffi::OsStrExt};
        let mut buf = [0u8; libc::PATH_MAX as usize];
        // SAFETY: F_GETPATH writes at most PATH_MAX bytes into the supplied buffer.
        if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETPATH, buf.as_mut_ptr()) } == -1 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let end = buf
            .iter()
            .position(|b| *b == 0)
            .ok_or("Invalid handle path")?;
        PathBuf::from(std::ffi::OsStr::from_bytes(&buf[..end]))
    };
    #[cfg(windows)]
    let actual = {
        use std::os::windows::{ffi::OsStringExt, io::AsRawHandle};
        use windows_sys::Win32::Storage::FileSystem::GetFinalPathNameByHandleW;
        let mut buf = vec![0u16; 32768];
        // SAFETY: the handle is live and the buffer has the declared capacity.
        let len = unsafe {
            GetFinalPathNameByHandleW(file.as_raw_handle(), buf.as_mut_ptr(), buf.len() as u32, 0)
        };
        if len == 0 || len as usize >= buf.len() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        PathBuf::from(std::ffi::OsString::from_wide(&buf[..len as usize]))
    };
    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    if !paths_equal(&actual, canonical) {
        return Err("File changed while opening".into());
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        // Other platforms lack handle-path verification here; retain the identity check.
        use std::os::unix::fs::MetadataExt;
        let meta = file.metadata().map_err(|e| e.to_string())?;
        let current = std::fs::metadata(canonical).map_err(|e| e.to_string())?;
        if meta.dev() != current.dev()
            || meta.ino() != current.ino()
            || canonical.canonicalize().map_err(|e| e.to_string())? != canonical
        {
            return Err("File changed while opening".into());
        }
    }
    Ok(())
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
    #[ignore = "invoked by process stress test"]
    fn grant_process_child() {
        let Some(base) = std::env::var_os("KICKS_GRANT_CHILD") else {
            return;
        };
        let base = PathBuf::from(base);
        let index = std::env::var("KICKS_GRANT_INDEX").unwrap();
        while !base.join("start").exists() {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        for round in 0..6 {
            let g = PathGrants::with_store(base.join("granted_dirs.json"));
            g.grant_directory(&base.join(format!("d{index}-{round}")))
                .unwrap();
        }
    }

    #[test]
    fn overlapping_instances_never_lose_grants() {
        let t = tmp("process-stress");
        let mut want = Vec::new();
        for i in 0..8 {
            for round in 0..6 {
                let dir = t.0.join(format!("d{i}-{round}"));
                std::fs::create_dir(&dir).unwrap();
                want.push(dir.canonicalize().unwrap());
            }
        }
        let mut children: Vec<_> = (0..8)
            .map(|i| {
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "path_guard::tests::grant_process_child",
                        "--ignored",
                    ])
                    .env("KICKS_GRANT_CHILD", &t.0)
                    .env("KICKS_GRANT_INDEX", i.to_string())
                    .stdout(std::process::Stdio::null())
                    .spawn()
                    .unwrap()
            })
            .collect();
        std::fs::write(t.0.join("start"), b"go").unwrap();
        for child in &mut children {
            assert!(child.wait().unwrap().success());
        }
        want.sort();
        assert_eq!(stored_roots(&t.0.join("granted_dirs.json")), want);
    }

    #[test]
    fn broad_roots_are_rejected_on_load_and_grant() {
        let t = tmp("broad");
        let store = t.0.join("granted_dirs.json");
        let mut broad = vec![PathBuf::from("/")];
        if let Some(home) = dirs::home_dir().and_then(|h| h.canonicalize().ok()) {
            broad.extend(home.ancestors().map(Path::to_path_buf));
        }
        broad.extend(["/usr", "/etc", "/var", "/tmp"].map(PathBuf::from));
        std::fs::write(&store, serde_json::to_vec(&broad).unwrap()).unwrap();
        let g = PathGrants::with_store(store.clone());
        for path in broad {
            assert!(g.grant_directory(&path).is_err());
            assert!(PathGrants::default().grant_directory(&path).is_err());
        }
        assert!(stored_roots(&store).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn opened_handle_survives_replacement_and_mismatch_is_rejected() {
        use std::io::Read;
        let t = tmp("handle");
        let g = PathGrants::default();
        g.grant_directory(&t.0.join("lib")).unwrap();
        let (path, mut file) = check_file(&s(t.0.join("lib/a.wav")), &g, &["wav"], 10).unwrap();
        std::fs::rename(&path, t.0.join("lib/old.wav")).unwrap();
        std::fs::write(&path, b"replacement").unwrap();
        assert!(validate_handle(&file, &path).is_err());
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"x");
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(t.0.join("outside/b.wav"), &path).unwrap();
        assert!(validate_handle(&file, &path).is_err());
    }

    #[test]
    fn stale_memory_does_not_resurrect_a_root_pruned_elsewhere() {
        let t = tmp("stale");
        let store = t.0.join("granted_dirs.json");
        let a = PathGrants::with_store(store.clone());
        let lib = a.grant_directory(&t.0.join("lib")).unwrap();
        // Another instance prunes `lib` after it was replaced.
        std::fs::remove_dir_all(&lib).unwrap();
        let _b = PathGrants::with_store(store.clone());
        std::fs::create_dir_all(&lib).unwrap();
        // `a` still remembers `lib`, but its next write must not re-add it.
        let out = a.grant_directory(&t.0.join("outside")).unwrap();
        assert_eq!(stored_roots(&store), vec![out]);
        assert!(check_dir(&s(lib), &a).is_err());
    }

    #[test]
    fn failed_temp_write_leaves_no_temp_file() {
        let t = tmp("tmp-cleanup");
        let store = t.0.join("granted_dirs.json");
        // Make the rename fail after the temp file was written.
        std::fs::create_dir_all(store.join("blocker")).unwrap();
        assert!(write_atomic(&store, b"[]").is_err());
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
    fn parents_of_broad_directories_are_rejected() {
        for p in ["/usr", "/etc", "/var", "/tmp"] {
            if let Ok(c) = Path::new(p).canonicalize() {
                for a in c.ancestors() {
                    assert!(!is_valid_root(a), "{} accepted", a.display());
                }
            }
        }
    }

    #[test]
    fn failed_store_write_does_not_grant_access() {
        let t = tmp("write-failure");
        let g = PathGrants::with_store(t.0.join("lib/a.wav/granted_dirs.json"));
        assert!(g.grant_directory(&t.0.join("outside")).is_err());
        assert!(check_dir(&s(t.0.join("outside")), &g).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn fifo_is_rejected_without_blocking() {
        use std::os::unix::ffi::OsStrExt;
        let t = tmp("fifo");
        let path = t.0.join("lib/pipe.wav");
        let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        // SAFETY: name is a valid NUL-terminated path.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let g = PathGrants::default();
        g.grant_directory(&t.0.join("lib")).unwrap();
        assert!(check_file(&s(path), &g, &["wav"], 10)
            .unwrap_err()
            .contains("Not a file"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn out_of_scope_file_is_never_opened() {
        use std::io::Read;
        use std::os::fd::{AsRawFd, FromRawFd};
        use std::os::unix::ffi::OsStrExt;
        let t = tmp("no-open");
        let path = t.0.join("outside/b.wav");
        let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        // SAFETY: inotify_init1 returns a new owned descriptor on success.
        let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
        assert!(fd >= 0);
        let mut watcher = unsafe { std::fs::File::from_raw_fd(fd) };
        // SAFETY: watcher is live and name is NUL-terminated.
        assert!(
            unsafe { libc::inotify_add_watch(watcher.as_raw_fd(), name.as_ptr(), libc::IN_OPEN) }
                >= 0
        );
        assert!(check_file(&s(path), &PathGrants::default(), &["wav"], 10)
            .unwrap_err()
            .contains("Access denied"));
        assert_eq!(
            watcher.read(&mut [0u8; 4096]).unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn home_detection_without_home_environment() {
        const CHILD: &str = "KICKS_HOME_TEST_CHILD";
        if std::env::var_os(CHILD).is_some() {
            let home = dirs::home_dir().expect("platform home lookup without HOME");
            assert!(!is_valid_root(&home.canonicalize().unwrap()));
            return;
        }
        assert!(std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "path_guard::tests::home_detection_without_home_environment"
            ])
            .env_remove("HOME")
            .env(CHILD, "1")
            .status()
            .unwrap()
            .success());
    }

    #[test]
    fn canonical_broad_directories_are_rejected() {
        for path in ["/usr", "/etc", "/var", "/tmp", "/home", "/Users"] {
            if let Ok(canonical) = Path::new(path).canonicalize() {
                assert!(!is_valid_root(&canonical));
            }
        }
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
