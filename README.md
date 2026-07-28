# quietype

**Local-first, open-source desktop dictation + assistant.** Two press-and-hold hotkeys, one voice pipeline:

- **Hold `Win`+`Ctrl` — Dictate.** Speak, let go. Transcribed locally, typed verbatim wherever your cursor is.
- **Hold `Win`+`Ctrl`+`Shift` — Assistant.** Speak an instruction ("make this more professional", "answer this using my resume"), let go. Your selected text becomes context, Claude does the edit/fill, the result gets typed in.

Both hotkeys share the same local speech-to-text engine — the only difference is what happens to the transcript: typed as-is, or handed to an LLM as an instruction. Both are hold-to-talk: key-down starts recording, key-up stops it, no toggle to remember.

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

1. **Local by default.** Transcription runs on-device. No audio or transcript leaves your machine unless you explicitly opt into a cloud/BYOK backend.
2. **Native, not Electron.** Tauri (Rust core + system webview) — small binaries, low idle memory, no 800MB-idle bloat.
3. **Transparent.** Open-source core. What the app captures and where it goes should be auditable, not a trust exercise.
4. **Streaming.** Text should appear as you speak, not only after you stop.
5. **Fair pricing, eventually.** Free local core; no plan to charge rent for something that runs on the user's own hardware.

## Status

Working. Lives in the system tray with no window in the way: a floating pill
appears when you hold a hotkey, shows the current phase (listening,
transcribing, thinking, done) by color and label, and disappears once the
text lands. A separate window holds history and settings.

Not done yet: streaming partial transcripts, rebindable hotkeys, packaging.

## How it's built

- **No window on the hot path.** The app is a tray process. The only thing that
  appears during dictation is a 190×40 overlay pill, which is created once at
  startup and shown/hidden — so there's no window cold-start between pressing
  the hotkey and seeing feedback.
- **The overlay is plain color and text, not a waveform.** A live amplitude
  meter needs continuous IPC (100+ events/sec from the audio callback); even
  a much rarer threshold-crossing signal never rendered reliably. Phase
  color (border + dot) and a short label update instantly on each state
  change and have worked correctly throughout every version of this UI, so
  that's the whole design now rather than a fallback.
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
- **Assistant backend:** Anthropic API (BYOK), cloud by design since this is the one part of the pipeline that benefits from a frontier model

## Setup

1. Download a whisper.cpp ggml model, e.g. the base English model (~142MB) from
   the [official whisper.cpp model repo](https://huggingface.co/ggerganov/whisper.cpp),
   and place it at `src-tauri/models/ggml-base.en.bin`. Not bundled or
   auto-downloaded — you choose what runs on your machine. The path is editable
   in Settings, or via `QUIETYPE_WHISPER_MODEL`.
2. For assistant mode only, add an Anthropic API key in Settings (or export
   `ANTHROPIC_API_KEY`, which takes precedence). Dictation works fully offline
   without one.
3. `npm install && npm run tauri dev`

## Roadmap

- [x] Global hotkeys, local inference, text injection, assistant mode
- [x] Tray app with floating overlay showing phase by color/text
- [x] History and settings UI
- [ ] Streaming partial transcripts
- [ ] Verbatim vs. AI-cleanup toggle for dictation
- [ ] Custom vocabulary
- [ ] Rebindable hotkeys (currently fixed)
- [ ] Screen-context capture beyond "selected text" (for blank-field form filling)
- [ ] Windows + Linux packaging (dev-only right now)

## Contributing

Too early for a formal process — the core dictation loop (hotkey → local transcription → text injection) doesn't exist yet. Open an issue if you want to discuss direction or help build it.

## License

MIT — see [LICENSE](LICENSE).
