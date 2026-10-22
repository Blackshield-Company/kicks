//! WAV recording / DI capture.
//!
//! The audio thread holds a [`RecordTap`] and pushes `(dry input, processed
//! output)` frame pairs into a lock-free ring. A writer thread drains the
//! ring into a 2-channel 32-bit float WAV: **channel 1 is the dry DI**
//! (re-ampable later), **channel 2 is the processed sound**.
//!
//! The tap never allocates, blocks or touches the filesystem. If the writer
//! falls behind, frames are dropped and counted rather than stalling audio.

use ringbuf::traits::{Consumer, Observer, Producer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

/// Ring capacity in seconds of stereo audio (gives the writer slack for
/// slow disks without dropping).
const RING_SECONDS: usize = 4;

/// Audio-thread side of a recording.
pub struct RecordTap {
    prod: HeapProd<f32>,
    dropped: Arc<AtomicU64>,
}

impl RecordTap {
    /// Push one block. Allocation-free and wait-free.
    pub fn push(&mut self, dry: &[f32], wet: &[f32]) {
        let n = dry.len().min(wet.len());
        let fit = n.min(self.prod.vacant_len() / 2);
        for i in 0..fit {
            let _ = self.prod.try_push(dry[i]);
            let _ = self.prod.try_push(wet[i]);
        }
        if fit < n {
            self.dropped.fetch_add((n - fit) as u64, Ordering::Relaxed);
        }
    }
}

/// Result of a finished recording.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RecordingSummary {
    pub path: String,
    pub frames: u64,
    pub dropped_frames: u64,
    pub seconds: f64,
}

/// Control-thread handle: stop it to finalise the file.
pub struct Recording {
    path: PathBuf,
    sample_rate: u32,
    stop: Arc<AtomicBool>,
    frames: Arc<AtomicU64>,
    dropped: Arc<AtomicU64>,
    writer: Option<JoinHandle<anyhow::Result<()>>>,
}

impl Recording {
    /// Create the WAV file and writer thread. Returns the tap to hand to the
    /// engine (see `KicksEngine::set_record_tap`).
    pub fn start(path: &Path, sample_rate: u32) -> anyhow::Result<(RecordTap, Recording)> {
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut wav = hound::WavWriter::create(path, spec)?;
        let cap = (sample_rate as usize * 2 * RING_SECONDS).max(8192);
        let (prod, cons) = HeapRb::<f32>::new(cap).split();
        let stop = Arc::new(AtomicBool::new(false));
        let frames = Arc::new(AtomicU64::new(0));
        let dropped = Arc::new(AtomicU64::new(0));
        let (stop2, frames2) = (stop.clone(), frames.clone());
        let writer = std::thread::Builder::new()
            .name("kicks-recorder".into())
            .spawn(move || -> anyhow::Result<()> {
                drain_loop(cons, &mut wav, &stop2, &frames2)?;
                wav.finalize()?;
                Ok(())
            })?;
        Ok((
            RecordTap {
                prod,
                dropped: dropped.clone(),
            },
            Recording {
                path: path.to_path_buf(),
                sample_rate,
                stop,
                frames,
                dropped,
                writer: Some(writer),
            },
        ))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn frames(&self) -> u64 {
        self.frames.load(Ordering::Relaxed)
    }

    pub fn dropped_frames(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Stop, flush everything still in the ring and finalise the WAV header.
    /// Drop the engine's [`RecordTap`] first so no more audio arrives.
    pub fn stop(mut self) -> anyhow::Result<RecordingSummary> {
        self.stop.store(true, Ordering::Release);
        if let Some(h) = self.writer.take() {
            h.join()
                .map_err(|_| anyhow::anyhow!("recorder thread panicked"))??;
        }
        let frames = self.frames();
        Ok(RecordingSummary {
            path: self.path.display().to_string(),
            frames,
            dropped_frames: self.dropped_frames(),
            seconds: frames as f64 / self.sample_rate as f64,
        })
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(h) = self.writer.take() {
            let _ = h.join();
        }
    }
}

fn drain_loop<W: std::io::Write + std::io::Seek>(
    mut cons: HeapCons<f32>, wav: &mut hound::WavWriter<W>, stop: &AtomicBool, frames: &AtomicU64,
) -> anyhow::Result<()> {
    let mut buf = vec![0.0f32; 16_384];
    loop {
        // Read the flag before draining so samples pushed before stop() are kept.
        let stopping = stop.load(Ordering::Acquire);
        let mut wrote = false;
        loop {
            // Only take whole frames (pairs).
            let avail = cons.occupied_len() & !1;
            if avail == 0 {
                break;
            }
            let take = avail.min(buf.len());
            let n = cons.pop_slice(&mut buf[..take]);
            for s in &buf[..n] {
                wav.write_sample(*s)?;
            }
            frames.fetch_add((n / 2) as u64, Ordering::Relaxed);
            wrote = true;
        }
        if stopping {
            return Ok(());
        }
        if !wrote {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

/// `kicks-<unix seconds>.wav` inside `dir`.
pub fn timestamped_path(dir: &Path) -> PathBuf {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut p = dir.join(format!("kicks-{secs}.wav"));
    let mut i = 1;
    while p.exists() {
        p = dir.join(format!("kicks-{secs}-{i}.wav"));
        i += 1;
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kicks-rec-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn records_dry_and_wet_channels() {
        let path = tmp("a.wav");
        let (mut tap, rec) = Recording::start(&path, 48_000).unwrap();
        let dry: Vec<f32> = (0..480).map(|i| i as f32 / 480.0).collect();
        let wet: Vec<f32> = dry.iter().map(|x| -x).collect();
        for _ in 0..10 {
            tap.push(&dry, &wet);
        }
        drop(tap);
        let sum = rec.stop().unwrap();
        assert_eq!(sum.frames, 4800);
        assert_eq!(sum.dropped_frames, 0);
        assert!((sum.seconds - 0.1).abs() < 1e-9);

        let mut r = hound::WavReader::open(&path).unwrap();
        assert_eq!(r.spec().channels, 2);
        assert_eq!(r.spec().sample_rate, 48_000);
        let s: Vec<f32> = r.samples::<f32>().map(Result::unwrap).collect();
        assert_eq!(s.len(), 9600);
        assert_eq!(s[2 * 100], dry[100]);
        assert_eq!(s[2 * 100 + 1], wet[100]);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn overflow_drops_and_counts_instead_of_blocking() {
        let path = tmp("b.wav");
        // Tiny sample rate → tiny ring (min 8192 samples = 4096 frames).
        let (mut tap, rec) = Recording::start(&path, 1000).unwrap();
        let block = vec![0.1f32; 100_000];
        tap.push(&block, &block);
        assert!(rec.dropped_frames() > 0);
        drop(tap);
        let sum = rec.stop().unwrap();
        assert_eq!(sum.frames + sum.dropped_frames, 100_000);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn timestamped_path_does_not_overwrite() {
        let dir = tmp("x").parent().unwrap().join("ts");
        std::fs::create_dir_all(&dir).unwrap();
        let a = timestamped_path(&dir);
        std::fs::write(&a, b"").unwrap();
        let b = timestamped_path(&dir);
        assert_ne!(a, b);
        std::fs::remove_file(&a).ok();
    }
}
