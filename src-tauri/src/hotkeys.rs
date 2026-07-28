//! Press-and-hold hotkeys via a raw low-level keyboard hook.
//!
//! Windows' classic `RegisterHotKey` API (what `tauri-plugin-global-shortcut`
//! wraps) has no VK code for a bare modifier as the trigger key -- registering
//! "Ctrl alone" panics with "Unknown VKCode for ControlLeft". A pure Win+Ctrl
//! chord, with no third key, can only be done by watching raw key state
//! ourselves. That naturally gives press-to-talk too: key-down starts
//! recording, key-up (of either required key) stops it -- no toggle needed.

use crate::Mode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::AppHandle;
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, SetWindowsHookExW, TranslateMessage,
    UnhookWindowsHookEx, KBDLLHOOKSTRUCT, MSG, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP,
    WM_SYSKEYDOWN, WM_SYSKEYUP,
};

static WIN_DOWN: AtomicBool = AtomicBool::new(false);
static CTRL_DOWN: AtomicBool = AtomicBool::new(false);
static SHIFT_DOWN: AtomicBool = AtomicBool::new(false);

const VK_LWIN: u32 = 0x5B;
const VK_RWIN: u32 = 0x5C;
const VK_LCONTROL: u32 = 0xA2;
const VK_RCONTROL: u32 = 0xA3;
const VK_LSHIFT: u32 = 0xA0;
const VK_RSHIFT: u32 = 0xA1;

/// How long Win+Ctrl must be held alone before committing to Dictate, in case
/// Shift is on its way in to make it Win+Ctrl+Shift (Assistant) instead.
/// Below normal human key-press stagger, so it isn't felt as a delay.
const CHORD_GRACE: Duration = Duration::from_millis(120);
/// How often the controller thread samples key state.
const POLL_INTERVAL: Duration = Duration::from_millis(15);

/// Runs on the thread that installed the hook, synchronously, for every
/// keystroke system-wide. Must never block: just record state and return.
unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let msg = wparam.0 as u32;
        if matches!(msg, WM_KEYDOWN | WM_KEYUP | WM_SYSKEYDOWN | WM_SYSKEYUP) {
            let info = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
            let down = matches!(msg, WM_KEYDOWN | WM_SYSKEYDOWN);
            match info.vkCode {
                VK_LWIN | VK_RWIN => WIN_DOWN.store(down, Ordering::SeqCst),
                VK_LCONTROL | VK_RCONTROL => CTRL_DOWN.store(down, Ordering::SeqCst),
                VK_LSHIFT | VK_RSHIFT => SHIFT_DOWN.store(down, Ordering::SeqCst),
                _ => {}
            }
        }
    }
    // Always pass the event on -- we're observing, not consuming. Eating
    // Ctrl/Shift/Win keystrokes would break every other Ctrl+C, Shift+click,
    // etc. on the system.
    CallNextHookEx(None, code, wparam, lparam)
}

/// Installs the hook and pumps messages forever. Low-level hooks are only
/// delivered on the thread that installed them, and only while that thread is
/// running a message loop -- so this call never returns.
fn run_hook() {
    unsafe {
        let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), None, 0)
            .expect("failed to install keyboard hook");

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = UnhookWindowsHookEx(hook);
    }
}

/// Watches the key-state the hook maintains and drives recording start/stop.
/// Runs on its own thread so nothing here can stall the hook's message pump.
fn run_controller(app: AppHandle) {
    let mut recording: Option<(Mode, crate::audio::RecordingHandle)> = None;
    let mut dictate_candidate_since: Option<Instant> = None;

    loop {
        std::thread::sleep(POLL_INTERVAL);
        let win = WIN_DOWN.load(Ordering::SeqCst);
        let ctrl = CTRL_DOWN.load(Ordering::SeqCst);
        let shift = SHIFT_DOWN.load(Ordering::SeqCst);

        if let Some((mode, _)) = &recording {
            let still_held = match mode {
                Mode::Dictate => win && ctrl,
                Mode::Assistant => win && ctrl && shift,
            };
            if !still_held {
                if let Some((mode, handle)) = recording.take() {
                    crate::end_recording(&app, mode, handle);
                }
            }
            continue;
        }

        if win && ctrl && shift {
            dictate_candidate_since = None;
            if let Some(handle) = crate::begin_recording(&app, Mode::Assistant) {
                recording = Some((Mode::Assistant, handle));
            }
        } else if win && ctrl {
            match dictate_candidate_since {
                None => dictate_candidate_since = Some(Instant::now()),
                Some(since) if since.elapsed() >= CHORD_GRACE => {
                    dictate_candidate_since = None;
                    if let Some(handle) = crate::begin_recording(&app, Mode::Dictate) {
                        recording = Some((Mode::Dictate, handle));
                    }
                }
                _ => {}
            }
        } else {
            dictate_candidate_since = None;
        }
    }
}

/// Spawns the hook thread and its controller. Fire-and-forget: both run for
/// the lifetime of the app.
pub fn spawn(app: AppHandle) {
    std::thread::spawn(run_hook);
    std::thread::spawn(move || run_controller(app));
}
