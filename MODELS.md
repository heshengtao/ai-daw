# Models, weights and licenses

This project ships **no model weights**. Neither the git repository nor the installers
contain any: weights are downloaded on demand from their publishers (Manager → Models)
into the user's own data directory, and the app verifies every **downloaded** file
against a pinned `sha256` before installing it.

That separation matters for licensing: **[LICENSE](LICENSE) (Apache-2.0) covers the
application code only.** Each set of weights stays under the license its publisher gave
it, and this file is the human-readable list of those licenses. The machine-readable
source of truth is the model catalog in
[`crates/local-ai/src/models/catalog.rs`](crates/local-ai/src/models/catalog.rs)
(`license` field of every entry), and the same license string is shown in the app's
model manager.
## Three separate layers

| Layer | Whose terms apply | Where it is stated |
|---|---|---|
| Application code | Apache-2.0, this project | [LICENSE](LICENSE) |
| Model weights | the model's publisher | this file + the publisher's model card |
| Audio / MIDI you generate | you — subject to the model's terms and to the rights you hold in the input | "Outputs" below |

Do not describe the whole product as "Apache-2.0 including its models": the code is
Apache-2.0, the weights are not ours to relicense.

## Bundled models

These small weights are shipped inside the app so the feature works on first launch.
They are permissively licensed and redistributed unmodified.

| Model | Feature | License | Publisher / source |
|---|---|---|---|
| Basic Pitch (`basic-pitch-nmp.onnx`) | audio → MIDI | Apache-2.0 | Spotify; ONNX export mirrored on Hugging Face (`AEmotionStudio/basic-pitch-onnx-models`) |
| LocalVQE (`localvqe-ns.onnx`) | vocal denoise / dereverb | Apache-2.0 | LocalAI-io; ONNX export produced by this project's `tools/export-localvqe-onnx.py` and mirrored on Hugging Face (`ailm32442/localvqe-ns-onnx`) |
| Beat This! log-mel frontend (`mel_spectrogram.onnx`) | tempo detection (front half) | MIT | danigb/beat-this-rs |

## On-demand models

Downloaded by the user, stored outside the source tree, never redistributed by this
project. Permissive licenses first; the non-commercial ones are called out separately.

| Model id | Feature | License | Publisher / source |
|---|---|---|---|
| `beat-this` | tempo detection | MIT | Beat This! (JKU Linz, ISMIR 2024); FP32 `final0` weights from `danigb/beat-this-rs` |
| `htdemucs-ort-v1` | stem separation (4 stems) | MIT | Meta Demucs v4 (`htdemucs`); ORT export mirrored on Hugging Face / ModelScope |
| `htdemucs-6s` | stem separation (6 stems) | MIT | Meta Demucs v4 (`htdemucs_6s`); self-contained ONNX export |
| `soulx-singer-fp16` / `soulx-singer-fp32` | singing voice | Apache-2.0 | SoulX-Singer (Soul AI Lab); ONNX split export on ModelScope, byte-identical Hugging Face mirror |
| `soulx-svc-fp16hq` / `soulx-svc-fp32` | voice swap (SVC) | Apache-2.0 | SoulX-Singer-SVC (Soul AI Lab); ONNX split export on ModelScope |
| `ace-step-turbo-q8` / `ace-step-turbo-bf16` | song generation | MIT | ACE-Step; GGUF weights from `audio-cpp/audio.cpp-gguf` (Hugging Face + ModelScope) |
| `qwen3-asr-1.7b` / `qwen3-asr-0.6b` | lyrics transcription (ASR) | Apache-2.0 | Qwen3-ASR (Alibaba); GGUF weights from `audio-cpp/audio.cpp-gguf` |
| `crisperwhisper-2-large` | lyrics transcription (ASR) | MIT | CrisperWhisper 2.0; GGUF weights from `audio-cpp/audio.cpp-gguf` |
| `muscriptor-small` / `-medium` / `-large` | audio → MIDI (multi-instrument) | **CC BY-NC 4.0** | Mirelo × Kyutai; GGUF conversions from `audio-cpp/audio.cpp-gguf` (Hugging Face + ModelScope), run by the `audiocpp` engine |

## Non-commercial models (CC BY-NC 4.0)

Three families are licensed **Creative Commons Attribution-NonCommercial 4.0
International** — [CC BY-NC 4.0](https://creativecommons.org/licenses/by-nc/4.0/).
They are optional, are not bundled, and are downloaded only when a user asks for them.
**You may not use these weights for commercial purposes**, and this project cannot grant
you such a right.

### YuE2 (song generation)

- Weights: `yue2-q8-0`, `yue2-q4-0`, `yue2-bf16` — released by **Multimodal Art
  Projection (M·A·P)** under CC BY-NC 4.0.
- Upstream **code** (inference tooling, the `audiocpp` engine) is Apache-2.0; only the
  weights are non-commercial. The engine itself is downloaded separately and is not
  bundled either.
- Sources: Hugging Face and ModelScope, repository `audio-cpp/Yue2-3B-GGUF`.
- Attribution to M·A·P and the HKUST collaborators is required for derived works.
- The weights may not be sublicensed, resold or used behind a paid service.
- **Outputs** follow the upstream terms: creators keep and may commercialize the tracks
  they generate (streaming, games, video, etc.), subject to ordinary copyright and
  Content-ID risk. Output rights do **not** make the weights commercial.

### SheetSage2 (cover / audio → score)

- Weights: `sheetsage2-orig` — CC BY-NC 4.0, published by the SheetSage2 authors.
- Sources: Hugging Face and ModelScope, repository `audio-cpp/SheetSage2-GGUF`.

### MuScriptor (multi-instrument audio → MIDI)

- Weights: `muscriptor-small`, `muscriptor-medium`, `muscriptor-large` — released by
  **Mirelo × Kyutai** under CC BY-NC 4.0 (the upstream repository is gated and asks for
  research use; user must hold the rights to the input audio).
- Sources: the GGUF conversions published by the audio.cpp project on Hugging Face and
  ModelScope, repository `audio-cpp/audio.cpp-gguf`, directory `MuScriptor-Small-GGUF/`
  (all three sizes live in that one directory).
- Run by the optional `audiocpp` engine (`--task midi --family muscriptor`), which is
  Apache-2.0; only the weights are non-commercial.
- Outputs: the MIDI notes landed in your project are yours, subject to the same
  input-rights requirement.
- Same restriction: non-commercial use of the weights only.

## Outputs

- MIDI, audio and score files you create with the app are yours. The model licenses do
  not restrict what you do with your own output, as long as you had the rights to the
  input audio.
- For music generation models the upstream publishers allow commercializing generated
  tracks (see YuE2 above), but no model can guarantee that an output does not resemble
  an existing work — that risk stays with you.
- Transcribing or covering a recording you do not have rights to is not permitted by any
  of these model licenses, whatever the local code does.

## Redistributing or building commercially

- The source repository and the official installers contain the code and the bundled
  permissive models only; if you fork and redistribute, keep it that way.
- Do not add the on-demand weights to your own repository, installer or Docker image.
  CC BY-NC weights cannot be redistributed as part of a commercial product.
- If you want a commercial build that includes the NC features, obtain a commercial
  license from the weight publisher (M·A·P for YuE2, the SheetSage2 authors), or ship
  the app without those models.
- Attribution: `NOTICE` lists the third-party code and assets this project redistributes;
  this file lists the weights it does not.

## Verifying the list

- Every entry in the catalog carries its SPDX-ish license string; the model manager shows
  it next to the size and the download source.
- Every downloaded file is checked against the `sha256` pinned in the catalog, so a
  mirrored or proxied download cannot silently replace the bytes.
- If a license here and a publisher's model card ever disagree, the publisher's model card
  wins — please open an issue so we can correct this file.
