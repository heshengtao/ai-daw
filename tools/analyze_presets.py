#!/usr/bin/env python3
"""预设客观测量：起振 / 衰减 / 频谱重心 / 高次谐波占比 / 混叠（非谐波）占比。

用法: python3 tools/analyze_presets.py <audio_dir> [--filter 8bit]
所有预设都在同一音高（C4 = 261.63 Hz）渲染，所以横向可比。
"""
import json
import sys
import wave
from pathlib import Path

import numpy as np

SR = 48000
F0 = 261.6255653005986  # C4


def read_wav(path: Path) -> np.ndarray:
    with wave.open(str(path), "rb") as w:
        assert w.getsampwidth() == 2
        raw = w.readframes(w.getnframes())
    return np.frombuffer(raw, dtype="<i2").astype(np.float64) / 32768.0


def envelope(x: np.ndarray, win: int = 512) -> np.ndarray:
    n = len(x) // win
    return np.abs(x[: n * win].reshape(n, win)).max(axis=1)


def harmonics_split(x: np.ndarray, f0: float, start: int):
    """把能量分成「落在谐波上」与「不在谐波上」两部分。

    非谐波占比是**混叠/噪声**的代理指标：干净的脉冲波能量只落在 n·f0 上。

    窗口有两点讲究：
    - 从**包络峰值之后 12 ms** 开始：起振那一下本身是宽带的，压在窗口上会把
      「起振瞬态」误读成混叠（实测能把一个干净的脉冲读成 65%）。
    - 长度 8192（约 170 ms）：短促音也能盖住主要发声段，同时频率分辨率仍有 5.9 Hz，
      足够分辨间隔 261 Hz 的谐波。
    """
    n = 1 << 13
    at = max(0, min(start + int(0.012 * SR), len(x) - 1))
    seg = x[at : at + n]
    if len(seg) < n:
        seg = np.pad(seg, (0, n - len(seg)))
    spec = np.abs(np.fft.rfft(seg * np.hanning(n))) ** 2
    freqs = np.fft.rfftfreq(n, 1 / SR)

    # **丢掉 0.5×f0 以下的能量**：占空比不为 50% 的脉冲天生带直流（12.5% 的均值是 -0.75），
    # 包络的慢速起伏也堆在这一带。它们是真实存在的，但不是「混叠」，算进来会把一个干净的
    # 芯片音读成 70% 非谐波。
    audible = freqs >= 0.5 * f0
    spec = spec * audible
    total = spec.sum()
    if total <= 1e-18:
        return 0.0, 0.0, 0.0, 0.0
    harmonic_mask = np.zeros(n // 2 + 1, dtype=bool)
    k = 1
    while k * f0 < SR / 2:
        centre = k * f0
        width = max(centre * 0.02, 12.0)
        harmonic_mask |= np.abs(freqs - centre) <= width
        k += 1
    harmonic = spec[harmonic_mask].sum()
    centroid = float((freqs * spec).sum() / total)
    # 6 kHz 以上的能量占比：刺耳感的常用代理。
    air = spec[freqs >= 6000].sum() / total
    alias = max(0.0, (total - harmonic) / total)
    # 发声期间的 RMS：跨预设比响度要用这个，不能用整段（短促音会被整段稀释）。
    tone = x[at : at + n]
    tone_rms = float(np.sqrt((tone**2).mean())) if len(tone) else 0.0
    return centroid, air, alias, tone_rms


def measure(x: np.ndarray, f0: float) -> dict:
    env = envelope(x)
    peak = float(np.abs(x).max())
    if peak <= 1e-6:
        return {"peak_db": -120, "attack_ms": 0, "centroid": 0, "air": 0, "alias": 0, "tone_db": -120}
    # 起振：从开始到达峰值 90% 的时间
    above = np.where(env >= 0.9 * env.max())[0]
    attack_ms = float(above[0] * 512 / SR * 1000) if len(above) else 0.0
    # 频谱取在包络峰值之后：打击类也能量到音色本身。
    at_peak = int(np.argmax(env)) * 512
    centroid, air, alias, tone_rms = harmonics_split(x, f0, at_peak)
    return {
        "peak_db": 20 * np.log10(peak),
        "attack_ms": attack_ms,
        "centroid": centroid,
        "air": air,
        "alias": alias,
        "tone_db": 20 * np.log10(max(tone_rms, 1e-9)),
    }


def main() -> None:
    audio = Path(sys.argv[1])
    only = None
    if "--filter" in sys.argv:
        only = sys.argv[sys.argv.index("--filter") + 1]
    meta_path = audio.parent / "presets.json"
    meta = {p["id"]: p for p in json.loads(meta_path.read_text())} if meta_path.exists() else {}

    rows = []
    for path in sorted(audio.glob("*.wav")):
        stem = path.stem
        pid, _, note = stem.partition("@")
        note = int(note) if note else 60
        category = meta.get(pid, {}).get("category", "?")
        if only and only not in category:
            continue
        m = measure(read_wav(path), 440.0 * 2 ** ((note - 69) / 12))
        rows.append((pid, note, category, m))

    print(
        f"{'预设':<32}{'音':>4} {'类别':<14}{'峰值dB':>7}{'起振ms':>7}{'重心Hz':>9}"
        f"{'>6k%':>7}{'混叠%':>7}{'发声dB':>8}"
    )
    for pid, note, category, m in rows:
        print(
            f"{pid:<32}{note:>4} {category:<14}{m['peak_db']:>7.1f}{m['attack_ms']:>7.0f}"
            f"{m['centroid']:>9.0f}{m['air'] * 100:>7.1f}{m['alias'] * 100:>7.1f}{m['tone_db']:>8.1f}"
        )


if __name__ == "__main__":
    main()
