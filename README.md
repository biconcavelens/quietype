# quietype

**Local-first, open-source voice dictation.** Speak into any app on Windows, macOS, or Linux — nothing leaves your machine unless you say so.

## Why

The dictation app market (Wispr Flow, Superwhisper, Aqua Voice, and others) is currently split into two camps, and nobody occupies both:

- **Cloud apps** (Wispr Flow, Aqua Voice, Willow) are polished but send your voice — and in at least one documented case, screenshots of your active window — to third-party servers. They also cost $12-15/month indefinitely, degrade in outages, and users repeatedly report a "great in the trial, worse after paying" pattern.
- **Local apps** (Superwhisper, VoiceInk, MacWhisper) protect privacy but are Mac-only, and several existing open-source alternatives (OpenFlow, OpenWhispr, FreeFlow) already chase this exact niche without matching the polish of the cloud incumbents.

quietype's bet: local-model transcription is now good enough (Parakeet/Whisper-class models) that there's no remaining excuse for cloud-only dictation, and the gap in the market is **execution quality on the local-first idea**, not the idea itself.

## Principles

1. **Local by default.** Transcription runs on-device. No audio or transcript leaves your machine unless you explicitly opt into a cloud/BYOK backend.
2. **Native, not Electron.** Tauri (Rust core + system webview) — small binaries, low idle memory, no 800MB-idle bloat.
3. **Transparent.** Open-source core. What the app captures and where it goes should be auditable, not a trust exercise.
4. **Streaming.** Text should appear as you speak, not only after you stop.
5. **Fair pricing, eventually.** Free local core; no plan to charge rent for something that runs on the user's own hardware.

## Status

Early scaffold. Not yet functional as a dictation app.

## Stack

- **Core:** Rust + [Tauri v2](https://tauri.app/)
- **UI:** Vanilla TypeScript (minimal, will evolve)
- **Transcription (planned):** local Whisper/Parakeet inference via Rust bindings, with an optional BYOK cloud backend for AI cleanup

## Development

```bash
npm install
npm run tauri dev
```

## Roadmap

- [ ] System-wide global hotkey capture (push-to-talk)
- [ ] Local model inference (whisper.cpp / Parakeet via ONNX)
- [ ] Streaming partial transcripts
- [ ] Text injection into focused app/field
- [ ] Verbatim vs. AI-cleanup toggle
- [ ] Custom vocabulary
- [ ] Windows + Linux packaging (not just macOS)
- [ ] Optional BYOK cloud cleanup pass

## License

MIT — see [LICENSE](LICENSE).
