use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::{json, Value};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// llama-server serves the assistant model over HTTP.
pub const URL: &str = "http://127.0.0.1:8090";
const PORT: u16 = 8090;

fn default_exe_path() -> String {
    std::env::var("QUIETYPE_LLAMA_SERVER")
        .unwrap_or_else(|_| "vendor/llama-server/llama-server.exe".to_string())
}

fn default_model_path() -> String {
    std::env::var("QUIETYPE_LLAMA_MODEL")
        .unwrap_or_else(|_| "models/gemma-4-E4B-it-Q4_0.gguf".to_string())
}

fn default_mmproj_path() -> String {
    std::env::var("QUIETYPE_LLAMA_MMPROJ")
        .unwrap_or_else(|_| "models/mmproj-gemma-4-E4B-it-BF16.gguf".to_string())
}

static CHILD: OnceLock<Mutex<Option<Child>>> = OnceLock::new();

fn cell() -> &'static Mutex<Option<Child>> {
    CHILD.get_or_init(|| Mutex::new(None))
}

/// Spawns llama-server in the background and waits for it to start accepting
/// connections before logging ready -- mirrors transcribe::warm()'s pattern
/// of paying the load cost at startup on a side thread, not blocking the
/// app or the user's first assistant-mode request. If a request comes in
/// before the server is up, call()/call_agent() surface a clear connection
/// error rather than hanging.
pub fn spawn() {
    std::thread::spawn(|| {
        let exe = default_exe_path();
        let model = default_model_path();
        let mmproj = default_mmproj_path();

        if !std::path::Path::new(&exe).exists() {
            eprintln!("[quietype] llama-server not found at '{exe}' -- assistant mode unavailable");
            return;
        }
        if !std::path::Path::new(&model).exists() {
            eprintln!("[quietype] Gemma model not found at '{model}' -- assistant mode unavailable");
            return;
        }

        let result = Command::new(&exe)
            .args([
                "-m",
                &model,
                "--mmproj",
                &mmproj,
                "--host",
                "127.0.0.1",
                "--port",
                &PORT.to_string(),
                "-c",
                "4096",
                // Required for tool-calling to work at all against Gemma's
                // template -- without it the server falls back to a plain
                // template and tool_choice is silently ignored.
                "--jinja",
                // Chain-of-thought otherwise gets included as visible text,
                // roughly doubling latency for no benefit in this app.
                "--reasoning-format",
                "none",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();

        let child = match result {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[quietype] failed to start llama-server: {e}");
                return;
            }
        };

        if let Ok(mut guard) = cell().lock() {
            *guard = Some(child);
        }

        // llama-server only binds its port once the model is fully loaded,
        // so a successful connect is a reliable readiness signal without
        // needing an HTTP client here.
        for _ in 0..60 {
            if TcpStream::connect(("127.0.0.1", PORT)).is_ok() {
                eprintln!("[quietype] llama-server ready on {URL}");
                return;
            }
            std::thread::sleep(Duration::from_secs(1));
        }
        eprintln!("[quietype] llama-server didn't come up within 60s");
    });
}

/// Kills the child process. Called on app quit -- llama-server has no
/// meaningful idle-unload behavior to rely on, since we own its lifecycle
/// directly rather than sharing it with other processes the way Ollama did.
pub fn stop() {
    if let Ok(mut guard) = cell().lock() {
        if let Some(mut child) = guard.take() {
            let _ = child.kill();
        }
    }
}

/// Without this, tool_choice="required" still lets Gemma ramble through a
/// conversational answer *before* emitting the tool call, wasting tokens and
/// latency (and occasionally not finishing the call before max_tokens).
/// Verified directly -- this instruction is what actually produces a clean,
/// immediate tool_calls response with empty `content`.
const NO_PREAMBLE_SYSTEM_PROMPT: &str = "You must call the submit_result function \
    immediately with the final answer text. Do not write any text before \
    calling it. Do not explain your reasoning. Do not offer multiple \
    options -- pick the single best result and submit it.";

fn tool_def(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": {
                "type": "object",
                "properties": properties,
                "required": required
            }
        }
    })
}

/// Sends one chat-completion request forcing a tool call, and returns the
/// raw `choice` object (`message` + `finish_reason`) for the caller to
/// interpret. Both hard-won reliability fixes from this session live here,
/// in the one place every caller goes through:
///
/// 1. `tool_choice` must be the *string* `"required"` -- the OpenAI
///    object-form forcing a specific named function is silently rejected by
///    this server build ("Expected 'string'") and falls back to unforced,
///    which lets the model skip tools entirely.
/// 2. `max_tokens: 500` gives headroom for this model's habit of rambling
///    100+ tokens of visible `<|channel|>thought...` reasoning before the
///    actual tool call. Callers must check `finish_reason` themselves before
///    trusting a fallback to plain `content` -- a response cut off mid-ramble
///    has no completed tool call, and its `content` is raw leaked reasoning
///    (occasionally literal unstripped special tokens), never a real answer.
///
/// A separate system-role message alongside an `input_audio` content block
/// breaks the model's ability to perceive the audio at all (it claims none
/// was given, even though it's present and correctly heard without one) --
/// callers sending audio must fold every instruction into the `user` turn
/// instead of adding a system message.
async fn post_chat(messages: Vec<Value>, tools: Vec<Value>) -> Result<Value, String> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{URL}/v1/chat/completions"))
        .json(&json!({
            "messages": messages,
            "tools": tools,
            "tool_choice": "required",
            "reasoning_effort": "none",
            "temperature": 0,
            "max_tokens": 500,
            "stream": false
        }))
        .send()
        .await
        .map_err(|e| format!("Couldn't reach the assistant model at {URL} ({e}). Is llama-server running?"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("llama-server error {status}: {body}"));
    }

    let response: Value = resp.json().await.map_err(|e| e.to_string())?;
    Ok(response["choices"][0].clone())
}

/// Reads `arg_name` out of a `tool_name` tool call, or falls back to plain
/// `content` if the model answered in text instead -- but only when the
/// response is genuinely complete (`finish_reason == "stop"`); see
/// `post_chat`'s doc comment for why a truncated response's `content` is
/// never safe to use.
fn extract_result(choice: &Value, tool_name: &str, arg_name: &str) -> Result<String, String> {
    let message = &choice["message"];
    let call = &message["tool_calls"][0];
    if call["function"]["name"].as_str() == Some(tool_name) {
        if let Some(args_str) = call["function"]["arguments"].as_str() {
            if let Ok(args) = serde_json::from_str::<Value>(args_str) {
                if let Some(text) = args[arg_name].as_str() {
                    return Ok(text.trim().to_string());
                }
            }
        }
    }

    if choice["finish_reason"].as_str() != Some("stop") {
        return Err("The assistant model didn't finish before running out of room. Try a shorter instruction.".to_string());
    }

    message["content"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string())
        .ok_or_else(|| "The assistant model returned no usable response.".to_string())
}

/// Sends a single user turn (a plain string, or an array of OpenAI-style
/// content blocks for image/audio input) to the local model, forcing a
/// `submit_result` tool call, and returns its `text` argument. Shared by
/// plain assistant-mode instructions and Gemma-based dictation.
///
/// `with_system_prompt` adds the no-preamble instruction as a separate
/// system-role message -- fine for text-only requests, but see
/// `post_chat`'s doc comment for why audio callers must pass `false` and
/// fold the equivalent instruction into their own user-turn text instead.
pub async fn call(with_system_prompt: bool, user_content: Value) -> Result<String, String> {
    let mut messages = Vec::new();
    if with_system_prompt {
        messages.push(json!({"role": "system", "content": NO_PREAMBLE_SYSTEM_PROMPT}));
    }
    messages.push(json!({"role": "user", "content": user_content}));

    let tools = vec![tool_def(
        "submit_result",
        "Submit the final result text once the instruction has been completed. Always call this instead of replying in plain text.",
        json!({
            "text": {
                "type": "string",
                "description": "The final text, ready to insert exactly as-is -- no preamble, no explanation, no markdown fences."
            }
        }),
        &["text"],
    )];

    let choice = post_chat(messages, tools).await?;
    extract_result(&choice, "submit_result", "text")
}

/// One step the agent loop can take, parsed from whichever tool the model
/// called. See `call_agent`'s doc comment for what each variant means.
pub enum AgentAction {
    SubmitResult(String),
    LookAtScreen,
    /// Click by index into the element list shown in the most recent turn
    /// (built from Windows UI Automation, not vision) -- the primary way to
    /// click now, since picking a labeled item is far more reliable for a
    /// small model than guessing pixel coordinates from a picture.
    ClickElement(u32),
    /// Fallback for anything not in that list (canvas, game, custom-drawn
    /// UI) -- coordinates are pixels on the *scaled* screenshot shown to the
    /// model, not native resolution; the caller must correct for that.
    Click { x: i32, y: i32 },
    TypeText(String),
    KeyPress(String),
    OpenApp(String),
    Say(String),
    Done(String),
}

/// Sends one turn of the agent loop (`agent.rs`) with a small fixed set of
/// computer-action tools, forcing exactly one to be called, and returns
/// which one. `click_element`/`click` are only included when `allow_click`
/// is true -- meaningless before the model has actually seen a screenshot in
/// this conversation (via `LookAtScreen`), and offering them earlier would
/// let the model click blind. Reuses the same `tool_choice`/`max_tokens`/
/// `finish_reason` machinery as `call()`, via `post_chat`.
pub async fn call_agent(messages: Vec<Value>, allow_click: bool) -> Result<AgentAction, String> {
    let mut tools = vec![
        tool_def(
            "submit_result",
            "Submit the final result text once the instruction is a plain text edit/answer with nothing to click or type elsewhere. Always call this instead of replying in plain text.",
            json!({"text": {"type": "string", "description": "The final text, ready to insert exactly as-is."}}),
            &["text"],
        ),
        tool_def(
            "look_at_screen",
            "Look at the current screen: returns a numbered list of real, clickable elements (buttons, fields, links) plus a screenshot. Call this before clicking anything if you haven't looked yet in this conversation. Prefer this plus click_element over open_app for anything involving an existing window, the taskbar, or the Start menu -- e.g. switching to a window that's already open, opening a new window in an app that's already running, or minimizing/maximizing/closing a window.",
            json!({}),
            &[],
        ),
        tool_def(
            "type_text",
            "Type text into whatever field currently has keyboard focus (e.g. after clicking into it).",
            json!({"text": {"type": "string", "description": "The exact text to type."}}),
            &["text"],
        ),
        tool_def(
            "key_press",
            "Press a single key or modifier combo, e.g. \"enter\", \"tab\", \"esc\", \"ctrl+a\".",
            json!({"combo": {"type": "string"}}),
            &["combo"],
        ),
        tool_def(
            "open_app",
            "Launch a brand-new instance of a named application, only when nothing suitable for it is already open on screen. For anything involving a window that might already be open, the taskbar, or the Start menu -- switching to an existing window, opening a new window in an app that's already running, minimizing/maximizing/closing a window -- use look_at_screen and click instead; guessing blind is less reliable than actually looking.",
            json!({"name": {"type": "string", "description": "e.g. \"notepad\", \"calc\", \"chrome\"."}}),
            &["name"],
        ),
        tool_def(
            "say",
            "Say something to the user in the dialogue box without ending the task -- use this to narrate what you're about to do.",
            json!({"message": {"type": "string"}}),
            &["message"],
        ),
        tool_def(
            "done",
            "Declare the task complete.",
            json!({"summary": {"type": "string", "description": "A short summary of what was done, shown to the user."}}),
            &["summary"],
        ),
    ];
    if allow_click {
        tools.push(tool_def(
            "click_element",
            "Click a numbered element from the list you were just shown. Always prefer this over click(x,y) when the thing you want is in that list -- it's exact, not a guess.",
            json!({"id": {"type": "integer", "description": "The number of the element to click, from the most recent list."}}),
            &["id"],
        ));
        tools.push(tool_def(
            "click",
            "Left-click an absolute pixel coordinate on the most recently seen screenshot. Only use this for something NOT in the numbered element list -- e.g. a canvas, game, or custom-drawn area with nothing listed there. Guessing coordinates for something that IS in the list is unreliable; use click_element instead.",
            json!({
                "x": {"type": "integer", "description": "Pixel X, from the left edge of the screenshot."},
                "y": {"type": "integer", "description": "Pixel Y, from the top edge of the screenshot."}
            }),
            &["x", "y"],
        ));
    }

    let choice = post_chat(messages, tools).await?;
    parse_agent_action(&choice)
}

fn parse_agent_action(choice: &Value) -> Result<AgentAction, String> {
    let call = &choice["message"]["tool_calls"][0];
    let (name, args_str) = match (
        call["function"]["name"].as_str(),
        call["function"]["arguments"].as_str(),
    ) {
        (Some(n), Some(a)) => (n, a),
        _ => {
            // No completed tool call -- see post_chat's doc comment for why
            // a truncated response's content is never trusted as an action.
            if choice["finish_reason"].as_str() != Some("stop") {
                return Err("The assistant didn't finish deciding what to do. Try a shorter instruction.".to_string());
            }
            return choice["message"]["content"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .map(|s| AgentAction::SubmitResult(s.trim().to_string()))
                .ok_or_else(|| "The assistant model returned no usable response.".to_string());
        }
    };

    let args: Value = serde_json::from_str(args_str).map_err(|e| e.to_string())?;
    match name {
        "submit_result" => Ok(AgentAction::SubmitResult(str_arg(&args, "text")?)),
        "look_at_screen" => Ok(AgentAction::LookAtScreen),
        "click_element" => Ok(AgentAction::ClickElement(int_arg(&args, "id")? as u32)),
        "click" => Ok(AgentAction::Click {
            x: int_arg(&args, "x")?,
            y: int_arg(&args, "y")?,
        }),
        "type_text" => Ok(AgentAction::TypeText(str_arg(&args, "text")?)),
        "key_press" => Ok(AgentAction::KeyPress(str_arg(&args, "combo")?)),
        "open_app" => Ok(AgentAction::OpenApp(str_arg(&args, "name")?)),
        "say" => Ok(AgentAction::Say(str_arg(&args, "message")?)),
        "done" => Ok(AgentAction::Done(str_arg(&args, "summary")?)),
        other => Err(format!("Model called an unknown tool '{other}'.")),
    }
}

fn str_arg(args: &Value, key: &str) -> Result<String, String> {
    args[key]
        .as_str()
        .map(|s| s.trim().to_string())
        .ok_or_else(|| format!("Missing '{key}' argument."))
}

fn int_arg(args: &Value, key: &str) -> Result<i32, String> {
    args[key]
        .as_i64()
        .map(|n| n as i32)
        .ok_or_else(|| format!("Missing '{key}' argument."))
}

/// Encodes mono f32 PCM as a minimal 16-bit WAV file in memory.
/// audio.rs's recorder already produces exactly the 16kHz mono format this
/// expects, so there's no resampling to do here.
fn encode_wav(samples: &[f32]) -> Vec<u8> {
    const SAMPLE_RATE: u32 = 16_000;
    let data_len = (samples.len() * 2) as u32;

    let mut buf = Vec::with_capacity(44 + data_len as usize);
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&(36 + data_len).to_le_bytes());
    buf.extend_from_slice(b"WAVE");
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
    buf.extend_from_slice(&1u16.to_le_bytes()); // mono
    buf.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    buf.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes()); // byte rate
    buf.extend_from_slice(&2u16.to_le_bytes()); // block align
    buf.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_len.to_le_bytes());
    for &s in samples {
        let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        buf.extend_from_slice(&v.to_le_bytes());
    }
    buf
}

/// Transcribes audio using Gemma's native audio input -- the alternative to
/// Whisper selectable as Settings > Personal's dictation engine. Slower than
/// Whisper on CPU (shares the same model/process as assistant mode rather
/// than a dedicated speech model), but needs no separate model download.
pub async fn transcribe_audio(samples: &[f32], vocabulary: &str) -> Result<String, String> {
    // Same minimum-audio guard as transcribe::transcribe() -- a stray
    // hotkey tap, not speech.
    if samples.len() < 16_000 / 4 {
        return Ok(String::new());
    }

    let b64 = STANDARD.encode(encode_wav(samples));

    // Folded into the user turn rather than a system message -- see
    // post_chat's doc comment for why a separate system message breaks
    // audio input.
    let mut instruction = "Transcribe this audio verbatim. Output exactly what was said, \
        nothing else -- no commentary, no added punctuation beyond what's natural. \
        Call the submit_result function immediately with the transcription; do not \
        write any other text before or after calling it."
        .to_string();
    if !vocabulary.trim().is_empty() {
        let terms = vocabulary.trim().replace('\n', ", ");
        instruction.push_str(&format!(
            " Known names/terms that may appear (use these exact spellings): {terms}"
        ));
    }

    call(
        false,
        json!([
            {"type": "text", "text": instruction},
            {"type": "input_audio", "input_audio": {"data": b64, "format": "wav"}}
        ]),
    )
    .await
}

/// Anthropic's own computer-use docs recommend capping screenshots at
/// roughly XGA/WXGA resolution -- sending native resolution (2880x1800 on a
/// typical laptop, 2-3x that ceiling) measurably hurts both accuracy and
/// latency.
const SCREENSHOT_MAX_DIM: u32 = 1280;

/// Screenshot as an OpenAI-style `image_url` content block (downscaled per
/// `SCREENSHOT_MAX_DIM`), for wiring into the agent loop's message history.
/// Returns the block plus the scale factor (native / sent) the caller needs
/// to correct a coordinate the model reads off of it -- only matters for the
/// fallback `click(x, y)` path; `click_element` needs no correction since
/// its coordinates come from UI Automation directly, already native.
pub fn screenshot_content_block() -> Result<(Value, f32), String> {
    let (b64, scale) = crate::computer::screenshot_scaled_base64(SCREENSHOT_MAX_DIM)?;
    Ok((
        json!({
            "type": "image_url",
            "image_url": {"url": format!("data:image/png;base64,{b64}")}
        }),
        scale,
    ))
}
