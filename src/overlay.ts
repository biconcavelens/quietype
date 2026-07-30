import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { initTheme } from "./theme";

type Phase = "idle" | "recording" | "transcribing" | "thinking" | "acting" | "done" | "error";

interface StateEvent {
  phase: Phase;
  mode: "dictate" | "assistant";
  text: string | null;
}

const stage = document.getElementById("stage") as HTMLDivElement;
const bubble = document.getElementById("bubble") as HTMLDivElement;
const bubbleText = document.getElementById("bubble-text") as HTMLSpanElement;
const badge = document.getElementById("badge") as HTMLDivElement;

/** Fixed labels for phases that don't carry their own text from the backend. */
const PHASE_LABEL: Partial<Record<Phase, string>> = {
  recording: "Listening",
  transcribing: "Transcribing",
  thinking: "Thinking",
};

function truncate(value: string, max: number): string {
  const clean = value.replace(/\s+/g, " ").trim();
  return clean.length > max ? `${clean.slice(0, max - 1)}…` : clean;
}

function apply(state: StateEvent) {
  stage.dataset.phase = state.phase;
  stage.dataset.mode = state.mode;
  badge.textContent = state.mode === "assistant" ? "Assistant" : "Dictate";

  // Idle carries no message -- the pet just sits there. Every other phase
  // shows either its own text (a result, an error, agent-loop narration) or
  // a fixed label for phases that don't carry one.
  const message =
    state.phase === "idle"
      ? ""
      : state.text
        ? truncate(state.text, state.phase === "error" ? 70 : 60)
        : (PHASE_LABEL[state.phase] ?? "");

  bubbleText.textContent = message;
  bubble.classList.toggle("show", message.length > 0);
}

listen<StateEvent>("overlay-state", (event) => {
  apply(event.payload);
});

// This window is created hidden, and WebView2 may not finish loading it until
// it's first shown -- by which point the state event that triggered the show
// has already fired with no listener attached. Pull the current state on load
// so the pet isn't stuck displaying stale markup.
invoke<StateEvent | null>("overlay_state")
  .then((state) => {
    if (state) apply(state);
  })
  .catch(() => {
    /* Nothing to catch up on. */
  });

initTheme();
