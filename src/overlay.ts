import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { initTheme } from "./theme";

type Phase = "recording" | "transcribing" | "thinking" | "done" | "error";

interface StateEvent {
  phase: Phase;
  mode: "dictate" | "assistant";
  text: string | null;
}

const pill = document.getElementById("pill") as HTMLDivElement;
const text = document.getElementById("text") as HTMLDivElement;
const badge = document.getElementById("badge") as HTMLDivElement;

/** Maps backend phase names to the CSS class that drives the whole pill. */
const PHASE_CLASS: Record<Phase, string> = {
  recording: "listening",
  transcribing: "transcribing",
  thinking: "thinking",
  done: "done",
  error: "error",
};

/** Fixed labels for phases that don't carry their own text from the backend. */
const PHASE_LABEL: Partial<Record<Phase, string>> = {
  recording: "Listening",
  transcribing: "Transcribing",
  thinking: "Thinking",
};

function truncate(value: string, max = 48): string {
  const clean = value.replace(/\s+/g, " ").trim();
  return clean.length > max ? `${clean.slice(0, max - 1)}…` : clean;
}

function apply(state: StateEvent) {
  const phaseClass = PHASE_CLASS[state.phase];
  if (!phaseClass) return;

  pill.className = `pill ${phaseClass}`;
  pill.dataset.mode = state.mode;
  badge.textContent = state.mode === "assistant" ? "Assistant" : "Dictate";

  if (state.phase === "done") {
    text.textContent = state.text ? truncate(state.text) : "Inserted";
  } else if (state.phase === "error") {
    text.textContent = state.text ? truncate(state.text, 56) : "Something went wrong";
  } else {
    text.textContent = PHASE_LABEL[state.phase] ?? "";
  }
}

listen<StateEvent>("overlay-state", (event) => {
  apply(event.payload);
});

// This window is created hidden, and WebView2 may not finish loading it until
// it's first shown -- by which point the state event that triggered the show
// has already fired with no listener attached. Pull the current state on load
// so the first dictation isn't stuck displaying stale markup.
invoke<StateEvent | null>("overlay_state")
  .then((state) => {
    if (state) apply(state);
  })
  .catch(() => {
    /* Nothing to catch up on. */
  });

initTheme();
