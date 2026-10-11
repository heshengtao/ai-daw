#!/usr/bin/env bash
#
# 下载本地 AI 的 ONNX 权重到 `models/`（不进仓库）。
#
#   ./tools/fetch-models.sh                  # 直连
#   AIDAW_MODEL_PROXY=http://127.0.0.1:7892 ./tools/fetch-models.sh   # 走代理
#
# 三个模型：
#   models/basic-pitch-nmp.onnx  Basic Pitch（Spotify, Apache-2.0）—— 官方 ONNX 导出的镜像
#   models/htdemucs.ort          HTDemucs v4（音源分离, Apache-2.0）—— 209 MB，不随包，正式版由
#                                应用内「管理器 → 模型」下载；这里只是开发路径
#   models/localvqe-ns.onnx      LocalVQE v1.2（Apache-2.0）—— 上游只发 .pt/GGUF，
#                                ONNX 是本地用 tools/export-localvqe-onnx.py 从官方权重导出的
#
# LocalVQE 那一步需要 python3 + torch；缺依赖时会跳过并打印手动步骤，其余模型仍然可用。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST="$ROOT/models"
mkdir -p "$DEST"

CURL=(curl -fL --retry 3 --retry-delay 2 --connect-timeout 20)
if [[ -n "${AIDAW_MODEL_PROXY:-}" ]]; then
  CURL+=(-x "$AIDAW_MODEL_PROXY")
  echo "使用代理：$AIDAW_MODEL_PROXY"
fi

echo "==> Basic Pitch (nmp.onnx)"
"${CURL[@]}" -o "$DEST/basic-pitch-nmp.onnx" \
  "https://huggingface.co/AEmotionStudio/basic-pitch-onnx-models/resolve/main/nmp.onnx"
ls -l "$DEST/basic-pitch-nmp.onnx"

echo "==> HTDemucs v4（音源分离，209 MB）"
HTDEMUCS_SHA="09dc165512d8ef7480bcb2cacea9dda82d571f8dbf421d8c44a2ca5568bec729"
"${CURL[@]}" -o "$DEST/htdemucs.ort" \
  "https://huggingface.co/gentij/htdemucs-ort/resolve/main/htdemucs.ort"
# sha256 硬门禁：字节对不上就报错（与运行时下载器的立场一致）。
sha256_of() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    sha256sum "$1" | awk '{print $1}'
  fi
}
got="$(sha256_of "$DEST/htdemucs.ort")"
[[ "$got" == "$HTDEMUCS_SHA" ]] || {
  echo "    sha256 校验失败：期望 $HTDEMUCS_SHA，实际 $got" >&2
  exit 1
}
ls -l "$DEST/htdemucs.ort"

echo "==> LocalVQE v1.2（.pt → ONNX 导出）"
python3 -c "import torch" 2>/dev/null || {
  echo "    跳过：需要 python3 + torch 才能从官方 .pt 导出 ONNX。"
  echo "    份量最大的那一步可以手动做："
  echo "      1) pip install torch onnx onnxruntime pyyaml"
  echo "      2) curl -L -o /tmp/lvqe-v1.2-1.3M.pt https://huggingface.co/LocalAI-io/LocalVQE/resolve/main/localvqe-v1.2-1.3M.pt"
  echo "      3) git clone --depth 1 https://github.com/localai-org/LocalVQE /tmp/LocalVQE"
  echo "      4) python3 tools/export-localvqe-onnx.py \\"
  echo "             --checkpoint /tmp/lvqe-v1.2-1.3M.pt \\"
  echo "             --pytorch /tmp/LocalVQE/pytorch \\"
  echo "             --out $DEST/localvqe-ns.onnx"
  exit 0
}

CACHE="${TMPDIR:-/tmp}/ai-daw-localvqe"
mkdir -p "$CACHE"
CKPT="$CACHE/localvqe-v1.2-1.3M.pt"
if [[ ! -f "$CKPT" ]]; then
  "${CURL[@]}" -o "$CKPT" \
    "https://huggingface.co/LocalAI-io/LocalVQE/resolve/main/localvqe-v1.2-1.3M.pt"
fi
if [[ ! -d "$CACHE/LocalVQE-main/pytorch" ]]; then
  "${CURL[@]}" -o "$CACHE/main.tgz" \
    "https://codeload.github.com/localai-org/LocalVQE/tar.gz/refs/heads/main"
  tar xzf "$CACHE/main.tgz" -C "$CACHE"
fi

python3 "$ROOT/tools/export-localvqe-onnx.py" \
  --checkpoint "$CKPT" \
  --pytorch "$CACHE/LocalVQE-main/pytorch" \
  --out "$DEST/localvqe-ns.onnx"

echo
echo "完成，模型目录："
ls -l "$DEST"
