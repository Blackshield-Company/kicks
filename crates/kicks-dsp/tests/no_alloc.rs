//! Fails if the real-time audio path allocates, frees or reallocates heap memory.
//!
//! A counting global allocator only counts calls made on a thread that has
//! armed it, so the test harness and other tests are not affected.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use kicks_dsp::param::param_channel;
use kicks_dsp::{AudioEngine, KicksEngine};

struct CountingAlloc;

// Both counters are per-thread so parallel tests can't disturb each other.
thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
    static ALLOCS: Cell<usize> = const { Cell::new(0) };
}

fn note() {
    if ARMED.try_with(|a| a.get()).unwrap_or(false) {
        let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
    }
}

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        note();
        System.alloc(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        note();
        System.dealloc(p, l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        note();
        System.realloc(p, l, n)
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

/// Run `f` and return how many heap operations it made on this thread.
fn count_allocs(f: impl FnOnce()) -> usize {
    ALLOCS.with(|c| c.set(0));
    ARMED.with(|a| a.set(true));
    f();
    ARMED.with(|a| a.set(false));
    ALLOCS.with(|c| c.get())
}

const BS: usize = 256;

fn test_signal(buf: &mut [f32], phase: &mut f32) {
    for s in buf.iter_mut() {
        *s = (*phase).sin() * 0.5;
        *phase += 2.0 * std::f32::consts::PI * 110.0 / 48_000.0;
    }
}

fn assert_chain_rt_safe(name: &str, build: impl FnOnce(&mut KicksEngine)) {
    let mut eng = KicksEngine::new();
    build(&mut eng);
    eng.init(48_000.0, BS as u32).unwrap();
    let ir: Vec<f32> = (0..2048).map(|i| (-(i as f32) / 200.0).exp()).collect();
    eng.load_ir_to_cab("test.wav".into(), ir, 48_000.0);

    let (tx, mut rx) = param_channel();
    let mut input = vec![0.0f32; BS];
    let mut output = vec![0.0f32; BS];
    let mut phase = 0.0f32;

    // Warm-up block outside the measured region (first-touch lazies).
    test_signal(&mut input, &mut phase);
    eng.process_block(&mut rx, &input, &mut output);

    // Enqueue parameter changes (main-thread side, may allocate).
    for (id, v) in [
        ("gain", 0.7),
        ("amp/drive", 0.9),
        ("boost/enabled", 0.0),
        ("delay/mix", 0.4),
        ("reverb/enabled", 1.0),
        ("master", 0.5),
        ("bpm", 0.6),
        ("running", 1.0),
    ] {
        tx.send(id, v).unwrap();
    }

    let n = count_allocs(|| {
        for _ in 0..200 {
            eng.process_block(&mut rx, &input, &mut output);
        }
        // Blocks larger and smaller than the init size.
        let big = [0.1f32; 8192];
        let mut big_out = [0.0f32; 8192];
        eng.process_block(&mut rx, &big, &mut big_out);
        eng.process_block(&mut rx, &input[..64], &mut output[..64]);
    });
    assert_eq!(n, 0, "{name}: audio path made {n} heap operations");
    assert!(output.iter().all(|s| s.is_finite()));
}

#[test]
fn default_chain_does_not_allocate() {
    assert_chain_rt_safe("default", |_| {});
}

#[test]
fn bass_chain_does_not_allocate() {
    assert_chain_rt_safe("bass", |e| e.build_bass_chain());
}

#[test]
fn practice_chain_does_not_allocate() {
    assert_chain_rt_safe("practice", |e| e.build_practice_chain());
}

#[test]
fn looper_chain_does_not_allocate() {
    assert_chain_rt_safe("looper", |e| {
        e.build_looper_chain();
    });
}

#[test]
fn counter_detects_allocation() {
    // Guard against the harness silently counting nothing.
    let n = count_allocs(|| {
        let v: Vec<u8> = Vec::with_capacity(16);
        std::hint::black_box(v);
    });
    assert!(n >= 1);
}
