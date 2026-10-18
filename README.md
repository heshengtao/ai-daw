<p align="center">
  <img src="./assets/readme/hero.svg" width="100%" alt="AI DAW — a local-first, AI-native digital audio workstation">
</p>

<p align="center">
  <img src="https://img.shields.io/badge/version-1.3.0-1A6CFF?style=flat-square" alt="Version 1.3.0">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-1A6CFF?style=flat-square" alt="License: Apache-2.0"></a>
  <img src="https://img.shields.io/badge/platforms-Windows%20/%20macOS%20/%20Linux-1A6CFF?style=flat-square" alt="Platforms: Windows, macOS, Linux">
</p>

<p align="center">
  <a href="README_zh.md"><b>简体中文</b></a> ·
  <a href="README.md"><b>English</b></a>
</p>

## What is AI DAW?

**AI DAW is a small, AI-native music studio for your desktop** — write, arrange, mix, and master in one lightweight app, and let an AI agent work in the session alongside you.

It is a real DAW: a multi-track timeline, a piano roll, a mixer with sends and buses, automation lanes, a built-in synth, and VST3 / CLAP plugin hosting. It is also built to be **operated by a model** — a bundled Skill teaches any AI harness to drive the running app over MCP, so an agent can add tracks, write notes, balance a mix, and analyse the result while you watch it happen in the UI.

Everything runs on your own machine. No account, no API key, no audio leaving your computer.

https://github.com/user-attachments/assets/e0da890d-854d-4727-9cfb-3eb82c2a9bd1

## Why this matters

Models got good at writing code because they could **run** it through a coding-agent framework; models got good at 3D modelling because they could drive real modelling software through Blender MCP. In both cases the model gets a real tool, a real result, and a real feedback loop — and that is what makes fast iteration possible.

Music has mostly been missing that loop. A model can generate audio, but it cannot *work on a session* the way a person does: no arrangement to edit, no mixer to ride, no undo history, no way to notice that its last change made things worse.

**AI DAW is a small, deliberate attempt to close that gap.** It is a real DAW an AI agent can operate through a Skill and MCP, which makes it useful for:

- **Testing what a model actually knows about songwriting** — not whether its audio sounds plausible, but whether it can build an arrangement, choose parts, and fix a mix.
- **Collecting real songwriting traces** — every agent edit lands in a normal project with a normal undo history, so a session becomes data about how a model reasons about music.
- **A step toward models that can write music the way they write code** — with a tool that talks back.

## Lightweight by design

Being small is a feature, not a compromise.

- **A lightweight footprint** — no Electron, no cloud, no bundled model zoo. The macOS app is about 38 MB — for reference, mainstream DAWs such as Ableton Live, Logic Pro, and FL Studio typically install at 1–3 GB, making AI DAW **95%+ smaller**.
- **Fast to start** — nothing heavyweight loads at boot; AI models are pulled in only when you use them.
- **Heavy AI stays where it belongs** — high-compute work such as music generation and timbre transfer is deliberately **not** integrated locally. Your AI harness generates audio, and the result can be imported straight into the DAW for editing, mixing, and mastering.
- **Built to be extended** — interfaces for plugging external AI models into the workflow are on the roadmap.

## Drive it from any AI harness

AI DAW ships a **Skill** (`skills/aidaw`) that teaches an AI agent how to operate the running app — reading and writing the project, tracks, clips and notes; mixing (gain, pan, sends, automation); transport and recording; loudness and spectrum analysis; WAV export; plugin scanning. It works over the built-in **MCP** server, so any harness that supports Agent Skills can pick it up.

**Option 1 · Install from the command line**

```bash
npx skills add heshengtao/ai-daw
```

**Option 2 · Ask your agent to install it**

```
Install this Skill: https://github.com/heshengtao/ai-daw
```

Every edit an agent makes goes through the same command pipeline as the UI: it lands in the undo history, shows up in the interface, and is attributed to the agent — so you can always see what it did and undo it.

## Features

### 🎹 Arranging & Editing — Compose on a real timeline

Lay out a song the way you hear it. Drag clips across tracks, split them where the music changes, and let the ruler and grid follow a tempo and time signature you can change anywhere in the song. MIDI clips draw their own notes and audio clips draw their own waveform, so you can read the arrangement at a glance.

- **Multi-track arranging** — MIDI and audio clips side by side, cross-track drag & drop, marquee select
- **A piano roll that reads well** — clip previews show notes; the editor gives you velocity, zoom, and a keyboard
- **Dynamic tempo & meter** — change tempo or time signature at any point; the grid follows
- **Transport & loop** — play, stop, seek, loop a region, metronome
- **Automation lanes** — draw curves for anything a plugin or the built-in synth exposes
- **A real editing toolkit** — split, move, copy/paste, undo/redo, plus a keyboard-shortcut cheat sheet

<p align="center">
  <img src="./assets/screenshots/02-edit.png" width="100%" alt="The piano-roll editor showing a chord clip's notes and a velocity lane">
</p>

### 🎚️ Mixing & Mastering — Finish the track

Balance the song, then make it loud. Channel strips give you gain, pan, sends, buses, mute/solo and metering; a mastering chain on the master bus handles loudness, peaks, and stereo width so you can render something release-ready without leaving the app.

- **Mixer** — gain, pan, sends, buses, mute/solo, and level metering
- **Built-in effects** — dynamics, EQ, and time-based effects, ready to drop on any track
- **Mastering chain** — loudness normalization, true-peak limiting, and stereo width
- **Built-in synth** — oscillators, envelopes, and a filter with a live oscilloscope
- **Export** — lossless WAV or LAME-encoded MP3, with the sample rate and bitrate you pick

<p align="center">
  <img src="./assets/screenshots/03-mix.png" width="100%" alt="The mixer: channel strips for the master bus and each track, with faders, pan and mute/solo">
</p>

### 🧩 Plugins — Bring your own instruments and effects

AI DAW hosts **VST3** and **CLAP** plugins, so the tools you already own come with you. A plugin manager searches the open registry, filters by type and OS compatibility, and installs, updates, or removes plugins in one click — or point it at something you already have on disk.

- **VST3 & CLAP hosting** — load third-party instruments and effects on any track
- **Plugin manager** — search the registry, filter by instrument/effect and platform, install / update / uninstall
- **Local import** — bring in a plugin you have installed elsewhere
- **Editors & parameters** — open a plugin's own UI, or drive its parameters from the inspector and automation lanes

<p align="center">
  <img src="./assets/screenshots/04-plugins.png" width="100%" alt="The plugins panel: a built-in synth and effect list, with filters for built-in and external plugins">
</p>

### 🤖 Local AI — Turn audio into material you can work with

The AI features are built for real sessions, not demos. Point them at a track and they hand back something you can keep editing: MIDI notes you can move, a vocal take with less room noise, stems you can rebalance. Everything runs on your own hardware — CPU by default, with optional CoreML (macOS) and DirectML (Windows) acceleration.

- **Tempo detection** — Beat This! (ISMIR 2024) infers a global BPM and time signature in one pass
- **Audio-to-MIDI** — Basic Pitch turns a recorded performance into editable notes
- **Voice cleanup** — LocalVQE reduces noise and reverb on vocal takes
- **Stem separation** — HTDemucs v4 splits a track into 4 or 6 stems, ready to remix
- **Offline by design** — no account, no API key, no uploads

<p align="center">
  <img src="./assets/screenshots/05-ai-panel.png" width="100%" alt="The AI panel: MCP server status, plus entries for audio-to-MIDI, voice cleanup, stem separation and tempo detection">
</p>

### 🔌 MCP Server — Let an AI agent work in your session

Flip one switch and your project becomes something an AI assistant can operate. The built-in **MCP (Model Context Protocol)** server exposes the session over HTTP bound to `127.0.0.1` — connect your agent, hand it a task, and watch it add clips, fix notes, or change levels while the UI updates in real time. The `aidaw` command-line tool is a thin client for the same server.

- **MCP over HTTP** — a loopback-only endpoint your agent connects to
- **Live edits** — agent actions show up immediately as ordinary project changes
- **`aidaw` CLI** — installable onto your `PATH` from Settings

<p align="center">
  <img src="./assets/screenshots/06-settings-ai-mcp.png" width="100%" alt="Settings → AI / MCP: execution backend selection and the MCP server endpoint">
</p>

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
| macOS (Apple Silicon) | ✅ Supported (see the “app is damaged” note below) |
| Linux (x86_64 / arm64) | ✅ Supported |

> **macOS says “App is damaged”?** The app is ad-hoc signed (not notarized), so macOS
> quarantines downloads. Run this in Terminal, then open the app normally:
>
> ```bash
> xattr -cr "/Applications/AI DAW.app"
> ```

> **Intel Macs are not supported yet.** ONNX Runtime publishes no prebuilt binaries for
> `x86_64-apple-darwin`, so the local AI models cannot be linked for Intel macOS.

## FAQ

### Is AI DAW free?

Yes. AI DAW is open source under Apache 2.0. There are no paid tiers and no feature gates.

### Does it work offline?

Yes. The app and every AI model run locally. The only network access is downloading plugin packages and AI models (which you start), and fetching the plugin registry.

### Do I need a GPU?

No. AI models run on CPU by default. macOS can use CoreML and Windows can use DirectML when the hardware supports it; both are optional.

### Which AI models ship with the app?

Two small ones do: **Basic Pitch** (audio-to-MIDI) and **LocalVQE** (voice cleanup), so those work the moment you install. The larger models — **Beat This!** (tempo detection) and **HTDemucs** (stem separation, 4- or 6-stem) — download on demand from the in-app model manager the first time you use them, and are stored outside the source tree.

### Which plugin formats are supported?

VST3 and CLAP.

### Do I need the app running to let an agent drive it?

Yes. The Skill and the `aidaw` CLI talk to a running AI DAW over MCP, so start the app first (`aidaw status` will tell you if it is up).

## License

Apache-2.0. See [LICENSE](LICENSE).

## Third-Party Licenses

This project builds on third-party open source software. Files adapted from other projects, and dependencies whose licenses are not in the permissive default set, are listed in [NOTICE](NOTICE).
