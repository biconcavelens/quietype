use arboard::Clipboard;
use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use std::thread::sleep;
use std::time::Duration;

// ponytail: clipboard+paste is how every dictation app injects text (no
// per-platform Accessibility/UI Automation text-insertion API needed). We
// briefly clobber the user's clipboard and restore it after; there's a small
// window where a fast manual copy/paste could race this, acceptable for now.

/// Types `text` into whatever field currently has focus, via clipboard + simulated paste.
pub fn type_text(text: &str) -> Result<(), String> {
    let mut clipboard = Clipboard::new().map_err(|e| e.to_string())?;
    let previous = clipboard.get_text().ok();

    clipboard.set_text(text.to_string()).map_err(|e| e.to_string())?;
    sleep(Duration::from_millis(50));

    paste()?;

    if let Some(prev) = previous {
        sleep(Duration::from_millis(200));
        let _ = clipboard.set_text(prev);
    }
    Ok(())
}

/// Grabs the currently-selected text (if any) by simulating copy and reading the clipboard.
pub fn capture_selection() -> Result<String, String> {
    let mut clipboard = Clipboard::new().map_err(|e| e.to_string())?;
    let previous = clipboard.get_text().ok();
    let _ = clipboard.clear();

    copy()?;
    sleep(Duration::from_millis(150));

    let selected = clipboard.get_text().unwrap_or_default();

    if let Some(prev) = previous {
        let _ = clipboard.set_text(prev);
    }
    Ok(selected)
}

fn paste() -> Result<(), String> {
    key_combo('v')
}

fn copy() -> Result<(), String> {
    key_combo('c')
}

fn key_combo(key: char) -> Result<(), String> {
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
    enigo
        .key(Key::Control, Direction::Press)
        .map_err(|e| e.to_string())?;
    enigo
        .key(Key::Unicode(key), Direction::Click)
        .map_err(|e| e.to_string())?;
    enigo
        .key(Key::Control, Direction::Release)
        .map_err(|e| e.to_string())?;
    Ok(())
}
