mod assistant;
mod audio;
mod hotkeys;
mod inject;
mod store;
pub mod transcribe;

use serde::Serialize;
use std::sync::{mpsc, Mutex, OnceLock};
use std::thread;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, LogicalPosition, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};

const MAIN_LABEL: &str = "main";
const OVERLAY_LABEL: &str = "overlay";
const OVERLAY_W: f64 = 296.0;
const OVERLAY_H: f64 = 52.0;
/// Gap between the overlay pill and the bottom of the screen.
const OVERLAY_BOTTOM_MARGIN: f64 = 110.0;

// WebView2 reads the system's WinINet proxy config even for localhost, so a
// stale proxy entry makes it hang loading the dev server. Force a direct
// connection. The msWebOOUI/msPdfOOUI flags are Tauri's own defaults, repeated
// here because setting this option replaces them.
const BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection \
     --proxy-server=direct:// --proxy-bypass-list=*";

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

#[derive(Serialize, Clone)]
struct OverlayState {
    /// recording | transcribing | thinking | done | error
    phase: &'static str,
    mode: Mode,
    text: Option<String>,
}

/// Last state we published, so a webview that loads mid-sequence can catch up.
static LAST_STATE: OnceLock<Mutex<Option<OverlayState>>> = OnceLock::new();

fn last_state() -> &'static Mutex<Option<OverlayState>> {
    LAST_STATE.get_or_init(|| Mutex::new(None))
}

fn emit_state(app: &AppHandle, phase: &'static str, mode: Mode, text: Option<String>) {
    let state = OverlayState { phase, mode, text };
    if let Ok(mut slot) = last_state().lock() {
        *slot = Some(state.clone());
    }
    // Broadcast rather than target a label: the overlay is the only listener
    // for this event, and targeting was one more thing to get wrong.
    let _ = app.emit("overlay-state", state);
}

/// The overlay window is created hidden at startup, and WebView2 doesn't
/// necessarily finish loading it until it's first shown -- so the very first
/// `overlay-state` event can fire before any listener exists, leaving the pill
/// stuck on its raw HTML defaults. The overlay pulls this on load so it can't
/// miss the state that was set while it was still booting.
#[tauri::command]
fn overlay_state() -> Option<OverlayState> {
    last_state().lock().ok().and_then(|s| s.clone())
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

/// Called by the hotkeys module when the required keys go down. Returns the
/// recording handle for the caller to hold until keys come back up -- there's
/// no shared "current recording" state here, the caller (hotkeys' controller
/// thread) is the sole owner of that for as long as it's active.
pub(crate) fn begin_recording(app: &AppHandle, mode: Mode) -> Option<audio::RecordingHandle> {
    let (level_tx, level_rx) = mpsc::channel::<f32>();
    match audio::start_recording(level_tx) {
        Ok(handle) => {
            show_overlay(app);
            emit_state(app, "recording", mode, None);

            // Forward mic levels to the overlay until capture ends and the
            // sender is dropped.
            let app_levels = app.clone();
            thread::spawn(move || {
                while let Ok(level) = level_rx.recv() {
                    let _ = app_levels.emit("overlay-level", level);
                }
            });

            Some(handle)
        }
        Err(e) => {
            show_overlay(app);
            emit_state(app, "error", mode, Some(e));
            fade_out(app.clone(), 2600);
            None
        }
    }
}

/// Called by the hotkeys module when a required key comes back up.
pub(crate) fn end_recording(app: &AppHandle, mode: Mode, handle: audio::RecordingHandle) {
    emit_state(app, "transcribing", mode, None);
    let samples = handle.stop();
    let app = app.clone();
    tauri::async_runtime::spawn(async move { process(app, mode, samples).await });
}

async fn process(app: AppHandle, mode: Mode, samples: Vec<f32>) {
    let t0 = std::time::Instant::now();
    let settings = store::load_settings(&app);

    let audio_ms = samples.len() as u128 * 1000 / 16_000;
    let model_path = settings.model_path.clone();
    // Whisper is CPU-bound; keep it off the async runtime's worker threads.
    let transcribed = tauri::async_runtime::spawn_blocking(move || {
        transcribe::transcribe(&samples, &model_path)
    })
    .await;
    eprintln!(
        "[quietype] {audio_ms}ms audio -> transcribed in {:?}",
        t0.elapsed()
    );

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

    eprintln!("[quietype] key-up to typed: {:?}", t0.elapsed());
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
    // The overlay is a separate window with its own copy of the theme, so it
    // has to be told; emitting to both keeps them from drifting apart.
    let _ = app.emit("settings-changed", &settings);
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
        .invoke_handler(tauri::generate_handler![
            get_settings,
            set_settings,
            get_history,
            clear_history,
            model_exists,
            overlay_state
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
                .tooltip("quietype — hold Win+Ctrl to dictate")
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

            hotkeys::spawn(handle.clone());

            // Pay the model-load cost now, not on the user's first dictation.
            transcribe::warm(&store::load_settings(&handle).model_path);

            eprintln!("[quietype] ready — hold Win+Ctrl to dictate, Win+Ctrl+Shift for assistant");
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
