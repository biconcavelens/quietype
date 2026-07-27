use std::sync::{Mutex, OnceLock};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

struct Loaded {
    path: String,
    ctx: WhisperContext,
}

static CONTEXT: OnceLock<Mutex<Option<Loaded>>> = OnceLock::new();

fn cell() -> &'static Mutex<Option<Loaded>> {
    CONTEXT.get_or_init(|| Mutex::new(None))
}

/// Loads the model into memory if it isn't already. Called once at startup so
/// the first dictation doesn't pay the model-load cost (~1s) on top of its own
/// latency.
pub fn warm(model_path: &str) {
    let path = model_path.to_string();
    std::thread::spawn(move || match ensure_loaded(&path) {
        Ok(_) => eprintln!("[quietype] model ready: {path}"),
        Err(e) => eprintln!("[quietype] model not loaded: {e}"),
    });
}

fn ensure_loaded(model_path: &str) -> Result<(), String> {
    let mut guard = cell().lock().map_err(|_| "model lock poisoned".to_string())?;

    let already_loaded = guard.as_ref().map(|l| l.path == model_path).unwrap_or(false);
    if already_loaded {
        return Ok(());
    }

    if !std::path::Path::new(model_path).exists() {
        return Err(format!(
            "No Whisper model at '{model_path}'. Download one and set its path in Settings."
        ));
    }

    let ctx = WhisperContext::new_with_params(model_path, WhisperContextParameters::default())
        .map_err(|e| format!("Failed to load model: {e}"))?;
    *guard = Some(Loaded {
        path: model_path.to_string(),
        ctx,
    });
    Ok(())
}

/// Transcribes mono 16kHz f32 PCM samples to text. Blocking and CPU-heavy —
/// callers should keep this off the async runtime.
pub fn transcribe(samples: &[f32], model_path: &str) -> Result<String, String> {
    // Whisper needs a minimum amount of audio; anything shorter is a stray
    // hotkey tap rather than speech.
    if samples.len() < 16_000 / 4 {
        return Ok(String::new());
    }

    ensure_loaded(model_path)?;
    let guard = cell().lock().map_err(|_| "model lock poisoned".to_string())?;
    let loaded = guard.as_ref().ok_or("model not loaded")?;

    let mut state = loaded.ctx.create_state().map_err(|e| e.to_string())?;

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_print_progress(false);
    params.set_print_special(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_suppress_blank(true);
    // Use all but one core so the UI thread stays responsive while decoding.
    let threads = std::thread::available_parallelism()
        .map(|n| (n.get() as i32 - 1).max(1))
        .unwrap_or(4);
    params.set_n_threads(threads);

    state.full(params, samples).map_err(|e| e.to_string())?;

    let mut text = String::new();
    for i in 0..state.full_n_segments() {
        if let Some(segment) = state.get_segment(i) {
            text.push_str(&segment.to_str_lossy().map_err(|e| e.to_string())?);
        }
    }
    Ok(text.trim().to_string())
}
