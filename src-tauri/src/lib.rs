mod assistant;
mod audio;
mod inject;
mod transcribe;

use std::sync::Mutex;
use tauri::Manager;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    Dictate,
    Assistant,
}

struct AppState {
    active: Mutex<Option<(Mode, audio::RecordingHandle)>>,
}

// ponytail: two fixed global hotkeys, no settings UI yet. Add a config
// surface when a second person besides us needs different keys.
//
// Both use Ctrl as the trigger key (Windows' RegisterHotKey allows a
// modifier key as the actual trigger vk) so Windows+Ctrl and
// Windows+Shift+Ctrl read as natural chords rather than needing a letter key.
const HOTKEY_TRIGGER: Code = Code::ControlLeft;

fn dictate_modifiers() -> Modifiers {
    Modifiers::SUPER
}

fn assistant_modifiers() -> Modifiers {
    Modifiers::SUPER | Modifiers::SHIFT
}

fn toggle(app: &tauri::AppHandle, mode: Mode) {
    let state = app.state::<AppState>();
    let mut active = state.active.lock().unwrap();

    match active.take() {
        None => match audio::start_recording() {
            Ok(handle) => {
                eprintln!("[quietype] recording started ({mode:?})");
                *active = Some((mode, handle));
            }
            Err(e) => eprintln!("[quietype] failed to start recording: {e}"),
        },
        Some((active_mode, handle)) if active_mode == mode => {
            eprintln!("[quietype] recording stopped ({mode:?}), transcribing...");
            let samples = handle.stop();
            tauri::async_runtime::spawn(async move {
                process(mode, samples).await;
            });
        }
        Some(other) => {
            // A different mode is already recording; ignore this press and
            // put the still-active recording back.
            eprintln!("[quietype] ignoring {mode:?} hotkey, {:?} already recording", other.0);
            *active = Some(other);
        }
    }
}

async fn process(mode: Mode, samples: Vec<f32>) {
    let text = match transcribe::transcribe(&samples) {
        Ok(t) if !t.is_empty() => t,
        Ok(_) => {
            eprintln!("[quietype] empty transcription, nothing to do");
            return;
        }
        Err(e) => {
            eprintln!("[quietype] transcription failed: {e}");
            return;
        }
    };

    match mode {
        Mode::Dictate => {
            if let Err(e) = inject::type_text(&text) {
                eprintln!("[quietype] failed to inject dictated text: {e}");
            }
        }
        Mode::Assistant => {
            let context = inject::capture_selection().unwrap_or_default();
            match assistant::run(&text, &context).await {
                Ok(result) => {
                    if let Err(e) = inject::type_text(&result) {
                        eprintln!("[quietype] failed to inject assistant result: {e}");
                    }
                }
                Err(e) => eprintln!("[quietype] assistant call failed: {e}"),
            }
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AppState {
            active: Mutex::new(None),
        })
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state != ShortcutState::Pressed {
                        return;
                    }
                    if shortcut.matches(assistant_modifiers(), HOTKEY_TRIGGER) {
                        toggle(app, Mode::Assistant);
                    } else if shortcut.matches(dictate_modifiers(), HOTKEY_TRIGGER) {
                        toggle(app, Mode::Dictate);
                    }
                })
                .build(),
        )
        .setup(|app| {
            let handle = app.handle();
            handle
                .global_shortcut()
                .register(Shortcut::new(Some(dictate_modifiers()), HOTKEY_TRIGGER))?;
            handle
                .global_shortcut()
                .register(Shortcut::new(Some(assistant_modifiers()), HOTKEY_TRIGGER))?;
            eprintln!("[quietype] Win+Ctrl = dictate, Win+Shift+Ctrl = assistant");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
