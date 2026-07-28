//! Measures transcription speed for the configured model on this machine,
//! with model loading (I/O-bound, disk read) timed separately from actual
//! inference (CPU-bound) -- conflating the two hides which one is actually
//! slow. `cargo run --release --example bench` vs the debug version shows
//! the optimization-level gap on the inference number specifically.

use quietype_lib::transcribe::{ensure_loaded, transcribe};
use std::time::Instant;

fn main() {
    let model_path = std::env::var("QUIETYPE_WHISPER_MODEL")
        .unwrap_or_else(|_| "models/ggml-base.en.bin".to_string());

    // Content doesn't matter for a timing benchmark -- the encoder/decoder
    // matrix multiplies that dominate cost run the same regardless of what's
    // in the buffer. A couple seconds of low-amplitude tone stands in for
    // real speech without needing an actual recording on hand.
    let seconds = 2.0_f32;
    let sample_rate = 16_000;
    let samples: Vec<f32> = (0..(seconds * sample_rate as f32) as usize)
        .map(|i| (i as f32 * 0.05).sin() * 0.1)
        .collect();

    println!("model: {model_path}");
    println!("audio: {seconds}s ({} samples)\n", samples.len());

    let t0 = Instant::now();
    if let Err(e) = ensure_loaded(&model_path) {
        println!("model load failed: {e}");
        return;
    }
    println!("model load (disk read + init): {:?}", t0.elapsed());

    // Run twice: the first call can still pay one-off lazy setup inside
    // whisper.cpp itself, the second is steady-state per-utterance cost.
    for run in 1..=2 {
        let t1 = Instant::now();
        match transcribe(&samples, &model_path) {
            Ok(text) => println!("inference run {run}: {:?} -> {text:?}", t1.elapsed()),
            Err(e) => println!("inference run {run} failed after {:?}: {e}", t1.elapsed()),
        }
    }
}
