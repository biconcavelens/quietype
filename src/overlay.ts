import { listen } from "@tauri-apps/api/event";

type Phase = "recording" | "transcribing" | "thinking" | "done" | "error";

interface StateEvent {
  phase: Phase;
  mode: "dictate" | "assistant";
  text: string | null;
}

const BAR_COUNT = 28;

const pill = document.getElementById("pill") as HTMLDivElement;
const wave = document.getElementById("wave") as HTMLDivElement;
const status = document.getElementById("status") as HTMLDivElement;
const badge = document.getElementById("badge") as HTMLDivElement;

const bars: HTMLDivElement[] = [];
for (let i = 0; i < BAR_COUNT; i++) {
  const bar = document.createElement("div");
  bar.className = "bar";
  wave.appendChild(bar);
  bars.push(bar);
}

/** Latest level from the audio thread, sampled by the animation loop. */
let level = 0;
let phase: Phase = "recording";
const trail: number[] = new Array(BAR_COUNT).fill(0);

// Mic levels arrive far faster than the screen refreshes, so the event handler
// only stores the newest value and rendering happens once per frame.
function render() {
  if (phase === "recording") {
    trail.push(level);
    trail.shift();
    for (let i = 0; i < BAR_COUNT; i++) {
      // Taper the edges so the waveform reads as a shape rather than a block.
      const taper = Math.sin((i / (BAR_COUNT - 1)) * Math.PI) * 0.45 + 0.55;
      const v = trail[i] * taper;
      bars[i].style.height = `${3 + v * 22}px`;
      bars[i].style.opacity = `${0.3 + v * 0.7}`;
    }
  }
  requestAnimationFrame(render);
}
requestAnimationFrame(render);

function truncate(text: string, max = 64): string {
  const clean = text.replace(/\s+/g, " ").trim();
  return clean.length > max ? `${clean.slice(0, max - 1)}…` : clean;
}

function apply(state: StateEvent) {
  phase = state.phase;
  pill.className = `pill ${state.phase}`;
  pill.dataset.mode = state.mode;
  badge.textContent = state.mode === "assistant" ? "Assistant" : "Dictate";

  switch (state.phase) {
    case "recording":
      trail.fill(0);
      status.textContent = "Listening";
      break;
    case "transcribing":
      status.textContent = "Transcribing";
      break;
    case "thinking":
      status.textContent = "Thinking";
      break;
    case "done":
      status.textContent = state.text ? truncate(state.text) : "Done";
      break;
    case "error":
      status.textContent = state.text ?? "Something went wrong";
      break;
  }
}

listen<number>("overlay-level", (event) => {
  level = event.payload;
});

listen<StateEvent>("overlay-state", (event) => {
  apply(event.payload);
});
