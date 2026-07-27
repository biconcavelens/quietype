import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

interface Settings {
  modelPath: string;
  apiKey: string;
  sound: boolean;
}

interface HistoryEntry {
  at: number;
  mode: string;
  transcript: string;
  output: string;
}

// serde uses snake_case on the Rust side.
interface RawSettings {
  model_path: string;
  api_key: string;
  sound: boolean;
}

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

const modelPathEl = $<HTMLInputElement>("model-path");
const modelHintEl = $<HTMLParagraphElement>("model-hint");
const apiKeyEl = $<HTMLInputElement>("api-key");
const soundEl = $<HTMLInputElement>("sound");
const savedFlagEl = $<HTMLSpanElement>("saved-flag");
const historyListEl = $<HTMLDivElement>("history-list");

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
  return new Date(ms).toLocaleDateString();
}

function renderHistory(entries: HistoryEntry[]) {
  historyListEl.replaceChildren();

  if (entries.length === 0) {
    const empty = document.createElement("div");
    empty.className = "empty";
    const inner = document.createElement("div");
    const strong = document.createElement("strong");
    strong.textContent = "Nothing yet";
    const p = document.createElement("p");
    p.textContent = "Press Win+Ctrl+` anywhere and start talking.";
    inner.append(strong, p);
    empty.append(inner);
    historyListEl.append(empty);
    return;
  }

  for (const entry of entries) {
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

    historyListEl.append(card);
  }
}

async function loadHistory() {
  renderHistory(await invoke<HistoryEntry[]>("get_history"));
}

$("clear-history").addEventListener("click", async () => {
  await invoke("clear_history");
  await loadHistory();
});

/* ---- settings -------------------------------------------------------- */

let current: Settings = { modelPath: "", apiKey: "", sound: false };
let saveTimer: number | undefined;

async function refreshModelHint(path: string) {
  const exists = await invoke<boolean>("model_exists", { path });
  modelHintEl.className = `hint ${exists ? "ok" : "bad"}`;
  modelHintEl.textContent = exists
    ? "Model found — dictation runs entirely on this machine."
    : "No model at this path. Download a whisper.cpp ggml model and point here.";
}

async function loadSettings() {
  const raw = await invoke<RawSettings>("get_settings");
  current = { modelPath: raw.model_path, apiKey: raw.api_key, sound: raw.sound };
  modelPathEl.value = current.modelPath;
  apiKeyEl.value = current.apiKey;
  soundEl.checked = current.sound;
  await refreshModelHint(current.modelPath);
}

function flashSaved() {
  savedFlagEl.classList.add("show");
  setTimeout(() => savedFlagEl.classList.remove("show"), 1200);
}

// Debounced so typing a path or key doesn't write a file per keystroke.
function queueSave() {
  window.clearTimeout(saveTimer);
  saveTimer = window.setTimeout(async () => {
    current = {
      modelPath: modelPathEl.value.trim(),
      apiKey: apiKeyEl.value.trim(),
      sound: soundEl.checked,
    };
    await invoke("set_settings", {
      settings: {
        model_path: current.modelPath,
        api_key: current.apiKey,
        sound: current.sound,
      },
    });
    await refreshModelHint(current.modelPath);
    flashSaved();
  }, 400);
}

modelPathEl.addEventListener("input", queueSave);
apiKeyEl.addEventListener("input", queueSave);
soundEl.addEventListener("change", queueSave);

/* ---- boot ------------------------------------------------------------ */

listen("history-changed", loadHistory);

loadSettings();
loadHistory();
