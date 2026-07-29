import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { applyTheme, initTheme, type Theme } from "./theme";

interface HistoryEntry {
  at: number;
  mode: string;
  transcript: string;
  output: string;
}

/** Mirrors the Rust `Settings` struct, which serializes as snake_case. */
interface RawSettings {
  model_path: string;
  theme: Theme;
}

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

const modelPathEl = $<HTMLInputElement>("model-path");
const modelHintEl = $<HTMLParagraphElement>("model-hint");
const savedFlagEl = $<HTMLSpanElement>("saved-flag");
const historyListEl = $<HTMLDivElement>("history-list");
const themePickerEl = $<HTMLDivElement>("theme-picker");

/* ---- tabs ------------------------------------------------------------ */

document.querySelectorAll<HTMLButtonElement>(".tab").forEach((tab) => {
  tab.addEventListener("click", () => {
    document.querySelectorAll(".tab").forEach((t) => t.classList.remove("is-active"));
    document.querySelectorAll(".panel").forEach((p) => p.classList.remove("is-active"));
    tab.classList.add("is-active");
    $(`panel-${tab.dataset.tab}`).classList.add("is-active");
  });
});

/* ---- history --------------------------------------------------------- */

function relativeTime(ms: number): string {
  const seconds = Math.round((Date.now() - ms) / 1000);
  if (seconds < 60) return "just now";
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.round(hours / 24);
  if (days < 7) return `${days}d ago`;
  return new Date(ms).toLocaleDateString();
}

function showEmpty(title: string, detail: string, isError = false) {
  const empty = document.createElement("div");
  empty.className = isError ? "empty is-error" : "empty";
  const inner = document.createElement("div");
  const strong = document.createElement("strong");
  strong.textContent = title;
  const p = document.createElement("p");
  p.textContent = detail;
  inner.append(strong, p);
  empty.append(inner);
  historyListEl.replaceChildren(empty);
}

function renderHistory(entries: HistoryEntry[]) {
  if (entries.length === 0) {
    showEmpty("Nothing yet", "Hold Win+Ctrl anywhere and start talking.");
    return;
  }

  const cards = entries.map((entry) => {
    const card = document.createElement("div");
    card.className = "entry";

    const meta = document.createElement("div");
    meta.className = "entry-meta";
    const tag = document.createElement("span");
    tag.className = entry.mode === "assistant" ? "tag assistant" : "tag";
    tag.textContent = entry.mode;
    const when = document.createElement("span");
    when.textContent = relativeTime(entry.at);
    meta.append(tag, when);

    const output = document.createElement("div");
    output.className = "entry-output";
    output.textContent = entry.output;

    card.append(meta, output);

    // For assistant runs the instruction differs from the result, so show both.
    if (entry.mode === "assistant" && entry.transcript !== entry.output) {
      const said = document.createElement("div");
      said.className = "entry-said";
      said.textContent = `You said: ${entry.transcript}`;
      card.append(said);
    }

    const actions = document.createElement("div");
    actions.className = "entry-actions";
    const copy = document.createElement("button");
    copy.className = "btn";
    copy.textContent = "Copy";
    copy.addEventListener("click", async () => {
      await navigator.clipboard.writeText(entry.output);
      copy.textContent = "Copied";
      setTimeout(() => (copy.textContent = "Copy"), 1200);
    });
    actions.append(copy);
    card.append(actions);

    return card;
  });

  historyListEl.replaceChildren(...cards);
}

// Every path that can fail reports on screen. A silent catch here is what
// makes "history is empty" indistinguishable from "history failed to load".
async function loadHistory() {
  try {
    renderHistory(await invoke<HistoryEntry[]>("get_history"));
  } catch (err) {
    showEmpty("Couldn't load history", String(err), true);
  }
}

$("clear-history").addEventListener("click", async () => {
  try {
    await invoke("clear_history");
  } catch (err) {
    showEmpty("Couldn't clear history", String(err), true);
    return;
  }
  await loadHistory();
});

/* ---- settings -------------------------------------------------------- */

let current: RawSettings = {
  model_path: "",
  theme: "system",
};
let saveTimer: number | undefined;

async function refreshModelHint(path: string) {
  try {
    const exists = await invoke<boolean>("model_exists", { path });
    modelHintEl.className = `hint ${exists ? "ok" : "bad"}`;
    modelHintEl.textContent = exists
      ? "Model found — dictation runs entirely on this machine."
      : "No model at this path. Download a whisper.cpp ggml model and point here.";
  } catch (err) {
    modelHintEl.className = "hint bad";
    modelHintEl.textContent = String(err);
  }
}

function markThemeButton(theme: Theme) {
  themePickerEl.querySelectorAll<HTMLButtonElement>("button").forEach((btn) => {
    btn.classList.toggle("is-active", btn.dataset.themeValue === theme);
  });
}

async function loadSettings() {
  try {
    current = await invoke<RawSettings>("get_settings");
    modelPathEl.value = current.model_path;
    markThemeButton(current.theme ?? "system");
    await refreshModelHint(current.model_path);
  } catch (err) {
    modelHintEl.className = "hint bad";
    modelHintEl.textContent = `Couldn't load settings: ${err}`;
  }
}

function flashSaved() {
  savedFlagEl.classList.add("show");
  setTimeout(() => savedFlagEl.classList.remove("show"), 1200);
}

async function save() {
  try {
    await invoke("set_settings", { settings: current });
    flashSaved();
  } catch (err) {
    modelHintEl.className = "hint bad";
    modelHintEl.textContent = `Couldn't save: ${err}`;
  }
}

// Debounced so typing a path or key doesn't write a file per keystroke.
function queueSave() {
  window.clearTimeout(saveTimer);
  saveTimer = window.setTimeout(async () => {
    current = {
      ...current,
      model_path: modelPathEl.value.trim(),
    };
    await save();
    await refreshModelHint(current.model_path);
  }, 400);
}

modelPathEl.addEventListener("input", queueSave);

themePickerEl.addEventListener("click", async (event) => {
  const btn = (event.target as HTMLElement).closest<HTMLButtonElement>("button");
  const theme = btn?.dataset.themeValue as Theme | undefined;
  if (!theme) return;

  // Apply immediately so the click feels instant, then persist.
  applyTheme(theme);
  markThemeButton(theme);
  current = { ...current, theme };
  await save();
});

/* ---- boot ------------------------------------------------------------ */

listen("history-changed", loadHistory);

// Also refresh whenever the window comes back into view. The push event above
// is the fast path, but this window spends most of its life hidden in the
// tray, so it should never depend on having caught an event while it wasn't
// being looked at.
document.addEventListener("visibilitychange", () => {
  if (!document.hidden) loadHistory();
});
window.addEventListener("focus", loadHistory);

initTheme();
loadSettings();
loadHistory();
