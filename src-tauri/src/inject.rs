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
    // Windows' clipboard needs a beat to actually commit before Ctrl+V reads
    // it back -- below this, paste intermittently grabs the previous content.
    sleep(Duration::from_millis(15));

    paste()?;

    if let Some(prev) = previous {
        // Only needs to outlast the paste's own read of the clipboard.
        sleep(Duration::from_millis(60));
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
    // Assistant mode only, so this cost isn't on dictation's hot path. Kept
    // more conservative than the paste delay above: this waits on an
    // arbitrary target app to respond to Ctrl+C, not just our own paste.
    sleep(Duration::from_millis(80));

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
