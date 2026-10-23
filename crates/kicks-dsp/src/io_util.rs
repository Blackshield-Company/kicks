//! Backend-independent helpers for the audio I/O callbacks.
//!
//! Everything here is pure, allocation-free and unit-tested, so the CPAL and
//! JACK callbacks stay thin.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

/// Live counters shared between the audio callbacks and the UI.
#[derive(Debug, Default)]
pub struct AudioStats {
    /// Output callback found too few input samples (input starved).
    pub underruns: AtomicU64,
    /// Input callback could not push (output side not keeping up).
    pub overruns: AtomicU64,
    /// Frames per output callback, as last observed.
    pub last_callback_frames: AtomicU64,
    /// Set if the input stream could not be opened or reported an error.
    pub input_ok: AtomicBool,
    /// Human-readable reason when `input_ok` is false. Written only from
    /// non-real-time code (stream setup / error callbacks).
    pub input_error: Mutex<Option<String>>,
}

impl AudioStats {
    pub fn set_input_error(&self, msg: impl Into<String>) {
        self.input_ok.store(false, Ordering::Relaxed);
        if let Ok(mut e) = self.input_error.lock() {
            *e = Some(msg.into());
        }
    }

    pub fn snapshot(&self) -> AudioStatsSnapshot {
        AudioStatsSnapshot {
            underruns: self.underruns.load(Ordering::Relaxed),
            overruns: self.overruns.load(Ordering::Relaxed),
            last_callback_frames: self.last_callback_frames.load(Ordering::Relaxed),
            input_ok: self.input_ok.load(Ordering::Relaxed),
            input_error: self.input_error.lock().ok().and_then(|e| e.clone()),
        }
    }
}

/// Copyable view of [`AudioStats`] for the UI.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct AudioStatsSnapshot {
    pub underruns: u64,
    pub overruns: u64,
    pub last_callback_frames: u64,
    pub input_ok: bool,
    pub input_error: Option<String>,
}

/// Pick the CPAL buffer size: the requested size when the device supports
/// it, otherwise the nearest supported value; `None` means "use the device
/// default" (the device reported no range).
pub fn choose_buffer_size(requested: u32, supported: Option<(u32, u32)>) -> Option<u32> {
    let (min, max) = supported?;
    if min > max {
        return None;
    }
    Some(requested.clamp(min, max))
}

/// Size the mono input ring from the backend's callback bound, not the
/// requested period (which CPAL may round or ignore). Unknown/invalid bounds
/// must be rejected during setup rather than silently dropping large callbacks.
pub fn input_ring_capacity(output_period: u32, input_range: Option<(u32, u32)>) -> Option<usize> {
    let (min, max) = input_range?;
    if min == 0 || min > max {
        return None;
    }
    let capacity = (max as usize)
        .checked_mul(2)?
        .checked_add(output_period as usize)?
        .max((output_period as usize).checked_mul(8)?)
        .max(4096);
    (capacity <= usize::MAX / 2).then_some(capacity)
}

/// Pick a channel index that exists on the device; falls back to 0.
pub fn clamp_channel(requested: u16, channels: u16) -> usize {
    if channels == 0 || requested >= channels {
        0
    } else {
        requested as usize
    }
}

/// Extract one channel from interleaved input, calling `push` per frame.
/// Returns how many samples `push` rejected (ring full).
#[inline]
pub fn extract_channel(
    data: &[f32], channels: usize, channel: usize, mut push: impl FnMut(f32) -> bool,
) -> usize {
    let channels = channels.max(1);
    let mut dropped = 0;
    for frame in data.chunks_exact(channels) {
        if !push(frame[channel.min(channels - 1)]) {
            dropped += 1;
        }
    }
    dropped
}

/// Write a mono signal to every channel of an interleaved output buffer.
#[inline]
pub fn write_mono_to_interleaved(mono: &[f32], out: &mut [f32], channels: usize) {
    let channels = channels.max(1);
    for (frame, &s) in out.chunks_exact_mut(channels).zip(mono) {
        frame.fill(s);
    }
}

/// Estimated round-trip latency in milliseconds: one input period, the ring
/// pre-fill, one output period.
pub fn estimated_latency_ms(buffer_frames: u32, prefill_frames: u32, sample_rate: f64) -> f64 {
    if sample_rate <= 0.0 {
        return 0.0;
    }
    (2.0 * buffer_frames as f64 + prefill_frames as f64) / sample_rate * 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_size_respects_device_range() {
        assert_eq!(choose_buffer_size(256, Some((64, 4096))), Some(256));
        assert_eq!(choose_buffer_size(32, Some((64, 4096))), Some(64));
        assert_eq!(choose_buffer_size(8192, Some((64, 4096))), Some(4096));
        assert_eq!(choose_buffer_size(256, None), None);
        assert_eq!(choose_buffer_size(256, Some((10, 5))), None);
    }

    #[test]
    fn input_period_is_negotiated_independently() {
        let requested = 256;
        assert_eq!(choose_buffer_size(requested, Some((64, 512))), Some(256));
        assert_eq!(
            choose_buffer_size(requested, Some((1024, 8192))),
            Some(1024)
        );
        assert_eq!(choose_buffer_size(requested, None), None);
    }

    #[test]
    fn input_ring_holds_large_callback_with_output_default() {
        use ringbuf::traits::{Consumer, Producer, Split};

        // Output's unknown default must not limit an 8192-frame input callback
        // to the old 4096-sample ring, even with the initial silence present.
        let prefill = 256;
        let capacity = input_ring_capacity(prefill, Some((64, 8192))).unwrap();
        let (mut producer, mut consumer) = ringbuf::HeapRb::<f32>::new(capacity).split();
        for _ in 0..prefill {
            producer.try_push(0.0).unwrap();
        }
        let input = vec![1.0; 8192];
        assert_eq!(
            extract_channel(&input, 1, 0, |s| producer.try_push(s).is_ok()),
            0
        );
        for _ in 0..prefill {
            assert_eq!(consumer.try_pop(), Some(0.0));
        }
        for _ in 0..8192 {
            assert_eq!(consumer.try_pop(), Some(1.0));
        }
        assert_eq!(consumer.try_pop(), None);
    }

    #[test]
    fn input_ring_requires_valid_backend_bound() {
        assert_eq!(input_ring_capacity(256, None), None);
        assert_eq!(input_ring_capacity(256, Some((8192, 64))), None);
        assert_eq!(input_ring_capacity(256, Some((0, 8192))), None);
    }

    #[test]
    fn channel_clamps_to_device() {
        assert_eq!(clamp_channel(1, 2), 1);
        assert_eq!(clamp_channel(2, 2), 0);
        assert_eq!(clamp_channel(0, 1), 0);
        assert_eq!(clamp_channel(3, 0), 0);
    }

    #[test]
    fn extracts_selected_channel_from_mono_stereo_and_quad() {
        let mut got = Vec::new();
        extract_channel(&[1.0, 2.0, 3.0], 1, 0, |s| {
            got.push(s);
            true
        });
        assert_eq!(got, [1.0, 2.0, 3.0]);

        got.clear();
        extract_channel(&[1.0, 10.0, 2.0, 20.0], 2, 1, |s| {
            got.push(s);
            true
        });
        assert_eq!(got, [10.0, 20.0]);

        got.clear();
        extract_channel(&[0.0, 0.0, 7.0, 0.0, 0.0, 0.0, 8.0, 0.0], 4, 2, |s| {
            got.push(s);
            true
        });
        assert_eq!(got, [7.0, 8.0]);
    }

    #[test]
    fn extract_counts_drops() {
        let dropped = extract_channel(&[1.0, 2.0, 3.0, 4.0], 1, 0, |s| s < 3.0);
        assert_eq!(dropped, 2);
    }

    #[test]
    fn writes_mono_to_any_channel_count() {
        let mut out = [9.0f32; 6];
        write_mono_to_interleaved(&[1.0, 2.0], &mut out, 3);
        assert_eq!(out, [1.0, 1.0, 1.0, 2.0, 2.0, 2.0]);
        let mut mono_out = [0.0f32; 2];
        write_mono_to_interleaved(&[5.0, 6.0], &mut mono_out, 1);
        assert_eq!(mono_out, [5.0, 6.0]);
    }

    #[test]
    fn latency_estimate() {
        let ms = estimated_latency_ms(256, 256, 48_000.0);
        assert!((ms - 16.0).abs() < 1e-9);
        assert_eq!(estimated_latency_ms(256, 0, 0.0), 0.0);
    }

    #[test]
    fn stats_snapshot_reports_input_error() {
        let s = AudioStats::default();
        s.input_ok.store(true, Ordering::Relaxed);
        s.underruns.fetch_add(3, Ordering::Relaxed);
        s.set_input_error("no device");
        let snap = s.snapshot();
        assert_eq!(snap.underruns, 3);
        assert!(!snap.input_ok);
        assert_eq!(snap.input_error.as_deref(), Some("no device"));
    }
}
