// Kicks — Digital Signal Processing Engine
//
// Real-time audio processing pipeline for the Kicks guitar workstation.
// Provides Boost, Amp, Cab IR, Delay, Reverb, Tuner, Metronome, Looper
// and BassAmp processing stages with CPAL (default) or JACK audio I/O.

pub mod audio_io;
pub mod bass_amp;
pub mod convolution;
pub mod engine;
#[cfg(feature = "fft-convolution")]
pub mod fft_convolution;
pub mod looper;
pub mod metronome;
pub mod nam;
pub mod param;
pub mod plugins;
pub mod tuner;

pub use audio_io::{
    list_audio_devices, AudioConfig, CpalAudioIO, DeviceInfo, JackAudioIO, JackConfig,
};
pub use bass_amp::BassAmp;
pub use convolution::Convolver;
pub use engine::{build_registry, AudioEngine, ChainKind, KicksEngine};
#[cfg(feature = "fft-convolution")]
pub use fft_convolution::FftConvolver;
pub use looper::Looper;
pub use metronome::Metronome;
pub use nam::{NamModelInfo, NeuralModel};
pub use plugins::{compute_rms, Cab, Input, IrInfo, Nam, Output, Plugin, PluginRegistry};
pub use tuner::Tuner;
