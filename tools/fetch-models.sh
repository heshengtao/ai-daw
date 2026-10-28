#!/usr/bin/env bash
#
# 下载本地 AI 的 ONNX 权重到 `models/`（不进仓库）。
#
#   ./tools/fetch-models.sh                  # 直连
#   AIDAW_MODEL_PROXY=http://127.0.0.1:7892 ./tools/fetch-models.sh   # 走代理
#
# 四个模型：
#   models/basic-pitch-nmp.onnx  Basic Pitch（Spotify, Apache-2.0）—— 官方 ONNX 导出的镜像
#   models/htdemucs.ort          HTDemucs v4（音源分离, Apache-2.0）—— 209 MB，不随包，正式版由
#                                应用内「管理器 → 模型」下载；这里只是开发路径
#   models/localvqe-ns.onnx      LocalVQE v1.2（Apache-2.0）—— 上游只发 .pt/GGUF，
#                                ONNX 是本地用 tools/export-localvqe-onnx.py 从官方权重导出的
#   models/mel_spectrogram.onnx  Beat This! 的 log-mel 前端（MIT）—— 264 KB，**随包**（与
#                                随包资源是同一个文件）
#   models/beat_this.onnx        Beat This! 主模型 `final0`（MIT）—— 约 83 MB，不随包，正式版由
#                                应用内「管理器 → 模型」下载（id `beat-this`）；这里只是开发路径
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

sha256_of() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    sha256sum "$1" | awk '{print $1}'
  fi
}

echo "==> Basic Pitch (nmp.onnx)"
"${CURL[@]}" -o "$DEST/basic-pitch-nmp.onnx" \
  "https://huggingface.co/AEmotionStudio/basic-pitch-onnx-models/resolve/main/nmp.onnx"
ls -l "$DEST/basic-pitch-nmp.onnx"

echo "==> HTDemucs v4（音源分离，209 MB）"
HTDEMUCS_SHA="09dc165512d8ef7480bcb2cacea9dda82d571f8dbf421d8c44a2ca5568bec729"
"${CURL[@]}" -o "$DEST/htdemucs.ort" \
  "https://huggingface.co/gentij/htdemucs-ort/resolve/main/htdemucs.ort"
# sha256 硬门禁：字节对不上就报错（与运行时下载器的立场一致）。
got="$(sha256_of "$DEST/htdemucs.ort")"
[[ "$got" == "$HTDEMUCS_SHA" ]] || {
  echo "    sha256 校验失败：期望 $HTDEMUCS_SHA，实际 $got" >&2
  exit 1
}
ls -l "$DEST/htdemucs.ort"

echo "==> Beat This!（节拍检测：mel 前端 + final0 主模型）"
# mel 前端（264 KB）：与随包资源 `src-tauri/resources/onnx/mel_spectrogram.onnx` 是同一个文件。
"${CURL[@]}" -o "$DEST/mel_spectrogram.onnx" \
  "https://raw.githubusercontent.com/danigb/beat-this-rs/main/models/mel_spectrogram.onnx"
# final0（FP32，约 83 MB）：上游放在 GitHub Releases 的 `model-large` 资产里（crate 本身不含权重）。
BEAT_THIS_SHA="5f810debe53459b559127fb55bbad40035bb47cc567b20e501670f968c770f02"
"${CURL[@]}" -o "$DEST/beat_this.onnx" \
  "https://github.com/danigb/beat-this-rs/releases/download/model-large/beat_this.onnx"
got="$(sha256_of "$DEST/beat_this.onnx")"
[[ "$got" == "$BEAT_THIS_SHA" ]] || {
  echo "    sha256 校验失败：期望 $BEAT_THIS_SHA，实际 $got" >&2
  exit 1
}
ls -l "$DEST/beat_this.onnx" "$DEST/mel_spectrogram.onnx"

echo "==> SoulX-Singer（歌声合成，9 张图 + 5 个外部权重，约 1.4 GB）"
# 不随包，正式版由应用内「管理器 → 模型」下载（id `soulx-singer-fp16`）；这里只是开发路径。
# 源：魔搭（国内直连）。国际镜像待补，见 tools/mirror-soulx-hf.md。
SVS_DIR="$DEST/soulx-singer-fp16"
mkdir -p "$SVS_DIR"
SVS_FILES=(
  "note_text_encoder.onnx 331415477a985a8962601a465c616806b5d32a33f5f93230e941a54656d352c6"
  "note_text_encoder.onnx.data 998889d7daff9cd50774abf23282d1acad01d2c753db3040b156d1a53b178e79"
  "note_pitch_encoder.onnx 741b08a2767e9134cd40d4e5b4e073590c2f74cd6dca0c107c7a7715965a346a"
  "note_pitch_encoder.onnx.data 20ddcaaea04315e5a46eb11d8cacb6dd6ecf6a9688ebc3af019ab7b27ee7b9c6"
  "note_type_encoder.onnx 0aba568ee59c350175663bdc04a40be44fe36fc46ceb648f462d65638bbd85b4"
  "note_type_encoder.onnx.data becf918cd5e23b25783ae42480c13bfa04bf38b84a4075d7c4273befcb3af4a0"
  "f0_encoder.onnx c7628a2af8b0347a28edbbb2d35bd8a12eb13aa1b138ae4ed267d0e08bb3a046"
  "f0_encoder.onnx.data 8c3f3c514b1bf78f723b8fa785ed2f7dd5cb994ca2c1349f22c54ea4909b0f4b"
  "preflow.onnx d06b7274491a2346194654e17297e63823a10ccc2e21dd39540f4057ffe62abe"
  "preflow.onnx.data ce2c424a3a22a22a1aff172767baacc3e6796df0eb83f979596d522d87d2422f"
  "cond_emb.onnx 3437e166b1dfd5094215565566fd146e71fa29ac2cf50653831a5e6da60deb88"
  "cond_emb.onnx.data fbfa95df1617d8dc12c1544c788be65d5bb08b365a68041c69953b02ac066dbe"
  "mel_transform.onnx 823a41e7b50506c28afc0d954823f27717fe47b50304012eec36daaa7d22685c"
  "mel_transform.onnx.data 2d1e18edf37041270f999b15bbfd08c19cccca723a6111425ae9a8a228c87927"
  "diff_step_dml.onnx 1a2857f819562e75b73b291ea86ec3c1157d77f6e51abad1e934d2f318711629"
  "vocoder_dml.onnx 185a707f907554196f28848256ecfd1b72031c5335571e2dedcbb0a0de35db95"
)
for entry in "${SVS_FILES[@]}"; do
  name="${entry%% *}"
  want="${entry##* }"
  target="$SVS_DIR/$name"
  if [[ -f "$target" && "$(sha256_of "$target")" == "$want" ]]; then
    echo "    跳过（已就绪）：$name"
    continue
  fi
  echo "    $name"
  "${CURL[@]}" -o "$target.part" \
    "https://modelscope.cn/models/syxppp/SoulX-Singer-onnx-directml-fp16/resolve/master/$name"
  got="$(sha256_of "$target.part")"
  [[ "$got" == "$want" ]] || {
    echo "    sha256 校验失败（$name）：期望 $want，实际 $got" >&2
    rm -f "$target.part"
    exit 1
  }
  mv "$target.part" "$target"
done
ls -l "$SVS_DIR"

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
