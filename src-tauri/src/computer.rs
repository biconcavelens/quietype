//! Screen automation primitives for the assistant's agent loop: taking a
//! screenshot, clicking/typing/pressing keys, and launching applications.
//!
//! Known, unfixable gap: clicks and keystrokes sent here silently no-op
//! against a UAC-elevated window (installers, admin dialogs, etc.), since
//! quietype itself runs unelevated -- this is Windows' UIPI restriction on
//! unelevated processes sending input to elevated ones, not something any
//! amount of code here can work around.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use enigo::{Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};
use std::io::Cursor;
use std::process::Command;
use uiautomation::types::{ControlType, Handle, TreeScope};
use uiautomation::UIAutomation;
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
use xcap::Monitor;

fn capture_primary() -> Result<image::RgbaImage, String> {
    let monitors = Monitor::all().map_err(|e| e.to_string())?;
    let monitor = monitors
        .into_iter()
        .find(|m| m.is_primary().unwrap_or(false))
        .ok_or("No primary monitor found.")?;
    monitor.capture_image().map_err(|e| e.to_string())
}

fn encode_png(image: &image::RgbaImage) -> Result<Vec<u8>, String> {
    let mut bytes = Cursor::new(Vec::new());
    image
        .write_to(&mut bytes, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(bytes.into_inner())
}

/// Captures the primary monitor as PNG bytes, at native resolution.
///
/// Scoped to the primary monitor only: Tauri/tao already runs
/// Per-Monitor-DPI-Aware-V2 on Windows, so a screenshot's physical-pixel
/// coordinates map 1:1 to `click()`'s `Coordinate::Abs` on the *same*
/// monitor with no scaling math -- multi-monitor origin offsets are a
/// separate problem this doesn't attempt to solve.
pub fn screenshot_png() -> Result<Vec<u8>, String> {
    encode_png(&capture_primary()?)
}

/// Downscales a screenshot before sending it to the model -- Anthropic's own
/// computer-use docs warn that images above roughly XGA/WXGA resolution
/// (1024x768 / 1280x800) measurably hurt click accuracy and latency, and
/// this was sending full native resolution (2880x1800 on a typical laptop)
/// before, 2-3x over that ceiling. Returns the scaled PNG plus the scale
/// factor (native / sent) needed to map a model-reported coordinate on the
/// *fallback* `click(x, y)` path back to real screen pixels -- `click_element`
/// needs no such correction since UIA bounding boxes are already native.
pub fn screenshot_scaled_base64(max_dim: u32) -> Result<(String, f32), String> {
    let image = capture_primary()?;
    let (w, h) = (image.width(), image.height());
    let longest = w.max(h);
    if longest <= max_dim {
        return Ok((STANDARD.encode(encode_png(&image)?), 1.0));
    }

    let scale = longest as f32 / max_dim as f32;
    let (new_w, new_h) = ((w as f32 / scale) as u32, (h as f32 / scale) as u32);
    let resized = image::imageops::resize(&image, new_w, new_h, image::imageops::FilterType::Triangle);
    Ok((STANDARD.encode(encode_png(&resized)?), scale))
}

/// A clickable thing found on screen via Windows' accessibility tree, with
/// its real name/role/position -- the primary way the agent now grounds
/// clicks, rather than asking the model to guess pixel coordinates from a
/// picture. Every real computer-use agent (Microsoft's UFO, UiPath) grounds
/// this way first, falling back to vision-based `click(x, y)` only for
/// surfaces UIA can't see (canvases, games, some custom-drawn UI).
pub struct UiElement {
    pub name: String,
    pub role: String,
    pub x: i32,
    pub y: i32,
}

const INTERESTING_TYPES: &[ControlType] = &[
    ControlType::Button,
    ControlType::Edit,
    ControlType::ComboBox,
    ControlType::CheckBox,
    ControlType::RadioButton,
    ControlType::Hyperlink,
    ControlType::MenuItem,
    ControlType::ListItem,
    ControlType::TabItem,
];

/// Hard cap, not a ranking heuristic -- ponytail: revisit if real windows
/// routinely need more than this to describe what's interactable.
const MAX_ELEMENTS: usize = 50;

/// Enumerates interactable elements in the foreground window only (not the
/// whole desktop) -- scoping to one window is what keeps this call to tens
/// of milliseconds rather than the multi-hundred-ms cost of walking a full
/// subtree, per how Microsoft's own UFO agent does the same enumeration.
pub fn enumerate_elements() -> Result<Vec<UiElement>, String> {
    let hwnd = unsafe { GetForegroundWindow() };
    let automation = UIAutomation::new().map_err(|e| e.to_string())?;
    let root = automation
        .element_from_handle(Handle::from(hwnd.0 as isize))
        .map_err(|e| e.to_string())?;
    // The "control view" is UIA's own filter for meaningful/interactable
    // elements (excludes purely decorative/structural nodes) -- narrowing
    // further to INTERESTING_TYPES happens in plain Rust below rather than
    // building a combinator condition tree for a one-shot enumeration.
    let condition = automation
        .get_control_view_condition()
        .map_err(|e| e.to_string())?;
    let found = root
        .find_all(TreeScope::Descendants, &condition)
        .map_err(|e| e.to_string())?;

    let mut out = Vec::new();
    for el in found {
        let Ok(ctrl_type) = el.get_control_type() else { continue };
        if !INTERESTING_TYPES.contains(&ctrl_type) {
            continue;
        }
        let Ok(name) = el.get_name() else { continue };
        if name.trim().is_empty() {
            continue;
        }
        let Ok(rect) = el.get_bounding_rectangle() else { continue };
        let (w, h) = (
            rect.get_right() - rect.get_left(),
            rect.get_bottom() - rect.get_top(),
        );
        if w <= 0 || h <= 0 {
            continue;
        }
        out.push(UiElement {
            name,
            role: format!("{ctrl_type:?}"),
            x: rect.get_left() + w / 2,
            y: rect.get_top() + h / 2,
        });
        if out.len() >= MAX_ELEMENTS {
            break;
        }
    }
    Ok(out)
}

/// Clicks a real UI element's bounding-box center -- reuses `click()`
/// unchanged, just sourced from UIA instead of a model's pixel guess.
pub fn click_element(el: &UiElement) -> Result<(), String> {
    click(el.x, el.y)
}

/// Moves to and left-clicks an absolute screen coordinate (physical pixels,
/// primary-monitor space -- see `screenshot_png`'s doc comment).
pub fn click(x: i32, y: i32) -> Result<(), String> {
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
    enigo
        .move_mouse(x, y, Coordinate::Abs)
        .map_err(|e| e.to_string())?;
    enigo
        .button(Button::Left, Direction::Click)
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Presses a key or modifier combo like `"enter"`, `"esc"`, `"ctrl+a"`.
/// Unrecognized single characters fall back to a plain Unicode keypress.
pub fn key_combo(combo: &str) -> Result<(), String> {
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
    let parts: Vec<&str> = combo.split('+').map(|p| p.trim()).collect();
    let (modifiers, main_key) = parts.split_at(parts.len().saturating_sub(1));
    let main_key = main_key.first().copied().unwrap_or("");

    let mods: Vec<Key> = modifiers.iter().filter_map(|m| named_key(m)).collect();
    for m in &mods {
        enigo.key(*m, Direction::Press).map_err(|e| e.to_string())?;
    }

    let key = named_key(main_key).unwrap_or_else(|| {
        Key::Unicode(main_key.chars().next().unwrap_or(' '))
    });
    enigo.key(key, Direction::Click).map_err(|e| e.to_string())?;

    for m in mods.iter().rev() {
        enigo.key(*m, Direction::Release).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn named_key(name: &str) -> Option<Key> {
    match name.to_lowercase().as_str() {
        "ctrl" | "control" => Some(Key::Control),
        "shift" => Some(Key::Shift),
        "alt" => Some(Key::Alt),
        "win" | "meta" | "super" => Some(Key::Meta),
        "enter" | "return" => Some(Key::Return),
        "esc" | "escape" => Some(Key::Escape),
        "tab" => Some(Key::Tab),
        "space" => Some(Key::Space),
        "backspace" => Some(Key::Backspace),
        "delete" | "del" => Some(Key::Delete),
        _ => None,
    }
}

/// Launches an application by name via the same resolution Windows' Run
/// dialog uses (`start`) -- covers common exe names, registered app names,
/// and file/URL associations without needing per-app path lookup logic.
pub fn open_app(name: &str) -> Result<(), String> {
    // The empty "" argument is `start`'s window-title parameter, required
    // whenever the target itself might contain spaces or quotes.
    Command::new("cmd")
        .args(["/C", "start", "", name])
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Has real side effects (captures the actual screen) -- opt-in only:
    // `cargo test --lib computer::tests::screenshot_captures_primary_monitor -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn screenshot_captures_primary_monitor() {
        let png = screenshot_png().expect("screenshot should succeed");
        assert!(png.starts_with(b"\x89PNG"), "output should be a valid PNG");
        assert!(png.len() > 1000, "a real screen capture shouldn't be near-empty");
        std::fs::write("../screenshot_smoke_test.png", &png).expect("should write to disk");
        println!("wrote {} bytes to screenshot_smoke_test.png", png.len());
    }

    // Has real side effects (reads the actual foreground window) -- opt-in only:
    // `cargo test --lib computer::tests::enumerate_lists_real_elements -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn enumerate_lists_real_elements() {
        let elements = enumerate_elements().expect("enumeration should succeed");
        for el in &elements {
            println!("[{}] \"{}\" at ({}, {})", el.role, el.name, el.x, el.y);
        }
        assert!(!elements.is_empty(), "the foreground window should have at least one named control");
    }
}
