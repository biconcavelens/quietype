import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { initTheme } from "./theme";

type Phase = "recording" | "transcribing" | "thinking" | "done" | "error";

interface StateEvent {
  phase: Phase;
  mode: "dictate" | "assistant";
  text: string | null;
}

const BAR_COUNT = 34;

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

const bars: HTMLDivElement[] = [];
for (let i = 0; i < BAR_COUNT; i++) {
  const bar = document.createElement("div");
  bar.className = "bar";
  wave.appendChild(bar);
  bars.push(bar);
}

/** Latest level from the audio thread, sampled once per frame. */
let level = 0;
let listening = false;
const trail: number[] = new Array(BAR_COUNT).fill(0);

// Mic levels arrive far faster than the screen refreshes, so the event handler
// only stores the newest value and rendering happens on rAF.
function render() {
  if (listening) {
    trail.push(level);
    trail.shift();
    for (let i = 0; i < BAR_COUNT; i++) {
      // Taper the ends so the waveform reads as a shape, not a block.
      const taper = Math.sin((i / (BAR_COUNT - 1)) * Math.PI) * 0.5 + 0.5;
      bars[i].style.height = `${3 + trail[i] * taper * 16}px`;
    }
  }
  requestAnimationFrame(render);
}
requestAnimationFrame(render);

function truncate(value: string, max = 48): string {
  const clean = value.replace(/\s+/g, " ").trim();
  return clean.length > max ? `${clean.slice(0, max - 1)}…` : clean;
}

function apply(state: StateEvent) {
  const phaseClass = PHASE_CLASS[state.phase];
  if (!phaseClass) return;

  listening = state.phase === "recording";
  if (listening) trail.fill(0);

  pill.className = `pill ${phaseClass}`;
  pill.dataset.mode = state.mode;
  badge.textContent = state.mode === "assistant" ? "Assistant" : "Dictate";

  if (state.phase === "done") {
    text.textContent = state.text ? truncate(state.text) : "Inserted";
  } else if (state.phase === "error") {
    text.textContent = state.text ? truncate(state.text, 56) : "Something went wrong";
  }
}

listen<number>("overlay-level", (event) => {
  level = event.payload;
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
