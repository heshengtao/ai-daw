#!/usr/bin/env bash
#
# 把 SoulX-Singer 的 fp16 ONNX 整组镜像到 Hugging Face（国际源）。
#
#   HF_REPO=ailm32442/SoulX-Singer-onnx-fp16 ./tools/mirror-soulx-hf.sh
#
# 为什么需要这一步：模型清单里**只允许官方源**（HF / GitHub / 魔搭）。同一份字节要在
# 国外也有一个直连点，就得有一个 HF 仓库。脚本做三件事：
#   1. 本地没有权重就先从魔搭下（复用 tools/fetch-models.sh 里同样的源）；
#   2. 逐个比对 sha256（与 `crates/local-ai/src/models/catalog.rs` 里的硬门禁同一份）；
#   3. 用 `hf` CLI 上传（跳过已存在且大小一致的文件，可重复跑）。
#
# 需要：`pip install -U huggingface_hub` 并 `hf auth login`（或设 HF_TOKEN）。
# 上传完成后，在每个文件的 `sources` 最前面插一条：
#     ModelSource { kind: "hf", region: "intl",
#                   url: "https://huggingface.co/<HF_REPO>/resolve/main/<filename>" }
# 下载器会按用户网络位置自动优先选源；两个源的字节必须完全一致，否则 sha256 校验不过。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST="${SVS_DIR:-$ROOT/models/soulx-singer-fp16}"
HF_REPO="${HF_REPO:-ailm32442/SoulX-Singer-onnx-fp16}"
BASE="https://modelscope.cn/models/syxppp/SoulX-Singer-onnx-directml-fp16/resolve/master"

FILES=(
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

sha256_of() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    sha256sum "$1" | awk '{print $1}'
  fi
}

command -v hf >/dev/null 2>&1 || command -v huggingface-cli >/dev/null 2>&1 || {
  echo "缺 huggingface_hub CLI：pip install -U huggingface_hub && hf auth login" >&2
  exit 1
}
UPLOAD=(hf upload)
command -v hf >/dev/null 2>&1 || UPLOAD=(huggingface-cli upload)

mkdir -p "$DEST"
for entry in "${FILES[@]}"; do
  name="${entry%% *}"
  want="${entry##* }"
  target="$DEST/$name"
  if [[ ! -f "$target" || "$(sha256_of "$target")" != "$want" ]]; then
    echo "==> 下载 $name"
    curl -fL --retry 3 --retry-delay 2 -o "$target.part" "$BASE/$name"
    got="$(sha256_of "$target.part")"
    [[ "$got" == "$want" ]] || {
      echo "sha256 校验失败（$name）：期望 $want，实际 $got" >&2
      rm -f "$target.part"
      exit 1
    }
    mv "$target.part" "$target"
  fi
  echo "==> 上传 $name → $HF_REPO"
  "${UPLOAD[@]}" "$HF_REPO" "$target" "$name" --repo-type model
done

echo
echo "完成。清单（crates/local-ai/src/models/catalog.rs 的 soulx-singer-fp16）已经带好 HF 源，"
echo "本脚本只负责把同一份字节推上去；传完跑一次 cargo test -p ai-daw-local-ai --lib 复核。"
