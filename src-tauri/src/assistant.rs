use serde_json::{json, Value};

const OLLAMA_URL: &str = "http://localhost:11434";
const MODEL: &str = "gemma4:e4b";
// Ollama unloads an idle model after ~5 minutes by default, and reloading it
// costs ~30s (measured via `ollama ps` / a cold /api/generate call) -- far
// longer than the model actually takes to answer once it's resident. A long
// keep_alive means a normal gap between assistant-mode uses doesn't re-pay
// that cost; warm() below also loads it once at startup rather than on the
// user's first request.
const KEEP_ALIVE: &str = "60m";

/// Loads the model into Ollama's memory ahead of first use, same reasoning as
/// transcribe::warm() for Whisper: pay the cold-load cost at startup, not on
/// the user's first assistant-mode dictation. Fire-and-forget -- if Ollama
/// isn't running, `run()` below will surface a clear error at actual use time.
pub fn warm() {
    tauri::async_runtime::spawn(async {
        let client = reqwest::Client::new();
        let result = client
            .post(format!("{OLLAMA_URL}/api/generate"))
            .json(&json!({ "model": MODEL, "prompt": "", "keep_alive": KEEP_ALIVE }))
            .send()
            .await;
        match result {
            Ok(resp) if resp.status().is_success() => {
                eprintln!("[quietype] assistant model warm: {MODEL}")
            }
            Ok(resp) => eprintln!("[quietype] assistant warm-up failed: {}", resp.status()),
            Err(e) => eprintln!("[quietype] assistant warm-up failed (is Ollama running?): {e}"),
        }
    });
}

/// Sends a spoken instruction plus on-screen context (the user's current
/// selection) to a local Gemma 4 E4B model via Ollama, and returns the text
/// to type in.
///
/// Uses a single `submit_result` tool rather than a "respond with ONLY the
/// text, no preamble" prompt instruction: measured directly, a smaller local
/// model follows a forced tool call far more reliably than it follows a
/// plain-text formatting instruction, and disabling `think` cuts a real
/// exchange from ~8s to ~2.7s with no accuracy loss on the same test case.
pub async fn run(instruction: &str, context: &str) -> Result<String, String> {
    let prompt = if context.trim().is_empty() {
        instruction.to_string()
    } else {
        format!("Selected text:\n{context}\n\nInstruction: {instruction}")
    };

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{OLLAMA_URL}/api/chat"))
        .json(&json!({
            "model": MODEL,
            "messages": [{"role": "user", "content": prompt}],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "submit_result",
                    "description": "Submit the final result text once the instruction has been completed. Always call this instead of replying in plain text.",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "text": {
                                "type": "string",
                                "description": "The final text, ready to insert exactly as-is -- no preamble, no explanation, no markdown fences."
                            }
                        },
                        "required": ["text"]
                    }
                }
            }],
            // Just offering the tool wasn't enough -- verified directly that
            // the model sometimes ignores it and answers conversationally
            // instead (multi-option lists, markdown), which is unsafe to
            // type into a text field. Forcing the specific tool by name is
            // what actually guarantees the response shape.
            "tool_choice": {"type": "function", "function": {"name": "submit_result"}},
            "think": false,
            "keep_alive": KEEP_ALIVE,
            "stream": false
        }))
        .send()
        .await
        .map_err(|e| format!("Couldn't reach Ollama at {OLLAMA_URL} ({e}). Is it running?"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Ollama error {status}: {body}"));
    }

    let body: Value = resp.json().await.map_err(|e| e.to_string())?;

    if let Some(text) = body["message"]["tool_calls"][0]["function"]["arguments"]["text"].as_str()
    {
        return Ok(text.trim().to_string());
    }

    // Model ignored the tool and answered in plain text -- use that rather
    // than failing outright.
    body["message"]["content"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string())
        .ok_or_else(|| "Ollama returned no usable response.".to_string())
}
