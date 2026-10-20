# Built-in instrument assets

All sample assets in this directory are third-party works redistributed under their
original licences. They are **data**, not source code; they are loaded by
`ai-daw-instruments` through `rustysynth-ext`.

## piano.sf3

- Source: FreePats — **Upright Piano KW (small)**
- Author: Gonzalo / Roberto (FreePats project)
- Licence: **CC0 1.0 Universal**
- Source URL: https://freepats.zenvoid.org/Piano/acoustic-grand-piano.html
- Modification: converted from SF2 to SF3 (Ogg Vorbis, quality 0.8); the `smpl` chunk
  was normalized to an even size for the pure-Rust reader.

## nylon_guitar.sf3

- Source: FreePats — **Spanish classical guitar**
- Author: Roberto (FreePats project)
- Licence: **CC0 1.0 Universal**
- Source URL: https://freepats.zenvoid.org/Guitar/acoustic-guitar.html
- Modification: converted from SF2 to SF3 (Ogg Vorbis, quality 0.8).

## electric_guitar.sf3

- Source: FreePats — **FSBS Electric Guitar Clean #1 (small, bridge pickups)**
- Author: FreePats project / FSBS
- Licence: **CC0 1.0 Universal**
- Source URL: https://freepats.zenvoid.org/ElectricGuitar/clean-electric-guitar.html
- Modification: converted from SF2 to SF3 (Ogg Vorbis, quality 0.8); the `smpl`
  chunk was normalized to an even size for the pure-Rust reader.

## bass.sf3

- Source: FreePats — **Bass Guitar YR (finger)**
- Author: Andrea Biasior (reusenoise@gmail.com), assembled by FreePats
- Licence: **CC0 1.0 Universal**
- Source URL: https://freepats.zenvoid.org/ElectricGuitar/clean-electric-bass.html
- Modification: converted from SF2 to SF3 (Ogg Vorbis, quality 0.8); the `smpl`
  chunk was normalized to an even size for the pure-Rust reader.

## drums.sf2

- Source: **MuldjordKit (FreePats version)**
- Author: Lars Muldjord; FreePats assembly by Roberto
- Licence: **Creative Commons Attribution 4.0 International (CC BY 4.0)**
- Source URL: https://freepats.zenvoid.org/Percussion/acoustic-drum-kit.html
- Modification: reduced from the full 256-velocity-layer stereo SF2 to a small
  mono GM-mapped kit: soft / mid / hard layers only, 22.05 kHz, tails trimmed and
  faded, GM keys 35–59.
- Attribution requirement: credit **Lars Muldjord** (MuldjordKit) and the FreePats
  project when redistributing this asset or a derivative.

## Decoder

`rustysynth-ext` (MIT) with the `sf3` feature decodes Ogg Vorbis samples at load
time. The decoded PCM stays in memory; SF3 only reduces the on-disk/installer size.
