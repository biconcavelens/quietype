use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};

const MAX_HISTORY: usize = 200;

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Settings {
    /// Path to the whisper.cpp ggml model used for local transcription.
    pub model_path: String,
    /// Anthropic key for assistant mode. Empty = fall back to ANTHROPIC_API_KEY.
    pub api_key: String,
    /// "system" | "light" | "dark" — resolved against the OS on the frontend.
    pub theme: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            model_path: default_model_path(),
            api_key: String::new(),
            theme: "system".to_string(),
        }
    }
}

impl Settings {
    /// Env var wins over the stored key so a shell-exported key keeps working
    /// without anyone having to retype it into the UI.
    pub fn resolved_api_key(&self) -> Option<String> {
        std::env::var("ANTHROPIC_API_KEY")
            .ok()
            .filter(|k| !k.trim().is_empty())
            .or_else(|| Some(self.api_key.clone()).filter(|k| !k.trim().is_empty()))
    }
}

fn default_model_path() -> String {
    if let Ok(p) = std::env::var("QUIETYPE_WHISPER_MODEL") {
        if !p.trim().is_empty() {
            return p;
        }
    }
    "models/ggml-base.en.bin".to_string()
}

#[derive(Serialize, Deserialize, Clone)]
pub struct HistoryEntry {
    /// Unix milliseconds; formatted for display on the frontend.
    pub at: u64,
    pub mode: String,
    /// What was actually said.
    pub transcript: String,
    /// What got typed (same as transcript for dictation).
    pub output: String,
}

fn config_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn read_json<T: for<'de> Deserialize<'de> + Default>(path: PathBuf) -> T {
    fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn write_json<T: Serialize>(path: PathBuf, value: &T) -> Result<(), String> {
    let raw = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    fs::write(path, raw).map_err(|e| e.to_string())
}

pub fn load_settings(app: &AppHandle) -> Settings {
    match config_dir(app) {
        Ok(dir) => read_json(dir.join("settings.json")),
        Err(_) => Settings::default(),
    }
}

pub fn save_settings(app: &AppHandle, settings: &Settings) -> Result<(), String> {
    write_json(config_dir(app)?.join("settings.json"), settings)
}

pub fn load_history(app: &AppHandle) -> Vec<HistoryEntry> {
    match config_dir(app) {
        Ok(dir) => read_json(dir.join("history.json")),
        Err(_) => Vec::new(),
    }
}

pub fn push_history(app: &AppHandle, mode: &str, transcript: &str, output: &str) {
    let entry = HistoryEntry {
        at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
        mode: mode.to_string(),
        transcript: transcript.to_string(),
        output: output.to_string(),
    };

    let mut history = load_history(app);
    history.insert(0, entry);
    history.truncate(MAX_HISTORY);

    if let Ok(dir) = config_dir(app) {
        let _ = write_json(dir.join("history.json"), &history);
    }
}

pub fn clear_history(app: &AppHandle) -> Result<(), String> {
    write_json(config_dir(app)?.join("history.json"), &Vec::<HistoryEntry>::new())
}
