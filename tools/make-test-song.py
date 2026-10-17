#!/usr/bin/env python3
"""生成一首「简单曲子」用来测试节拍检测（Beat This!）。

**为什么要专门造一首**：真实音乐里全是干扰项——弱起的前奏、渐入的鼓、现场演奏的速度漂移、
自由节拍的段落。第一次验证一条检测链路时，这些都会让「到底是模型不准还是集成错了」变得
说不清。这个脚本合成的东西是干净的：

- **死准的 120 BPM、4/4**（每拍 0.5 s，每小节 2 s），全程没有速度漂移；
- **底鼓（1、3 拍）+ 军鼓（2、4 拍）+ 八分闭镲 + 每小节一个和弦 + 小节头吊镲**；
- 长度**正好 `--bars` 个小节**（默认 12 小节 = 24.000 s，末尾只有 20 ms 淡出防爆音）——
  导入后片段的拍长因此是**整数**（48 拍 = 12 小节），不会多出半截空小节；
- 第 0 拍就是重拍（没有前奏），所以「首重拍对齐小节线」应当把片段放在**第 1 小节**。

> 为什么每小节要换一个和弦：Beat This! 的**拍点**非常稳，但**重拍**要靠音乐里的小节级线索
> （和声变化、吊镲、贝斯根音）。只放「四踩底鼓 + 2/4 军鼓」时它会把每两拍当成一小节，
> 拍号就会被推成 2/4；加上每小节一个和弦后才是清楚的 4/4。这也是这首曲子「简单但清楚」的关键。

**期望结果**：进「AI 面板 → 节拍检测」后应得到 `速度 120.0 BPM · 4/4`，且波形、网格、节拍器
三者对齐；片段起点约为 0（首重拍落在第 1 小节线上）。

用法：
    python3 tools/make-test-song.py                       # → test-files/beat-test-120bpm.wav
    python3 tools/make-test-song.py --bpm 90 --bars 16    # 换个速度 / 长度
    python3 tools/make-test-song.py /tmp/out.wav

只依赖标准库（`wave` / `array` / `math` / `random`），不需要 numpy。
"""

import argparse
import array
import math
import os
import random
import wave

SR = 44_100


def _mix(buf, start_sec, sr, dur_sec, gen):
    """把 `gen(i, t)` 生成的包络叠进 `buf`（`i` = 样点序号，`t` = 相对秒数）。"""
    start = int(start_sec * sr)
    n = int(dur_sec * sr)
    for i in range(n):
        idx = start + i
        if idx >= len(buf):
            break
        buf[idx] += gen(i, i / sr)


def _kick(buf, t, sr, gain=1.0):
    # 频率从 ~135 Hz 迅速滑到 ~50 Hz 的正弦，指数衰减。
    def gen(i, _t):
        env = math.exp(-i / (0.09 * sr))
        f = 50.0 + 85.0 * math.exp(-i / (0.018 * sr))
        return gain * env * math.sin(2 * math.pi * f * i / sr)

    _mix(buf, t, sr, 0.25, gen)


def _snare(buf, t, sr, rng, gain=0.5):
    # 190 Hz 音体 + 白噪声，短促衰减。
    def gen(i, _t):
        env = math.exp(-i / (0.055 * sr))
        tone = 0.35 * math.sin(2 * math.pi * 190 * i / sr)
        return gain * env * (tone + 0.65 * rng.uniform(-1.0, 1.0))

    _mix(buf, t, sr, 0.2, gen)


def _hat(buf, t, sr, rng, gain=0.18):
    # 一阶差分的噪声 ≈ 高通，得到「嚓」声。
    prev = 0.0

    def gen(i, _t):
        nonlocal prev
        env = math.exp(-i / (0.012 * sr))
        n = rng.uniform(-1.0, 1.0)
        hp = n - prev
        prev = n
        return gain * env * hp

    _mix(buf, t, sr, 0.05, gen)


def _bass(buf, t, freq, sr, dur=0.45, gain=0.45):
    def gen(i, _t):
        env = min(1.0, i / (0.005 * sr)) * math.exp(-i / (0.35 * sr))
        return gain * env * math.sin(2 * math.pi * freq * i / sr)

    _mix(buf, t, sr, dur, gen)


def _crash(buf, t, sr, rng, gain=0.2):
    # 吊镲：宽带噪声 + 长衰减，用来「标出小节头」。
    def gen(i, _t):
        env = math.exp(-i / (0.35 * sr))
        n = rng.uniform(-1.0, 1.0)
        return gain * env * (n + 0.5 * math.sin(2 * math.pi * 6200 * i / sr))

    _mix(buf, t, sr, 0.9, gen)


def _chord(buf, t, freqs, sr, dur, gain=0.12):
    # 持续和弦（每小节一个）：软起音 + 缓慢衰减，给小节一个和声锚点。
    def gen(i, _t):
        env = min(1.0, i / (0.02 * sr)) * math.exp(-i / (dur * 0.9 * sr))
        s = sum(math.sin(2 * math.pi * f * i / sr) for f in freqs) / len(freqs)
        return gain * env * s

    _mix(buf, t, sr, dur, gen)


# 一个四小节的简单走向（A 小调）：(贝斯根音, 和弦)。每小节换一个和弦 = 强 4/4 线索。
PROGRESSION = [
    (55.00, [220.00, 261.63, 329.63]),  # Am
    (43.65, [174.61, 220.00, 261.63]),  # F
    (65.41, [130.81, 164.81, 196.00]),  # C
    (49.00, [196.00, 246.94, 293.66]),  # G
]


def synthesize(bpm, bars, sr=SR):
    """返回单声道浮点采样（-1..1 附近，未归一化）。长度正好 `bars` 个小节。"""
    beat = 60.0 / bpm
    bar = 4 * beat
    total = int(round(bars * bar * sr))
    buf = [0.0] * total
    rng = random.Random(20261015)

    for b in range(bars):
        bar_t = b * bar
        root, chord = PROGRESSION[b % len(PROGRESSION)]
        _crash(buf, bar_t, sr, rng)  # 小节头吊镲
        _bass(buf, bar_t, root, sr, dur=1.8 * beat)  # 贝斯根音撑满整小节（不做两拍一换）
        _chord(buf, bar_t, chord, sr, dur=2.0 * beat)  # 每小节一个和弦
        for k in range(4):
            t = bar_t + k * beat
            if k % 2 == 0:
                _kick(buf, t, sr)  # 1、3 拍底鼓
            else:
                _snare(buf, t, sr, rng)  # 2、4 拍军鼓
            _hat(buf, t, sr, rng)  # 八分闭镲
            _hat(buf, t + beat / 2.0, sr, rng)

    # 末尾 20 ms 淡出：文件正好结束在小节线上，直接切会「啪」一声。
    fade = min(total, int(0.02 * sr))
    for i in range(fade):
        buf[total - fade + i] *= (fade - i) / fade

    return buf


def write_wav(path, mono, sr=SR):
    """把单声道浮点写成 16-bit 立体声 PCM WAV（左右相同）。"""
    peak = max((abs(v) for v in mono), default=1.0) or 1.0
    scale = 0.89 / peak  # 留 ~1 dB 余量
    samples = array.array("h")
    for v in mono:
        s = int(max(-1.0, min(1.0, v * scale)) * 32767)
        samples.append(s)  # 左
        samples.append(s)  # 右
    os.makedirs(os.path.dirname(os.path.abspath(path)), exist_ok=True)
    with wave.open(path, "wb") as w:
        w.setnchannels(2)
        w.setsampwidth(2)
        w.setframerate(sr)
        w.writeframes(samples.tobytes())


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    default_out = os.path.join(here, "..", "test-files", "beat-test-120bpm.wav")
    ap = argparse.ArgumentParser(description="合成一首用于测试节拍检测的简单曲子。")
    ap.add_argument("out", nargs="?", default=default_out, help="输出 WAV 路径")
    ap.add_argument("--bpm", type=float, default=120.0, help="速度（默认 120）")
    ap.add_argument("--bars", type=int, default=12, help="小节数（默认 12 ≈ 24 秒）")
    args = ap.parse_args()

    mono = synthesize(args.bpm, args.bars)
    write_wav(args.out, mono)
    seconds = len(mono) / SR
    print(f"已写入 {os.path.abspath(args.out)}")
    print(f"  {args.bpm:g} BPM · 4/4 · {args.bars} 小节 · {seconds:.1f} 秒 · 44.1 kHz 立体声")


if __name__ == "__main__":
    main()
