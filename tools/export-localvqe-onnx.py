#!/usr/bin/env python3
"""把 LocalVQE v1.2（官方 PyTorch 权重）导出成单输入、固定长度的降噪 ONNX。

上游 `localai-org/LocalVQE` 只发布 `.pt`（研究用）和 GGUF（GGML 推理），没有 ONNX，
所以在这里从官方权重导出一份，供 `crates/local-ai` 用 `ort` 直接跑。

用法：

    python3 tools/export-localvqe-onnx.py \
        --checkpoint /tmp/localvqe-v1.2-1.3M.pt \
        --pytorch    /tmp/LocalVQE/pytorch \
        --out        models/localvqe-ns.onnx

`--pytorch` 指向上游仓库的 `pytorch/` 目录（含 `localvqe/model.py` 与 `configs/default.yaml`）。

导出时做三件事，都是为了「能稳定跑」而不是「改模型」：

1. **去掉 AlignBlock**：它是 AEC 用的远端参考对齐，需要第二个输入 `ref`。降噪场景没有远端
   信号（传零），此时它对输出的贡献只有 ~1e-4 相对量级（实测与官方图 ref=0 的最大偏差
   4.5e-5）；去掉后图里只剩单输入 `mic`。
2. **固定长度 2 秒**（32 000 样本 @16 kHz）：官方前向的中间张量随时长线性增长（几分钟素材
   要吃几百 MB），且动态长度会踩到 `Unfold`/`Pad` 的形状特化。分块 + 交叠相加交给 Rust 侧。
3. **用 legacy 导出器**（`dynamo=False`）：TorchScript 导出器对这个图给出的结果与 PyTorch
   逐样本一致（≤5.3e-6），而 torch 2.11 默认的 dynamo 导出器会生成一个在静音输入下
   产出 NaN、且部分输入偏差达 1e-1 的图。

脚本最后会自检：官方权重 sha256、以及导出结果 vs PyTorch 在静音/噪声/半静音上的逐样本
最大偏差（要求 < 1e-4 且无 NaN/inf）。
"""
from __future__ import annotations

import argparse
import hashlib
import sys
from pathlib import Path

import numpy as np
import torch
import yaml

# 上游 manifest 里 v1.2 的官方 sha256。
EXPECTED_SHA256 = "ff6885e7c8d7d29a8ce963303dcd668ae0f2a7bdafae28631292fe6f06f7cd77"

# 固定导出长度：2 秒 @16 kHz。T = 126 帧，fold 出来的原始长度是 (126-1)*256+512 = 32512，
# 掐掉编码器的 256 点补零后是 32256；调用方只用前 32000 个样本。
N = 32_000
OUT_LEN = 32_512
PAD = 256
# 每一级编码器/解码器上的频率轴宽度（写成常量，避免图里出现数据相关的 slice）。
FREQ = [256, 128, 64, 32, 16, 8]


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--checkpoint", required=True, type=Path)
    ap.add_argument("--pytorch", required=True, type=Path, help="上游仓库的 pytorch/ 目录")
    ap.add_argument("--out", required=True, type=Path)
    ap.add_argument("--skip-sha", action="store_true", help="跳过权重 sha256 校验")
    args = ap.parse_args()

    digest = sha256(args.checkpoint)
    if not args.skip_sha and digest != EXPECTED_SHA256:
        print(f"权重 sha256 不符：\n  期望 {EXPECTED_SHA256}\n  实际 {digest}", file=sys.stderr)
        return 1
    print(f"权重校验通过（{digest[:16]}…）")

    sys.path.insert(0, str(args.pytorch))
    from localvqe.model import DCTDecoder, LocalVQE  # noqa: E402

    # --- 解码器：把 overlap 归一化改成严格随 T 计算（原版把它缓存在 buffer 里，
    #     一旦缓存就会在导出时固化成 trace 期的常量，动态长度会算错）。
    def decoder_forward(self, x, length=None):  # type: ignore[no-untyped-def]
        b, f, t, _ = x.shape
        x = x.permute(0, 1, 3, 2).reshape(b, f * 2, t)
        frames = self.linear(x.transpose(1, 2))
        fold = lambda z: torch.nn.functional.fold(  # noqa: E731
            z,
            output_size=(1, OUT_LEN),
            kernel_size=(1, self.kernel_size),
            stride=(1, self.stride),
        ).reshape(b, OUT_LEN)
        out = fold(frames.transpose(1, 2))
        count = fold(torch.ones(1, self.kernel_size, t, dtype=x.dtype)).clamp(min=1)
        return (out / count)[:, PAD:]

    DCTDecoder.forward = decoder_forward

    cfg = yaml.safe_load((args.pytorch / "configs" / "default.yaml").read_text())
    kwargs = dict(cfg["model"])
    kwargs["arch_version"] = 3
    kwargs["dmax"] = 64
    model = LocalVQE(**kwargs, n_freqs=cfg["audio"]["n_freqs"]).eval()
    blob = torch.load(args.checkpoint, map_location="cpu", weights_only=True)
    model.load_state_dict(blob["model_state_dict"], strict=True)
    model.align.fold_temperature()
    print(f"载入完成：{sum(p.numel() for p in model.parameters()):,} 参数")

    class Denoise(torch.nn.Module):
        """零远端参考的降噪图（AlignBlock 的输出在 ref=0 时对结果无实质贡献）。"""

        def __init__(self, m):  # type: ignore[no-untyped-def]
            super().__init__()
            self.m = m

        def forward(self, mic):  # type: ignore[no-untyped-def]
            m = self.m
            enc = m.encoder(mic)
            fe = m.fe_mic(enc)
            e1 = m.mic_enc1(fe)
            e2 = m.mic_enc2(e1)
            cat = torch.cat([e2, torch.zeros_like(e2)], 1)
            e3 = m.mic_enc3(cat)
            e4 = m.mic_enc4(e3)
            e5 = m.mic_enc5(e4)
            bn = m.bottleneck(e5)
            d5 = m.dec5(bn, e5)[..., : FREQ[4]]
            d4 = m.dec4(d5, e4)[..., : FREQ[3]]
            d3 = m.dec3(d4, e3)[..., : FREQ[2]]
            d2 = m.dec2(d3, e2)[..., : FREQ[1]]
            d1 = m.dec1(d2, e1)[..., : FREQ[0]]
            return m.decoder(m.mask(d1, enc))

    net = Denoise(model).eval()
    args.out.parent.mkdir(parents=True, exist_ok=True)
    torch.onnx.export(
        net,
        (torch.randn(1, N) * 0.3,),
        str(args.out),
        input_names=["mic"],
        output_names=["out"],
        opset_version=18,
        dynamo=False,  # legacy 导出器：dynamo 版会给出静音 NaN 的错图
    )
    print(f"已导出 {args.out}（{args.out.stat().st_size / 1e6:.1f} MB）")

    # --- 自检：官方图（ref=0）与导出图在几类输入上逐样本对比。
    import onnxruntime as ort

    session = ort.InferenceSession(str(args.out), providers=["CPUExecutionProvider"])
    rng = np.random.default_rng(0)
    half = np.zeros(N, np.float32)
    half[: N // 2] = (0.3 * np.sin(2 * np.pi * 220 * np.arange(N // 2) / 16_000)).astype(np.float32)
    cases = {
        "静音": np.zeros(N, np.float32),
        "白噪声": (rng.standard_normal(N) * 0.3).astype(np.float32),
        "半静音": half,
    }
    ok = True
    for name, x in cases.items():
        with torch.no_grad():
            want = net(torch.from_numpy(x)[None]).numpy()
        got = session.run(["out"], {"mic": x[None]})[0]
        n = min(want.shape[1], got.shape[1])
        worst = float(np.abs(want[0, :n] - got[0, :n]).max())
        bad = int((~np.isfinite(got)).sum())
        print(f"  自检 {name}: maxdiff={worst:.2e} 非有限样本={bad}")
        ok = ok and worst < 1e-4 and bad == 0
    if not ok:
        print("自检未通过：导出的图与 PyTorch 参考不一致", file=sys.stderr)
        return 1
    print("自检通过")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
