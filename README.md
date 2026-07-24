# quietype

**Local-first, open-source desktop dictation + assistant.** Two hotkeys, one voice pipeline:

- **F9 — Dictate.** Press, speak, press again. Transcribed locally, typed verbatim wherever your cursor is.
- **F10 — Assistant.** Press, speak an instruction ("make this more professional", "answer this using my resume"), press again. Your selected text becomes context, Claude does the edit/fill, the result gets typed in.

Both hotkeys share the same local speech-to-text engine — the only difference is what happens to the transcript: typed as-is, or handed to an LLM as an instruction.

## Why

The dictation app market (Wispr Flow, Superwhisper, Aqua Voice, and others) is currently split into two camps, and nobody occupies both:

- **Cloud apps** (Wispr Flow, Aqua Voice, Willow) are polished but send your voice — and in at least one documented case, screenshots of your active window — to third-party servers. They also cost $12-15/month indefinitely, degrade in outages, and users repeatedly report a "great in the trial, worse after paying" pattern.
- **Local apps** (Superwhisper, VoiceInk, MacWhisper) protect privacy but are Mac-only, and several existing open-source alternatives (OpenFlow, OpenWhispr, FreeFlow) already chase this exact niche without matching the polish of the cloud incumbents.

quietype's bet: local-model transcription is now good enough (Parakeet/Whisper-class models) that there's no remaining excuse for cloud-only dictation, and the gap in the market is **execution quality on the local-first idea**, not the idea itself.

## How it compares

| | quietype | Wispr Flow | Superwhisper |
|---|---|---|---|
| Processing | Local by default | Cloud only | Local |
| Platforms | Windows, macOS, Linux | Mac, Win, iOS, Android | Mac, iOS |
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

First working harness: global hotkeys, local transcription, clipboard-based text injection, and an LLM-backed assistant mode. No streaming, no settings UI, no packaging yet.

## Stack

- **Core:** Rust + [Tauri v2](https://tauri.app/)
- **Audio capture:** [cpal](https://docs.rs/cpal)
- **Local transcription:** [whisper-rs](https://docs.rs/whisper-rs) (whisper.cpp bindings) — bring your own ggml model, see Setup
- **Text injection / selection capture:** clipboard + simulated copy/paste ([arboard](https://docs.rs/arboard) + [enigo](https://docs.rs/enigo)) — the same mechanism every dictation app uses under the hood
- **Assistant backend:** Anthropic API (BYOK — set `ANTHROPIC_API_KEY`), cloud by design since this is the one part of the pipeline that benefits from a frontier model

## Setup

1. Download a whisper.cpp ggml model, e.g. the base English model (~142MB) from the [official whisper.cpp model repo](https://huggingface.co/ggerganov/whisper.cpp), and place it at `src-tauri/models/ggml-base.en.bin` (or point `QUIETYPE_WHISPER_MODEL` at wherever you put it). Not bundled or auto-downloaded — you choose what runs on your machine.
2. Set `ANTHROPIC_API_KEY` in your environment (only needed for the F10 assistant hotkey; F9 dictation works fully offline without it).
3. `npm install && npm run tauri dev`

## Roadmap

- [x] Global hotkey capture (F9 dictate / F10 assistant)
- [x] Local model inference (whisper.cpp via whisper-rs)
- [x] Text injection into focused app/field
- [x] Assistant mode: selected text as context, LLM edit/fill, BYOK
- [ ] Streaming partial transcripts
- [ ] Verbatim vs. AI-cleanup toggle for dictation
- [ ] Custom vocabulary
- [ ] Configurable hotkeys (currently hardcoded F9/F10)
- [ ] Screen-context capture beyond "selected text" (for blank-field form filling)
- [ ] Windows + Linux packaging (dev-only right now)

## Contributing

Too early for a formal process — the core dictation loop (hotkey → local transcription → text injection) doesn't exist yet. Open an issue if you want to discuss direction or help build it.

## License

MIT — see [LICENSE](LICENSE).
