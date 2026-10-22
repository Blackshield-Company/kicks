//! Mutex helpers for Tauri command handlers.

use std::sync::{Mutex, MutexGuard};

/// Lock a mutex, recovering the data if a previous holder panicked.
///
/// Tauri commands must never panic on a poisoned lock: one panicking
/// command would otherwise take every later command down with it.
pub trait LockOrRecover<T> {
    fn lock_or_recover(&self) -> MutexGuard<'_, T>;
}

impl<T> LockOrRecover<T> for Mutex<T> {
    fn lock_or_recover(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|poisoned| {
            tracing::warn!("recovering from poisoned mutex");
            poisoned.into_inner()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn recovers_poisoned_mutex() {
        let m = Arc::new(Mutex::new(41));
        let m2 = m.clone();
        let _ = std::thread::spawn(move || {
            let _g = m2.lock().unwrap();
            panic!("poison it");
        })
        .join();
        assert!(m.is_poisoned());
        *m.lock_or_recover() += 1;
        assert_eq!(*m.lock_or_recover(), 42);
    }

    #[test]
    fn release_config_disables_devtools() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        for w in conf["app"]["windows"].as_array().unwrap() {
            assert_ne!(w.get("devtools"), Some(&serde_json::Value::Bool(true)));
        }
    }
}
