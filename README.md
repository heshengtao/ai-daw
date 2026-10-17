<p align="center">
  <img src="./assets/readme/hero.svg" width="100%" alt="AI DAW — a local-first, AI-native digital audio workstation">
</p>

<p align="center">
  <img src="https://img.shields.io/badge/version-0.1.0-1A6CFF?style=flat-square" alt="Version 0.1.0">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-1A6CFF?style=flat-square" alt="License: Apache-2.0"></a>
  <img src="https://img.shields.io/badge/platforms-Windows%20/%20macOS%20/%20Linux-1A6CFF?style=flat-square" alt="Platforms: Windows, macOS, Linux">
</p>

<p align="center">
  <a href="README_zh.md"><b>简体中文</b></a> ·
  <a href="README.md"><b>English</b></a>
</p>

## What is AI DAW?

**AI DAW** is an open-source, **local-first digital audio workstation** built with **Tauri v2**, a **Rust audio engine**, and a **Vue 3** interface. Arrange, edit, mix, and master on your own machine — with the engine and every AI model running natively at your side, not in someone else's cloud.

The engine is written in Rust and talks to the audio device through `cpal`, moving samples on a lock-free ring buffer so the real-time thread never blocks. Arranging, transport, scheduling, buses, and time-stretching all live in the engine; the UI is a Material You–styled Vue 3 app that mirrors that state over a typed command/event contract.

Local AI runs **fully offline**. Tempo detection, audio-to-MIDI, voice cleanup, and stem separation are all ONNX models executed on-device — no account, no API key, no audio leaving your computer. A built-in **MCP server** (Model Context Protocol) then lets an AI agent read and edit the project directly, so assistants can drive the DAW for you.

## Features

### 🎹 Arranging & Editing — Compose on a Real Timeline

A familiar arrange view with multiple tracks, mini piano-roll clips, and audio waveform clips. Select, drag, split, copy, paste, and delete clips across tracks; the ruler and grid stay locked to a dynamic tempo and time signature you can change at any point in the song. A transport with loop, metronome, and a scrubable playhead ties it together, and automation lanes let you draw curves for anything a plugin or the built-in synth exposes.

- **Multi-Track Arranging** — MIDI and audio clips side by side, cross-track drag & drop, marquee select
- **Mini Piano Roll & Waveforms** — Clips render their own notes or waveform so you can read the song at a glance
- **Dynamic Tempo & Meter** — Tempo and time-signature changes anywhere on the timeline; the grid follows automatically
- **Transport & Loop** — Play, stop, seek, loop region, and a metronome
- **Automation Lanes** — Draw parameter curves, with a parameter browser to pick what to automate
- **Editing** — Split, move, copy/paste, undo/redo, plus a keyboard-shortcut cheat sheet

### 🎚️ Mixing & Mastering — Finish the Track

A mixer with per-track gain, pan, sends, buses, mute/solo, and meters, plus built-in effects. A mastering chain — loudness normalization, true-peak limiting, and stereo width — gives you a release-ready render without leaving the app.

- **Mixer** — Gain, pan, sends, buses, mute/solo, and meters
- **Built-in Effects** — Ready-to-use dynamics, EQ, and time-based effects
- **Mastering Chain** — Loudness normalization + true-peak limiting + stereo width
- **Built-in Synth** — Oscillators, envelopes, and a filter with a live oscilloscope

### 🧩 Plugins — Bring Your Own Instruments and Effects

Hosts **VST3** and **CLAP** plugins. A plugin manager searches the open registry, filters by type and OS compatibility, and installs, updates, or removes plugins in one click — or import a plugin you already have on disk.

- **VST3 & CLAP Hosting** — Load third-party instruments and effects on any track
- **Plugin Manager** — Search the open registry, filter by instrument/effect and compatibility, install / update / uninstall
- **Local Import** — Point the manager at a plugin you already have installed elsewhere
- **Plugin UI & Parameters** — Open a plugin's own editor, or drive its parameters from the inspector and automation lanes

### 🤖 Local AI Models — Offline, On Your Machine

Every AI feature is a local ONNX model that runs on CPU by default, with optional **CoreML** (macOS) and **DirectML** (Windows) execution providers when the hardware supports them. Weights download on demand from the in-app **model manager**; nothing is uploaded anywhere.

- **Tempo Detection** — Beat This! (ISMIR 2024) infers a global BPM and time signature in one pass
- **Audio-to-MIDI** — Basic Pitch turns a recorded performance into editable MIDI notes
- **Voice Cleanup** — LocalVQE reduces noise and reverb on voice takes
- **Stem Separation** — HTDemucs v4 splits a track into 4 or 6 stems
- **Execution Backends** — CPU by default; CoreML / DirectML where available

### 🔌 MCP Server — Let an AI Agent Drive the DAW

A built-in **MCP (Model Context Protocol)** server exposes the project to AI agents over Streamable HTTP, bound to `127.0.0.1` only. Enable it in **Settings → AI / MCP**, copy the endpoint, and connect your agent — it can add clips, edit notes, change gain, and more, with every change showing up live in the UI.

- **MCP over HTTP** — A loopback-only endpoint your agent connects to
- **Live Project Edits** — Agent actions appear immediately as normal project changes
- **`aidaw` CLI** — A thin MCP client you can install onto `PATH` from Settings

## Installation

Pre-built binaries will be published on the [GitHub Releases](https://github.com/heshengtao/ai-daw/releases) page. Until then, build from source.

### Requirements

- Rust (stable) with Cargo
- Node.js ≥ 18 and [pnpm](https://pnpm.io/)
- Tauri v2 system prerequisites — see <https://tauri.app/start/prerequisites/>

### Build from Source

```bash
git clone https://github.com/heshengtao/ai-daw.git
cd ai-daw

pnpm install
pnpm tauri dev        # run the app in development
pnpm tauri build      # produce a release build
```

Other useful commands:

```bash
pnpm build            # type-check the frontend and build it
pnpm build:aidaw      # build the aidaw CLI (see tools/build-aidaw.sh)
```

### Platform Support

| Platform | Status |
| --- | --- |
| Windows (x86_64) | ✅ Supported |
| macOS (Apple Silicon / Intel) | ✅ Supported |
| Linux (x86_64) | ✅ Supported |

## FAQ

### Is AI DAW free?

Yes. AI DAW is open source under Apache 2.0. There are no paid tiers and no feature gates.

### Does it work offline?

Yes. The engine, the UI, and every AI model run locally. The only network access is downloading plugin/model packages (which you control) and fetching the plugin registry.

### Do I need a GPU?

No. AI models run on CPU by default. macOS can use CoreML and Windows can use DirectML when the hardware supports it; both are optional.

### Which plugin formats are supported?

VST3 and CLAP.

### What can an AI agent do through the MCP server?

With the MCP server enabled, an agent can read the project and make edits — adding clips, editing notes, changing gain, and more — over a loopback-only HTTP endpoint. The `aidaw` CLI is a thin client for the same server.

### Where do the AI model weights live?

A small log-mel front-end ships with the app. Larger weights download on demand from the in-app model manager and are stored outside the repository, so they are never committed.

## License

Apache-2.0. See [LICENSE](LICENSE).

## Third-Party Licenses

This project builds on third-party open source software. Files adapted from other projects, and dependencies whose licenses are not in the permissive default set, are listed in [NOTICE](NOTICE).
