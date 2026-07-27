mod assistant;
mod audio;
mod inject;
mod store;
mod transcribe;

use serde::Serialize;
use std::sync::{mpsc, Mutex};
use std::thread;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, LogicalPosition, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

const MAIN_LABEL: &str = "main";
const OVERLAY_LABEL: &str = "overlay";
const OVERLAY_W: f64 = 340.0;
const OVERLAY_H: f64 = 64.0;
/// Gap between the overlay pill and the bottom of the screen.
const OVERLAY_BOTTOM_MARGIN: f64 = 110.0;

// ponytail: two fixed global hotkeys, no rebinding UI yet. A pure modifier
// chord (Win+Ctrl alone) can't be registered -- global-hotkey has no VK code
// for a bare modifier as trigger -- and Space collides with Windows' IME
// switcher, so Backquote is the trigger.
const HOTKEY_TRIGGER: Code = Code::Backquote;

// WebView2 reads the system's WinINet proxy config even for localhost, so a
// stale proxy entry makes it hang loading the dev server. Force a direct
// connection. The msWebOOUI/msPdfOOUI flags are Tauri's own defaults, repeated
// here because setting this option replaces them.
const BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection \
     --proxy-server=direct:// --proxy-bypass-list=*";

fn dictate_modifiers() -> Modifiers {
    Modifiers::SUPER | Modifiers::CONTROL
}

fn assistant_modifiers() -> Modifiers {
    Modifiers::SUPER | Modifiers::CONTROL | Modifiers::SHIFT
}

#[derive(Clone, Copy, PartialEq, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
enum Mode {
    Dictate,
    Assistant,
}

impl Mode {
    fn as_str(self) -> &'static str {
        match self {
            Mode::Dictate => "dictate",
            Mode::Assistant => "assistant",
        }
    }
}

struct AppState {
    active: Mutex<Option<(Mode, audio::RecordingHandle)>>,
}

#[derive(Serialize, Clone)]
struct OverlayState {
    /// recording | transcribing | thinking | done | error
    phase: &'static str,
    mode: Mode,
    text: Option<String>,
}

fn emit_state(app: &AppHandle, phase: &'static str, mode: Mode, text: Option<String>) {
    let _ = app.emit_to(OVERLAY_LABEL, "overlay-state", OverlayState { phase, mode, text });
}

fn show_overlay(app: &AppHandle) {
    let Some(win) = app.get_webview_window(OVERLAY_LABEL) else {
        return;
    };
    // Re-position each time: the primary monitor or its resolution may have
    // changed since the window was created.
    if let Ok(Some(monitor)) = win.primary_monitor() {
        let scale = monitor.scale_factor();
        let size = monitor.size().to_logical::<f64>(scale);
        let x = (size.width - OVERLAY_W) / 2.0;
        let y = size.height - OVERLAY_H - OVERLAY_BOTTOM_MARGIN;
        let _ = win.set_position(LogicalPosition::new(x, y));
    }
    let _ = win.show();
}

fn hide_overlay(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(OVERLAY_LABEL) {
        let _ = win.hide();
    }
}

fn show_main(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(MAIN_LABEL) {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
}

/// Hotkey handler: first press starts recording, second press for the same
/// mode stops it and kicks off processing.
fn toggle(app: &AppHandle, mode: Mode) {
    let state = app.state::<AppState>();
    let mut active = state.active.lock().unwrap();

    match active.take() {
        None => {
            let (level_tx, level_rx) = mpsc::channel::<f32>();
            match audio::start_recording(level_tx) {
                Ok(handle) => {
                    show_overlay(app);
                    emit_state(app, "recording", mode, None);

                    // Forward mic levels to the overlay until capture ends and
                    // the sender is dropped.
                    let app_levels = app.clone();
                    thread::spawn(move || {
                        while let Ok(level) = level_rx.recv() {
                            let _ = app_levels.emit_to(OVERLAY_LABEL, "overlay-level", level);
                        }
                    });

                    *active = Some((mode, handle));
                }
                Err(e) => {
                    show_overlay(app);
                    emit_state(app, "error", mode, Some(e));
                    fade_out(app.clone(), 2600);
                }
            }
        }
        Some((active_mode, handle)) if active_mode == mode => {
            emit_state(app, "transcribing", mode, None);
            let samples = handle.stop();
            let app = app.clone();
            tauri::async_runtime::spawn(async move { process(app, mode, samples).await });
        }
        Some(other) => {
            // The other mode is mid-recording; leave it running.
            *active = Some(other);
        }
    }
}

async fn process(app: AppHandle, mode: Mode, samples: Vec<f32>) {
    let settings = store::load_settings(&app);

    let model_path = settings.model_path.clone();
    // Whisper is CPU-bound; keep it off the async runtime's worker threads.
    let transcribed = tauri::async_runtime::spawn_blocking(move || {
        transcribe::transcribe(&samples, &model_path)
    })
    .await;

    let transcript = match transcribed {
        Ok(Ok(t)) if !t.is_empty() => t,
        Ok(Ok(_)) => {
            emit_state(&app, "error", mode, Some("Didn't catch that.".into()));
            return fade_out(app, 1800);
        }
        Ok(Err(e)) => {
            emit_state(&app, "error", mode, Some(e));
            return fade_out(app, 3600);
        }
        Err(e) => {
            emit_state(&app, "error", mode, Some(e.to_string()));
            return fade_out(app, 3600);
        }
    };

    let output = match mode {
        Mode::Dictate => transcript.clone(),
        Mode::Assistant => {
            emit_state(&app, "thinking", mode, Some(transcript.clone()));
            let context = inject::capture_selection().unwrap_or_default();
            let key = settings.resolved_api_key().unwrap_or_default();
            match assistant::run(&transcript, &context, &key).await {
                Ok(result) => result,
                Err(e) => {
                    emit_state(&app, "error", mode, Some(e));
                    return fade_out(app, 4000);
                }
            }
        }
    };

    emit_state(&app, "done", mode, Some(output.clone()));

    if let Err(e) = inject::type_text(&output) {
        emit_state(&app, "error", mode, Some(format!("Couldn't type that: {e}")));
        return fade_out(app, 3600);
    }

    store::push_history(&app, mode.as_str(), &transcript, &output);
    let _ = app.emit_to(MAIN_LABEL, "history-changed", ());
    fade_out(app, 900);
}

/// Leaves the overlay up briefly so the final state is readable, then hides it.
fn fade_out(app: AppHandle, after_ms: u64) {
    thread::spawn(move || {
        thread::sleep(std::time::Duration::from_millis(after_ms));
        hide_overlay(&app);
    });
}

#[tauri::command]
fn get_settings(app: AppHandle) -> store::Settings {
    store::load_settings(&app)
}

#[tauri::command]
fn set_settings(app: AppHandle, settings: store::Settings) -> Result<(), String> {
    store::save_settings(&app, &settings)?;
    transcribe::warm(&settings.model_path);
    Ok(())
}

#[tauri::command]
fn get_history(app: AppHandle) -> Vec<store::HistoryEntry> {
    store::load_history(&app)
}

#[tauri::command]
fn clear_history(app: AppHandle) -> Result<(), String> {
    store::clear_history(&app)
}

#[tauri::command]
fn model_exists(path: String) -> bool {
    !path.trim().is_empty() && std::path::Path::new(&path).exists()
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
                    // Assistant first: its modifier set is a superset of dictate's.
                    if shortcut.matches(assistant_modifiers(), HOTKEY_TRIGGER) {
                        toggle(app, Mode::Assistant);
                    } else if shortcut.matches(dictate_modifiers(), HOTKEY_TRIGGER) {
                        toggle(app, Mode::Dictate);
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            get_settings,
            set_settings,
            get_history,
            clear_history,
            model_exists
        ])
        .setup(|app| {
            let handle = app.handle().clone();

            WebviewWindowBuilder::new(app, MAIN_LABEL, WebviewUrl::App("index.html".into()))
                .title("quietype")
                .inner_size(940.0, 660.0)
                .min_inner_size(760.0, 540.0)
                .visible(false)
                .additional_browser_args(BROWSER_ARGS)
                .build()?;

            // The overlay must never take focus: text injection pastes into
            // whatever window is focused, so stealing focus would paste into us.
            WebviewWindowBuilder::new(app, OVERLAY_LABEL, WebviewUrl::App("overlay.html".into()))
                .inner_size(OVERLAY_W, OVERLAY_H)
                .decorations(false)
                .transparent(true)
                .always_on_top(true)
                .skip_taskbar(true)
                .shadow(false)
                .resizable(false)
                .focused(false)
                .focusable(false)
                .visible(false)
                .additional_browser_args(BROWSER_ARGS)
                .build()?;

            let open_item = MenuItem::with_id(app, "open", "Open quietype", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(
                app,
                &[&open_item, &PredefinedMenuItem::separator(app)?, &quit_item],
            )?;

            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("quietype — Win+Ctrl+` to dictate")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => show_main(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_main(tray.app_handle());
                    }
                })
                .build(app)?;

            handle
                .global_shortcut()
                .register(Shortcut::new(Some(dictate_modifiers()), HOTKEY_TRIGGER))?;
            handle
                .global_shortcut()
                .register(Shortcut::new(Some(assistant_modifiers()), HOTKEY_TRIGGER))?;

            // Pay the model-load cost now, not on the user's first dictation.
            transcribe::warm(&store::load_settings(&handle).model_path);

            eprintln!("[quietype] ready — Win+Ctrl+` dictate, Win+Ctrl+Shift+` assistant");
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the main window returns to the tray instead of quitting.
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == MAIN_LABEL {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
