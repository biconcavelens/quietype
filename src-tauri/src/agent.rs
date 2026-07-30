//! The assistant-mode agent loop: turns a spoken instruction into either a
//! single text result (today's plain-edit case, unchanged in cost/latency)
//! or a sequence of on-screen actions (look at the screen, click, type,
//! press keys, open an app) -- narrating progress via the pet's dialogue
//! and gating each on-screen action on the configured autonomy level.

use crate::{computer, emit_state, engine, hotkeys, inject, store, Mode};
use serde_json::{json, Value};
use std::time::Duration;

/// Hard safety cap on loop iterations -- not a user setting, nobody asked
/// for a step-count knob. Rare to hit for reasonable tasks; exists so a
/// confused model can't loop forever.
const MAX_STEPS: u32 = 8;

/// How long "preview" autonomy mode shows the pending action before
/// auto-proceeding.
const PREVIEW_DELAY: Duration = Duration::from_millis(1500);
/// How long "confirm" autonomy mode waits for Enter/Escape before treating
/// silence as a cancel.
const CONFIRM_TIMEOUT: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// What the loop settled on. `process()` in lib.rs treats these
/// differently: `TypeText` goes through the exact same
/// type-into-focused-field path dictation already uses; `Done` has already
/// shown its summary via `emit_state` inside the loop and has nothing to
/// type anywhere.
pub enum AgentOutcome {
    TypeText(String),
    Done(String),
}

fn build_prompt(instruction: &str, context: &str, personal_context: &str, vocabulary: &str) -> String {
    let mut preamble = String::new();
    if !personal_context.trim().is_empty() {
        preamble.push_str(&format!("About the user: {}\n", personal_context.trim()));
    }
    if !vocabulary.trim().is_empty() {
        let terms = vocabulary.trim().replace('\n', ", ");
        preamble.push_str(&format!("Known names/terms (use these exact spellings): {terms}\n"));
    }
    if !preamble.is_empty() {
        preamble.push('\n');
    }
    let body = if context.trim().is_empty() {
        instruction.to_string()
    } else {
        format!("Selected text:\n{context}\n\nInstruction: {instruction}")
    };
    format!("{preamble}{body}")
}

/// Runs assistant mode: a spoken instruction (plus any selected-text
/// context) either resolves in one turn to plain text (typed in exactly
/// like today), or spins up a multi-step loop that can look at the screen,
/// click, type, press keys, and open applications.
pub async fn run(
    app: &tauri::AppHandle,
    mode: Mode,
    instruction: &str,
    context: &str,
    settings: &store::Settings,
) -> Result<AgentOutcome, String> {
    hotkeys::clear_abort();

    let prompt = build_prompt(
        instruction,
        context,
        &settings.personal_context,
        &settings.vocabulary,
    );
    let mut messages: Vec<Value> = vec![json!({"role": "user", "content": prompt})];
    let mut has_screenshot = false;
    let mut elements: Vec<computer::UiElement> = Vec::new();
    let mut scale_factor: f32 = 1.0;

    for step in 0..MAX_STEPS {
        if hotkeys::abort_requested() {
            return Err("Cancelled.".to_string());
        }

        let action = engine::call_agent(messages.clone(), has_screenshot).await?;

        match action {
            engine::AgentAction::SubmitResult(text) => return Ok(AgentOutcome::TypeText(text)),

            engine::AgentAction::Done(summary) => {
                emit_state(app, "done", mode, Some(summary.clone()));
                return Ok(AgentOutcome::Done(summary));
            }

            engine::AgentAction::Say(message) => {
                emit_state(app, "acting", mode, Some(message.clone()));
                messages.push(json!({"role": "user", "content": format!(
                    "(You said: \"{message}\") Continue with the task."
                )}));
            }

            engine::AgentAction::LookAtScreen => {
                emit_state(app, "acting", mode, Some("Looking at the screen…".to_string()));
                refresh_context(&mut messages, &mut elements, &mut scale_factor, "Here's the current screen.")?;
                has_screenshot = true;
            }

            engine::AgentAction::ClickElement(id) => {
                let Some(el) = (id as usize).checked_sub(1).and_then(|i| elements.get(i)) else {
                    messages.push(json!({"role": "user", "content": format!(
                        "There's no element numbered {id} in the list you were shown. Look at the screen again if you need an up-to-date list."
                    )}));
                    continue;
                };
                let desc = format!("Clicking \"{}\"", el.name);
                if !gate(app, mode, &settings.autonomy, &desc).await {
                    return Err("Cancelled.".to_string());
                }
                let result = computer::click_element(el);
                let summary = action_summary(&result, &format!("Clicked \"{}\"", el.name));
                refresh_context(&mut messages, &mut elements, &mut scale_factor, &summary)?;
                has_screenshot = true;
            }

            engine::AgentAction::Click { x, y } => {
                // Coordinates came from the *scaled* screenshot the model
                // saw, not native resolution -- see screenshot_content_block's
                // doc comment for why this correction matters.
                let (real_x, real_y) = ((x as f32 * scale_factor) as i32, (y as f32 * scale_factor) as i32);
                let desc = format!("Clicking at ({real_x}, {real_y})");
                if !gate(app, mode, &settings.autonomy, &desc).await {
                    return Err("Cancelled.".to_string());
                }
                let result = computer::click(real_x, real_y);
                let summary = action_summary(&result, &format!("Clicked at ({real_x}, {real_y})"));
                refresh_context(&mut messages, &mut elements, &mut scale_factor, &summary)?;
                has_screenshot = true;
            }

            engine::AgentAction::TypeText(text) => {
                let desc = format!("Typing: {}", truncate(&text, 40));
                if !gate(app, mode, &settings.autonomy, &desc).await {
                    return Err("Cancelled.".to_string());
                }
                let result = inject::type_text(&text);
                let summary = action_summary(&result, "Typed the text");
                refresh_context(&mut messages, &mut elements, &mut scale_factor, &summary)?;
                has_screenshot = true;
            }

            engine::AgentAction::KeyPress(combo) => {
                let desc = format!("Pressing {combo}");
                if !gate(app, mode, &settings.autonomy, &desc).await {
                    return Err("Cancelled.".to_string());
                }
                let result = computer::key_combo(&combo);
                let summary = action_summary(&result, &format!("Pressed {combo}"));
                refresh_context(&mut messages, &mut elements, &mut scale_factor, &summary)?;
                has_screenshot = true;
            }

            engine::AgentAction::OpenApp(name) => {
                let desc = format!("Opening {name}");
                if !gate(app, mode, &settings.autonomy, &desc).await {
                    return Err("Cancelled.".to_string());
                }
                let result = computer::open_app(&name);
                // computer::open_app only confirms the shell command itself
                // launched -- Windows resolves the app name (and shows its
                // own "can't find that" dialog on failure) asynchronously,
                // invisibly to that Ok(()). A screenshot is the only way to
                // actually find out what happened, and both a real app
                // window and that error dialog need a beat to render.
                tokio::time::sleep(Duration::from_millis(700)).await;
                let summary = action_summary(&result, &format!("Ran the open command for {name}"));
                refresh_context(&mut messages, &mut elements, &mut scale_factor, &summary)?;
                has_screenshot = true;
            }
        }

        if step + 1 == MAX_STEPS {
            emit_state(app, "error", mode, Some("Couldn't finish that.".to_string()));
            return Err("Ran out of steps.".to_string());
        }
    }

    Err("Ran out of steps.".to_string())
}

fn action_summary(result: &Result<(), String>, ok_text: &str) -> String {
    match result {
        Ok(()) => format!("{ok_text}."),
        Err(e) => format!("{ok_text} failed: {e}."),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        format!("{}…", s.chars().take(max).collect::<String>())
    } else {
        s.to_string()
    }
}

/// Refreshes both signals the model has for "what's actually on screen" --
/// a numbered list of real UI elements (Windows UI Automation, not vision)
/// plus a screenshot -- and appends them as the next user turn. Called after
/// an explicit look-at-screen and after every action that can change the
/// screen, since the model has no other way to verify an action actually
/// worked (e.g. `open_app` only confirms its launch *command* ran, not that
/// Windows found the app -- this is what catches that failure).
fn refresh_context(
    messages: &mut Vec<Value>,
    elements: &mut Vec<computer::UiElement>,
    scale_factor: &mut f32,
    result_text: &str,
) -> Result<(), String> {
    drop_old_screenshots(messages);
    *elements = computer::enumerate_elements().unwrap_or_default();
    let (block, scale) = engine::screenshot_content_block()?;
    *scale_factor = scale;
    let listing = format_element_list(elements);
    messages.push(json!({"role": "user", "content": [
        {"type": "text", "text": format!("{result_text}\n\n{listing}")},
        block
    ]}));
    Ok(())
}

/// Numbers elements 1-based for the model to reference in `click_element` --
/// matching how the list is read back out of `elements` (`id - 1`).
fn format_element_list(elements: &[computer::UiElement]) -> String {
    if elements.is_empty() {
        return "No labeled elements found on screen -- use click(x, y) instead.".to_string();
    }
    let mut out = String::from("Elements on screen (click_element by number):\n");
    for (i, el) in elements.iter().enumerate() {
        out.push_str(&format!("{}. [{}] \"{}\"\n", i + 1, el.role, el.name));
    }
    out
}

/// Strips image content blocks from all existing messages, replacing them
/// with a short placeholder -- keeps the text history of what was done
/// intact for context, but ensures only the *latest* turn's screenshot
/// counts toward prompt size, so per-turn cost doesn't grow with step count.
fn drop_old_screenshots(messages: &mut [Value]) {
    for m in messages.iter_mut() {
        if let Some(arr) = m["content"].as_array_mut() {
            for block in arr.iter_mut() {
                if block["type"] == "image_url" {
                    *block = json!({"type": "text", "text": "[earlier screenshot omitted]"});
                }
            }
        }
    }
}

/// Gates a pending on-screen action on the configured autonomy level.
/// Returns `true` to proceed, `false` if the user canceled (or the confirm
/// window timed out). Confirm/cancel goes through the same system-wide
/// keyboard hook used for the hotkeys themselves (Enter/Escape), never a
/// clickable button -- the overlay window is deliberately non-focusable.
async fn gate(app: &tauri::AppHandle, mode: Mode, autonomy: &str, description: &str) -> bool {
    match autonomy {
        "autonomous" => true,
        "confirm" => {
            emit_state(app, "acting", mode, Some(format!("{description} — Enter to confirm, Esc to cancel")));
            hotkeys::begin_pending_confirmation();
            let mut waited = Duration::ZERO;
            let proceed = loop {
                if hotkeys::abort_requested() {
                    break false;
                }
                match hotkeys::confirm_signal() {
                    1 => break true,
                    2 => break false,
                    _ => {}
                }
                if waited >= CONFIRM_TIMEOUT {
                    break false;
                }
                tokio::time::sleep(POLL_INTERVAL).await;
                waited += POLL_INTERVAL;
            };
            hotkeys::end_pending_confirmation();
            proceed
        }
        // "preview" (and the default for anything unrecognized -- the safer
        // choice if settings.json ever holds an unexpected value).
        _ => {
            emit_state(app, "acting", mode, Some(format!("{description} — Esc to cancel")));
            let mut waited = Duration::ZERO;
            while waited < PREVIEW_DELAY {
                if hotkeys::abort_requested() {
                    return false;
                }
                tokio::time::sleep(POLL_INTERVAL).await;
                waited += POLL_INTERVAL;
            }
            true
        }
    }
}
