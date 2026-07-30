//! Press-and-hold hotkeys via a raw low-level keyboard hook.
//!
//! Windows' classic `RegisterHotKey` API (what `tauri-plugin-global-shortcut`
//! wraps) has no VK code for a bare modifier as the trigger key -- registering
//! "Ctrl alone" panics with "Unknown VKCode for ControlLeft". A pure Win+Ctrl
//! chord, with no third key, can only be done by watching raw key state
//! ourselves. That naturally gives press-to-talk too: key-down starts
//! recording, key-up (of either required key) stops it -- no toggle needed.

use crate::Mode;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
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

// Agent-loop control, driven by the same system-wide hook so neither needs
// window focus -- the overlay is deliberately non-focusable (stealing focus
// would break click targeting and paste-based text injection), so a
// confirm/cancel UI can't rely on a clickable button in it.
/// Escape sets this unconditionally, any time, regardless of autonomy mode
/// -- one flag serves both "cancel this pending action" and "abort a
/// runaway loop." agent.rs checks it at the top of every loop iteration.
static ABORT_AGENT: AtomicBool = AtomicBool::new(false);
/// Set by agent.rs while awaiting a decision in "confirm" autonomy mode.
static PENDING_ACTION: AtomicBool = AtomicBool::new(false);
/// 0 = waiting, 1 = confirmed (Enter), 2 = canceled (Escape). Only
/// meaningful while `PENDING_ACTION` is true.
static CONFIRM_SIGNAL: AtomicU8 = AtomicU8::new(0);

const VK_LWIN: u32 = 0x5B;
const VK_RWIN: u32 = 0x5C;
const VK_LCONTROL: u32 = 0xA2;
const VK_RCONTROL: u32 = 0xA3;
const VK_LSHIFT: u32 = 0xA0;
const VK_RSHIFT: u32 = 0xA1;
const VK_RETURN: u32 = 0x0D;
const VK_ESCAPE: u32 = 0x1B;

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
                VK_RETURN if down => {
                    if PENDING_ACTION.load(Ordering::SeqCst) {
                        CONFIRM_SIGNAL.store(1, Ordering::SeqCst);
                    }
                }
                VK_ESCAPE if down => {
                    ABORT_AGENT.store(true, Ordering::SeqCst);
                    if PENDING_ACTION.load(Ordering::SeqCst) {
                        CONFIRM_SIGNAL.store(2, Ordering::SeqCst);
                    }
                }
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

/// True if Escape has been pressed since the last `clear_abort()`. Checked
/// by the agent loop at the top of every iteration.
pub fn abort_requested() -> bool {
    ABORT_AGENT.load(Ordering::SeqCst)
}

/// Resets the abort flag -- called once at the start of a new agent run, so
/// a stale Escape from a previous task can't abort the next one immediately.
pub fn clear_abort() {
    ABORT_AGENT.store(false, Ordering::SeqCst);
}

/// Enters "confirm" autonomy mode's waiting state: resets the signal and
/// marks a decision as pending, so the hook's Enter/Escape branches start
/// producing a result.
pub fn begin_pending_confirmation() {
    CONFIRM_SIGNAL.store(0, Ordering::SeqCst);
    PENDING_ACTION.store(true, Ordering::SeqCst);
}

/// 0 = still waiting, 1 = confirmed, 2 = canceled.
pub fn confirm_signal() -> u8 {
    CONFIRM_SIGNAL.load(Ordering::SeqCst)
}

/// Leaves the waiting state -- call once a decision has been read, whether
/// by confirmation, cancellation, or timeout.
pub fn end_pending_confirmation() {
    PENDING_ACTION.store(false, Ordering::SeqCst);
}
