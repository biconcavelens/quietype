use std::sync::{Mutex, OnceLock};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

static CONTEXT: OnceLock<Mutex<WhisperContext>> = OnceLock::new();

fn model_path() -> String {
    std::env::var("QUIETYPE_WHISPER_MODEL").unwrap_or_else(|_| "models/ggml-base.en.bin".to_string())
}

fn get_context() -> Result<&'static Mutex<WhisperContext>, String> {
    if let Some(ctx) = CONTEXT.get() {
        return Ok(ctx);
    }

    let path = model_path();
    if !std::path::Path::new(&path).exists() {
        return Err(format!(
            "Whisper model not found at '{path}'. Download a ggml model \
             (see README) or set QUIETYPE_WHISPER_MODEL to its path."
        ));
    }

    let ctx = WhisperContext::new_with_params(&path, WhisperContextParameters::default())
        .map_err(|e| format!("failed to load whisper model: {e}"))?;
    let _ = CONTEXT.set(Mutex::new(ctx));
    Ok(CONTEXT.get().expect("just set"))
}

/// Transcribes mono 16kHz f32 PCM samples to text.
pub fn transcribe(samples: &[f32]) -> Result<String, String> {
    if samples.is_empty() {
        return Ok(String::new());
    }

    let ctx = get_context()?;
    let ctx = ctx.lock().map_err(|_| "whisper context lock poisoned".to_string())?;
    let mut state = ctx.create_state().map_err(|e| e.to_string())?;

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_print_progress(false);
    params.set_print_special(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);

    state.full(params, samples).map_err(|e| e.to_string())?;

    let num_segments = state.full_n_segments();
    let mut text = String::new();
    for i in 0..num_segments {
        if let Some(segment) = state.get_segment(i) {
            text.push_str(&segment.to_str_lossy().map_err(|e| e.to_string())?);
        }
    }
    Ok(text.trim().to_string())
}
