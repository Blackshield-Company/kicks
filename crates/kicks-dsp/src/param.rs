// Kicks — Lock-free Parameter Channel
//
// An SPSC (single-producer, single-consumer) channel for lock-free
// parameter changes from the main thread to the audio callback.
// Uses ringbuf's atomic-index-based heap ring buffer internally —
// the Producer and Consumer share state through atomics, not locks.
//
// Real-time safety:
//   - send()  (main thread side)   — NOT real-time safe (brief `Mutex` lock
//     for borrow checker; uncontended in practice)
//   - receive (audio callback side) — real-time safe (lock-free `try_pop`
//     in the consumer closure, no mutex involved)

use ringbuf::traits::{Consumer, Producer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};

/// Maximum number of pending parameter changes.
///
/// This should be large enough to handle bulk operations such as
/// scene loading (~50+ parameter pushes) without dropping changes.
/// 256 entries at ~24 bytes each ≈ 6 KB — negligible overhead.
pub const PARAM_QUEUE_CAPACITY: usize = 256;

/// Maximum byte length of a parameter id (e.g. `"reverb/enabled"`).
pub const PARAM_KEY_MAX: usize = 47;

/// A parameter id stored inline, so queue messages never own heap memory.
///
/// The old queue carried `(String, f32)`; popping it on the audio thread
/// freed the `String` there, which is a real-time violation.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ParamKey {
    len: u8,
    buf: [u8; PARAM_KEY_MAX],
}

impl ParamKey {
    /// Build a key; `None` if `id` is longer than [`PARAM_KEY_MAX`] bytes.
    pub fn new(id: &str) -> Option<Self> {
        let bytes = id.as_bytes();
        if bytes.len() > PARAM_KEY_MAX {
            return None;
        }
        let mut buf = [0u8; PARAM_KEY_MAX];
        buf[..bytes.len()].copy_from_slice(bytes);
        Some(Self {
            len: bytes.len() as u8,
            buf,
        })
    }

    pub fn as_str(&self) -> &str {
        // Built from a valid &str and never mutated.
        std::str::from_utf8(&self.buf[..self.len as usize]).unwrap_or("")
    }
}

impl std::fmt::Debug for ParamKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ParamKey({:?})", self.as_str())
    }
}

/// A queued parameter change.
pub type ParamMsg = (ParamKey, f32);

/// Error returned when a parameter change cannot be queued.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamSendError {
    /// The queue is at capacity.
    Full,
    /// The id is longer than [`PARAM_KEY_MAX`].
    KeyTooLong,
}

/// Kept for source compatibility.
pub type ParamQueueFull = ParamSendError;

/// Thread-safe sender for parameter changes.
///
/// Internally wraps `HeapProd` in a `std::sync::Mutex` only because
/// the ringbuf traits require `&mut self` on some operations. The ring
/// buffer's index operations are atomic (lock-free), so the inner
/// `Mutex` is uncontended — it purely satisfies the borrow checker.
///
/// `ParamSender` is `Send` + `Sync`, so it can be shared via `&` across
/// threads (e.g., in Tauri's `State`).
pub struct ParamSender {
    inner: std::sync::Mutex<HeapProd<ParamMsg>>,
}

impl ParamSender {
    /// Enqueue a parameter change.
    ///
    /// Returns `Err(ParamQueueFull)` if the ring buffer is full. The
    /// audio callback drains the queue before every `process_all` call,
    /// so this should only fill up during extreme bulk operations.
    pub fn send(&self, id: impl AsRef<str>, value: f32) -> Result<(), ParamSendError> {
        let key = ParamKey::new(id.as_ref()).ok_or(ParamSendError::KeyTooLong)?;
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner
            .try_push((key, value))
            .map_err(|_| ParamSendError::Full)
    }
}

/// The consumer end of the parameter channel, moved into the audio
/// callback and drained before each processing cycle.
pub type ParamConsumer = HeapCons<ParamMsg>;

/// Pop one pending change. Real-time safe: no allocation, no lock.
#[inline]
pub fn try_recv(rx: &mut ParamConsumer) -> Option<ParamMsg> {
    rx.try_pop()
}

/// Create a new parameter channel.
///
/// Returns a `(ParamSender, ParamConsumer)` pair. The sender is stored
/// in `AppState` for main-thread access; the consumer is passed into
/// `CpalAudioIO::start()` and moved into the audio callback closure.
pub fn param_channel() -> (ParamSender, ParamConsumer) {
    let rb = HeapRb::<ParamMsg>::new(PARAM_QUEUE_CAPACITY);
    let (producer, consumer) = rb.split();
    (
        ParamSender {
            inner: std::sync::Mutex::new(producer),
        },
        consumer,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_and_receive() {
        let (tx, mut rx) = param_channel();
        tx.send("gain", 0.5).unwrap();
        tx.send("master", 0.8).unwrap();
        let (k, v) = try_recv(&mut rx).unwrap();
        assert_eq!((k.as_str(), v), ("gain", 0.5));
        let (k, v) = try_recv(&mut rx).unwrap();
        assert_eq!((k.as_str(), v), ("master", 0.8));
        assert!(try_recv(&mut rx).is_none());
    }

    #[test]
    fn queue_full_error() {
        let small_capacity = 2;
        let rb = HeapRb::<ParamMsg>::new(small_capacity);
        let (producer, _consumer) = rb.split();
        let tx = ParamSender {
            inner: std::sync::Mutex::new(producer),
        };

        assert!(tx.send("a", 1.0).is_ok());
        assert!(tx.send("b", 2.0).is_ok());
        assert_eq!(tx.send("c", 3.0), Err(ParamSendError::Full));
    }

    #[test]
    fn key_too_long_is_rejected() {
        let (tx, _rx) = param_channel();
        let long = "x".repeat(PARAM_KEY_MAX + 1);
        assert_eq!(tx.send(&long, 1.0), Err(ParamSendError::KeyTooLong));
        assert!(tx.send("x".repeat(PARAM_KEY_MAX), 1.0).is_ok());
    }

    #[test]
    fn key_round_trips_utf8() {
        assert_eq!(ParamKey::new("amp/drive").unwrap().as_str(), "amp/drive");
        assert_eq!(ParamKey::new("").unwrap().as_str(), "");
    }

    #[test]
    fn message_is_copy() {
        fn assert_copy<T: Copy>() {}
        assert_copy::<ParamMsg>();
    }

    #[test]
    fn param_sender_is_sync() {
        fn assert_sync<T: Sync>() {}
        assert_sync::<ParamSender>();
    }
}
