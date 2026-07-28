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

pub fn ensure_loaded(model_path: &str) -> Result<(), String> {
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
    // "All available cores minus one" is wrong on hybrid CPUs (P-cores +
    // efficiency cores): ggml's thread pool has no concept of core
    // heterogeneity, so spreading work onto slow E-cores makes a
    // synchronized parallel matmul wait on its slowest thread. Measured on a
    // 16-core/22-thread Meteor Lake chip: 21 threads = ~24s, 6 threads =
    // ~6.9s, 4 threads = ~9.4s for the same clip -- more threads was actively
    // *worse* past a fairly low ceiling. Can be overridden for tuning.
    let threads = std::env::var("QUIETYPE_THREADS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|n| n.get() as i32)
                .unwrap_or(6)
                .min(6)
        });
    params.set_n_threads(threads);

    state.full(params, samples).map_err(|e| e.to_string())?;

    let mut text = String::new();
    for i in 0..state.full_n_segments() {
        if let Some(segment) = state.get_segment(i) {
            text.push_str(&segment.to_str_lossy().map_err(|e| e.to_string())?);
        }
    }
    Ok(strip_non_speech_tags(&text))
}

/// Whisper describes non-speech audio with literal bracketed/parenthetical
/// text -- "[BLANK_AUDIO]", "[MUSIC]", "(background noise)" -- rather than a
/// control token, so param-level suppression doesn't catch it. Trimming
/// silence in audio.rs makes this rare; this is the safety net for what
/// still slips through (breathing, a stray click, etc).
fn strip_non_speech_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut depth: i32 = 0;
    for ch in text.chars() {
        match ch {
            '[' | '(' => depth += 1,
            ']' | ')' => depth = (depth - 1).max(0),
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_bracketed_and_parenthetical_tags() {
        assert_eq!(
            strip_non_speech_tags("Hello, hello, hello.[BLANK_AUDIO]"),
            "Hello, hello, hello."
        );
        assert_eq!(
            strip_non_speech_tags("(background noise) turn left at the light"),
            "turn left at the light"
        );
    }

    #[test]
    fn leaves_normal_text_untouched() {
        assert_eq!(
            strip_non_speech_tags("just a normal sentence, no tags here"),
            "just a normal sentence, no tags here"
        );
    }
}
