import { listen } from "@tauri-apps/api/event";
import { initTheme } from "./theme";

type Phase = "recording" | "transcribing" | "thinking" | "done" | "error";

interface StateEvent {
  phase: Phase;
  mode: "dictate" | "assistant";
  text: string | null;
}

const BAR_COUNT = 28;

const pill = document.getElementById("pill") as HTMLDivElement;
const glyph = document.getElementById("glyph") as HTMLDivElement;
const label = document.getElementById("label") as HTMLDivElement;
const wave = document.getElementById("wave") as HTMLDivElement;
const result = document.getElementById("result") as HTMLDivElement;
const badge = document.getElementById("badge") as HTMLDivElement;

const ICONS = {
  mic: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3z"/><path d="M19 10v2a7 7 0 0 1-14 0v-2"/><line x1="12" y1="19" x2="12" y2="22"/></svg>`,
  spinner: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round"><path d="M12 3a9 9 0 1 0 9 9" /></svg>`,
  check: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>`,
  alert: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9"/><line x1="12" y1="7" x2="12" y2="13"/><line x1="12" y1="17" x2="12" y2="17"/></svg>`,
};

// "recording" is the backend's word for it; the UI says "Listening", which
// reads as a state the app is in rather than something being saved.
const PHASES: Record<Phase, { css: string; label: string; icon: string }> = {
  recording: { css: "listening", label: "Listening", icon: ICONS.mic },
  transcribing: { css: "transcribing", label: "Transcribing", icon: ICONS.spinner },
  thinking: { css: "thinking", label: "Thinking", icon: ICONS.spinner },
  done: { css: "done", label: "Inserted", icon: ICONS.check },
  error: { css: "error", label: "Failed", icon: ICONS.alert },
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

// Mic levels arrive far faster than the screen refreshes, so the event
// handler only stores the newest value and rendering happens on rAF.
function render() {
  if (listening) {
    trail.push(level);
    trail.shift();
    for (let i = 0; i < BAR_COUNT; i++) {
      // Taper the edges so the waveform reads as a shape, not a block.
      const taper = Math.sin((i / (BAR_COUNT - 1)) * Math.PI) * 0.45 + 0.55;
      const v = trail[i] * taper;
      bars[i].style.height = `${2 + v * 18}px`;
      bars[i].style.opacity = `${0.28 + v * 0.72}`;
    }
  }
  requestAnimationFrame(render);
}
requestAnimationFrame(render);

function truncate(text: string, max = 60): string {
  const clean = text.replace(/\s+/g, " ").trim();
  return clean.length > max ? `${clean.slice(0, max - 1)}…` : clean;
}

function apply(state: StateEvent) {
  const phase = PHASES[state.phase];
  if (!phase) return;

  listening = state.phase === "recording";
  if (listening) trail.fill(0);

  pill.className = `pill ${phase.css}`;
  pill.dataset.mode = state.mode;
  glyph.innerHTML = phase.icon;
  label.textContent = phase.label;
  badge.textContent = state.mode === "assistant" ? "Assistant" : "Dictate";

  if (state.phase === "done") {
    result.textContent = state.text ? truncate(state.text) : "Done";
  } else if (state.phase === "error") {
    result.textContent = state.text ?? "Something went wrong";
  }
}

listen<number>("overlay-level", (event) => {
  level = event.payload;
});

listen<StateEvent>("overlay-state", (event) => {
  apply(event.payload);
});

initTheme();
