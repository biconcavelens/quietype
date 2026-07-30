# quietype

**Local-first, open-source desktop dictation + assistant.** Two press-and-hold hotkeys, one voice pipeline:

- **Hold `Win`+`Ctrl` — Dictate.** Speak, let go. Transcribed locally, typed verbatim wherever your cursor is.
- **Hold `Win`+`Ctrl`+`Shift` — Assistant.** Speak an instruction ("make this more professional", "answer this using my resume", "fill this form"), let go. Your selected text becomes context; for anything on-screen rather than in a text field, the assistant can look at the screen and actually click/type/press keys/open an app to do it, narrating what it's doing as it goes — not just text in, text out.

Both hotkeys share the same local speech-to-text engine — the only difference is what happens to the transcript: typed as-is, or handed to a local LLM as an instruction. Both are hold-to-talk: key-down starts recording, key-up stops it, no toggle to remember. Nothing in either path leaves the machine.

The speech-to-text engine itself is a Settings toggle: **Whisper** (a dedicated, fast speech model — the default) or **Gemma 4** (the same model/process assistant mode already uses, so nothing extra to download, but slower on CPU since it's a general multimodal model rather than one built specifically for speech).

## Why

The dictation app market (Wispr Flow, Superwhisper, Aqua Voice, and others) is currently split into two camps, and nobody occupies both:

- **Cloud apps** (Wispr Flow, Aqua Voice, Willow) are polished but send your voice — and in at least one documented case, screenshots of your active window — to third-party servers. They also cost $12-15/month indefinitely, degrade in outages, and users repeatedly report a "great in the trial, worse after paying" pattern.
- **Local apps** (Superwhisper, VoiceInk, MacWhisper) protect privacy but are Mac-only, and several existing open-source alternatives (OpenFlow, OpenWhispr, FreeFlow) already chase this exact niche without matching the polish of the cloud incumbents.

quietype's bet: local-model transcription is now good enough (Parakeet/Whisper-class models) that there's no remaining excuse for cloud-only dictation, and the gap in the market is **execution quality on the local-first idea**, not the idea itself.

## How it compares

| | quietype | Wispr Flow | Superwhisper |
|---|---|---|---|
| Processing | Local by default | Cloud only | Local |
| Platforms | Windows today, Linux/Mac targeted | Mac, Win, iOS, Android | Mac, iOS |
| Open source | Yes | No | No |
| Streaming transcription | Planned | No | No |
| Pricing | Free | $12–15/mo | $8.49/mo or $249 lifetime |
| Works offline | Yes | No | Yes |

Superwhisper already covers local + Mac well; quietype's edit is Windows/Linux-first, streaming, and fully open. Wispr Flow's edit is polish, at the cost of privacy, price, and offline reliability.

## Principles

1. **Local by default.** Transcription and the assistant both run on-device. No audio, transcript, or instruction leaves your machine.
2. **Native, not Electron.** Tauri (Rust core + system webview) — small binaries, low idle memory, no 800MB-idle bloat.
3. **Transparent.** Open-source core. What the app captures and where it goes should be auditable, not a trust exercise.
4. **Streaming.** Text should appear as you speak, not only after you stop.
5. **Fair pricing, eventually.** Free local core; no plan to charge rent for something that runs on the user's own hardware.

## Status

Working. Lives in the system tray with no window in the way: a small pet
avatar sits always-on-screen (idle when nothing's happening), showing the
current phase (listening, transcribing, thinking, acting, done) by color,
animation, and a speech bubble. A separate window holds history and
settings.

Not done yet: streaming partial transcripts, rebindable hotkeys, packaging.

## How it's built

- **No window on the hot path.** The app is a tray process. The pet avatar
  window is created once at startup and stays visible for the app's
  lifetime (idle by default) — so there's no window cold-start between
  pressing the hotkey and seeing feedback.
- **The pet's motion is all self-running CSS, never event-driven.** A live
  amplitude meter needs continuous IPC (100+ events/sec from the audio
  callback); even a much rarer threshold-crossing signal never rendered
  reliably. Idle bob/blink, a faster listening pulse, and phase-tinted glow
  are pure CSS `@keyframes` triggered only by a discrete phase change — the
  same lesson that killed an earlier waveform-visualizer attempt, applied
  here from the start instead of rediscovered.
- **Assistant mode is a small agentic loop, not one fixed request.** A plain
  text-edit instruction still resolves in a single model call, exactly as
  fast as before — the model only pays for a screenshot when it explicitly
  asks to look at the screen (`look_at_screen`), and `click` isn't even
  offered as an option until it has. That gating exists specifically to
  keep the model from ever clicking blind. Clicks/keystrokes/app-launches are
  gated by a Settings autonomy level (preview-then-proceed, fully
  autonomous, or confirm-every-step) enforced through the same low-level
  keyboard hook the hotkeys use — Enter confirms, Escape always cancels —
  since the pet window is deliberately non-focusable and can't rely on a
  clickable button.
- **Hotkeys are a raw low-level keyboard hook, not `RegisterHotKey`.** A pure
  modifier chord (Win+Ctrl, no third key) has no VK code to register as a
  trigger through Windows' classic hotkey API — watching raw key state
  ourselves is the only way to get both that chord *and* press-and-hold
  semantics (key-down starts recording, key-up stops it, no toggle). This is
  Windows-specific (`hotkeys.rs` calls Win32 directly) — the project is
  Windows-only again for the moment, until an equivalent low-level watcher
  exists for macOS (CGEventTap) and Linux (varies by X11/Wayland).
- **The overlay can't take focus** (`focusable(false)`). Text injection works by
  pasting into whatever window is focused, so an overlay that stole focus would
  paste into itself.
- **The model is loaded at startup**, not on first use, so the first dictation
  doesn't pay a one-off model-load cost on top of its own latency.
- **Transcription runs on a blocking thread pool**, off the async runtime.
- **Whisper's thread count is capped at 6, not "all available cores minus one."**
  On a hybrid CPU (performance cores + weaker efficiency cores — most current
  Intel/Apple/Qualcomm chips), ggml's thread pool has no concept of that
  heterogeneity: a synchronized parallel matmul waits on its slowest thread, so
  spilling work onto E-cores makes more threads actively *worse*. Measured on a
  16-core/22-thread Meteor Lake chip transcribing the same 2s clip: 21 threads
  → ~24s, 6 threads → ~6.9s, 4 threads → ~9.4s. Overridable via
  `QUIETYPE_THREADS` for tuning on other hardware.
- **Silence is trimmed before transcription.** Hold-to-talk removes the
  "reaction time before pressing stop" problem toggle-based recording had, but
  leading/trailing near-silence is still trimmed — it's what makes Whisper
  hallucinate tokens like `[BLANK_AUDIO]`, and less audio means less to
  transcribe.

## Stack

- **Core:** Rust + [Tauri v2](https://tauri.app/)
- **Audio capture:** [cpal](https://docs.rs/cpal)
- **Local transcription:** [whisper-rs](https://docs.rs/whisper-rs) (whisper.cpp bindings) — bring your own ggml model, see Setup
- **Text injection / selection capture:** clipboard + simulated copy/paste ([arboard](https://docs.rs/arboard) + [enigo](https://docs.rs/enigo)) — the same mechanism every dictation app uses under the hood
- **Assistant backend:** local Gemma 4 E4B, served by quietype's own bundled
  [llama-server](https://github.com/ggml-org/llama.cpp) (the official
  llama.cpp server) on `localhost:8090` — spawned and owned by the app itself
  (see `src-tauri/src/engine.rs`), not a separate service you have to run.
  Native tool-calling (`submit_result`, forced via `tool_choice: "required"`)
  plus an explicit "call it immediately, don't explain" system prompt is what
  actually keeps the output clean — verified directly that without both, the
  model either ignores the tool and answers conversationally, or rambles
  through a multi-option answer before eventually calling it. Running our own
  server instead of a wrapper like Ollama also gets native audio and image
  input for free (Gemma 4's multimodal projector, `--mmproj`) — the audio
  path is what powers Gemma as an alternative dictation engine (Settings);
  image input isn't wired into the UI yet, but the serving layer already
  supports it. One quirk worth knowing: a separate system-role message
  breaks Gemma's audio perception entirely (it claims no audio was given,
  even though it's present) — the no-preamble instruction has to be folded
  into the same user turn as the audio instead, which is what `engine::call`
  does differently for text vs. audio requests.
- **Screen automation:** clicks are grounded in real UI elements, not guessed
  pixels — [uiautomation](https://docs.rs/uiautomation) enumerates the
  foreground window's actual buttons/fields/links via Windows UI Automation
  (real name, role, exact bounding box), the model picks one by number, and
  quietype clicks its real center. This mirrors how Microsoft's own UFO
  agent and RPA tools like UiPath ground clicks — accessibility-tree first,
  vision only as a fallback for canvases/games/custom-drawn UI a small local
  model can't reliably pixel-click anyway. [xcap](https://docs.rs/xcap)
  provides that vision fallback's screenshots (primary monitor only — Tauri
  already runs Per-Monitor-DPI-Aware-V2 on Windows, so a capture's
  physical-pixel coordinates map 1:1 to a click with no scaling math needed
  *at native resolution* — screenshots are downscaled to ~1280px before
  being sent to the model per Anthropic's own computer-use accuracy
  guidance, so the fallback click path corrects for that scale factor before
  actually clicking). [enigo](https://docs.rs/enigo) drives the mouse
  (already a dependency for keyboard injection above — no new crate needed
  for mouse control). See `src-tauri/src/computer.rs`.

## Setup

1. Download a whisper.cpp ggml model, e.g. the base English model (~142MB) from
   the [official whisper.cpp model repo](https://huggingface.co/ggerganov/whisper.cpp),
   and place it at `src-tauri/models/ggml-base.en.bin`. Not bundled or
   auto-downloaded — you choose what runs on your machine. The path is editable
   in Settings, or via `QUIETYPE_WHISPER_MODEL`.
2. For assistant mode, download three things (dictation works without them;
   only assistant mode needs them):
   - A prebuilt CPU `llama-server` for Windows from the
     [llama.cpp releases page](https://github.com/ggml-org/llama.cpp/releases)
     (`llama-*-bin-win-cpu-x64.zip`) — unzip into `src-tauri/vendor/llama-server/`.
   - The Gemma 4 E4B GGUF weights and its multimodal projector from
     [ggml-org/gemma-4-E4B-it-GGUF](https://huggingface.co/ggml-org/gemma-4-E4B-it-GGUF)
     (`gemma-4-E4B-it-Q4_0.gguf`, ~4.6GB, and `mmproj-gemma-4-E4B-it-BF16.gguf`,
     ~1GB — the BF16 projector specifically, lower-precision ones measurably
     degrade the audio encoder) — both into `src-tauri/models/`.

   All three paths are overridable via `QUIETYPE_LLAMA_SERVER`,
   `QUIETYPE_LLAMA_MODEL`, and `QUIETYPE_LLAMA_MMPROJ` if you'd rather keep
   them elsewhere. The app spawns and health-checks the server at startup and
   keeps it resident for the app's lifetime — no idle-unload cost to repay
   between requests, unlike a shared service.
3. `npm install && npm run tauri dev`

## Roadmap

- [x] Global hotkeys, local inference, text injection, assistant mode
- [x] Tray app with floating overlay showing phase by color/text
- [x] History and settings UI
- [x] Personal context + custom vocabulary, fed into both dictation and assistant mode
- [ ] Streaming partial transcripts
- [ ] Verbatim vs. AI-cleanup toggle for dictation
- [x] Gemma 4 as an alternative dictation engine (audio-native, no separate model)
- [x] Always-visible pet avatar with idle/listening/thinking/acting states and a dialogue box
- [x] Screen-context capture beyond "selected text" (agent loop: look at the screen, click, type, press keys, open apps, gated by a Settings autonomy level)
- [ ] Arbitrary user-supplied image input for assistant mode (the agent loop's own screenshots are vision input already; attaching an image yourself isn't wired in)
- [ ] Rebindable hotkeys (currently fixed)
- [ ] Windows + Linux packaging (dev-only right now)

## Contributing

Too early for a formal process — the core dictation loop (hotkey → local transcription → text injection) doesn't exist yet. Open an issue if you want to discuss direction or help build it.

## License

MIT — see [LICENSE](LICENSE).
