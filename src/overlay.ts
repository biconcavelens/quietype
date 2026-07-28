import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { initTheme } from "./theme";

type Phase = "recording" | "transcribing" | "thinking" | "done" | "error";

interface StateEvent {
  phase: Phase;
  mode: "dictate" | "assistant";
  text: string | null;
}

const BAR_COUNT = 12;

const pill = document.getElementById("pill") as HTMLDivElement;
const wave = document.getElementById("wave") as HTMLDivElement;
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

for (let i = 0; i < BAR_COUNT; i++) {
  const bar = document.createElement("div");
  bar.className = "bar";
  // Staggered animation-delay per bar is what makes the CSS keyframe (in
  // overlay.css) read as a lively equalizer instead of every bar moving in
  // lockstep -- driven entirely by CSS, no per-frame JS involved.
  bar.style.animationDelay = `${(i / BAR_COUNT) * 0.6}s`;
  wave.appendChild(bar);
}

function truncate(value: string, max = 48): string {
  const clean = value.replace(/\s+/g, " ").trim();
  return clean.length > max ? `${clean.slice(0, max - 1)}…` : clean;
}

function apply(state: StateEvent) {
  const phaseClass = PHASE_CLASS[state.phase];
  if (!phaseClass) return;

  // Dropped mid-recording state (still recording, just no longer "active")
  // when a new phase arrives -- listening ends whenever a real state change
  // happens, so this can't get stuck on.
  wave.classList.remove("active");

  pill.className = `pill ${phaseClass}`;
  pill.dataset.mode = state.mode;
  badge.textContent = state.mode === "assistant" ? "Assistant" : "Dictate";

  if (state.phase === "done") {
    text.textContent = state.text ? truncate(state.text) : "Inserted";
  } else if (state.phase === "error") {
    text.textContent = state.text ? truncate(state.text, 56) : "Something went wrong";
  }
}

// Continuous mic-level streaming (100+ events/sec) never arrived at all --
// confirmed via a diagnostic counter that Tauri's plain emit/listen silently
// drops it at that rate, even after throttling to 30Hz. This only fires on
// a threshold crossing (silence <-> speech), which happens a few times a
// second at most -- comfortably inside the range overlay-state already
// proves works.
listen<boolean>("overlay-active", (event) => {
  wave.classList.toggle("active", event.payload);
});

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
