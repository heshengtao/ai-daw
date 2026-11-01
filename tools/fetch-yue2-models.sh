#!/usr/bin/env bash
#
# 把 YuE2 需要的权重复制到本地 `.tmp/yue2-mirror/`（**不进 git**），供你之后手动传到魔搭。
#
#   bash tools/fetch-yue2-models.sh                # 推荐集合（全功能最小体积，约 4.0 GiB）
#   bash tools/fetch-yue2-models.sh --check        # 只核对上游大小 / sha256，不下载
#   bash tools/fetch-yue2-models.sh --all-quants   # 再加 Q6_K / Q8_0 / BF16（约 +14 GiB）
#   bash tools/fetch-yue2-models.sh --f32-transcriber   # 转谱器用 F32（+1.65 GiB）
#   bash tools/fetch-yue2-models.sh --only SheetSage2   # 只下名字里带这个串的（调试用）
#
# 每个文件都用 Hugging Face 的 LFS oid（就是 sha256）逐个校验——传上去以后字节不会变，
# 清单里可以直接钉同一个 sha256。同时会把上游的 LICENSE 与 README 一起拷进来。
#
# 需要：pip install -U huggingface_hub（可选 hf_transfer 加速）、能连 HF（不通时自动借
# 本机 10809 / 7890 / 7897 的代理口，也可 HF_PROXY=http://127.0.0.1:10809 指定）。
# 有 HF token 就 export HF_TOKEN=...（只是提高限速，不是必须）。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST="${DEST:-$ROOT/.tmp/yue2-mirror}"
HF_REV="${HF_REV:-e630f2b8f6aedf8254c3186438633c1b5eeaf80a}"        # 清单钉的 backbone 提交
LORA_REPO="Mothersuperior/yue2-mothersuperior-realaudio-tokenizer-v4"
LORA_REV="${LORA_REV:-e2e63d859f3af879baf1b4d4e9f22d1eeda6fde5}"
BACKBONE_REPO="Serveurperso/YuE2-GGUF"

ALL_QUANTS=0
F32_TRANSCRIBER=0
CHECK_ONLY=0
ONLY_PATTERNS=()

while (($#)); do
  case "$1" in
    --check)            CHECK_ONLY=1 ;;
    --all-quants)       ALL_QUANTS=1 ;;
    --f32-transcriber)  F32_TRANSCRIBER=1 ;;
    --only)             shift; ONLY_PATTERNS+=("${1:?--only 后面要跟一个文件名片段}") ;;
    --dir)              shift; DEST="${1:?--dir 后面要跟目录}" ;;
    -h|--help)          sed -n '2,20p' "$0"; exit 0 ;;
    *) echo "未知参数：$1（--check / --all-quants / --f32-transcriber / --only <片段> / --dir <目录>）" >&2; exit 2 ;;
  esac
  shift
done

# 相对路径一律按仓库根算，后面才好判「在不在 .tmp 下」
case "$DEST" in
  /*) ;;
  [A-Za-z]:[\\/]*) ;;                 # Git Bash 下传进来的 Windows 绝对路径
  *) DEST="$ROOT/$DEST" ;;
esac

# ---- 代理：HF 直连不通时借用本机常见的口 --------------------------------------
if [[ -z "${HF_PROXY:-}" ]]; then
  for port in 10809 7890 7897 7891 1080; do
    if curl -s -o /dev/null --max-time 1 "http://127.0.0.1:$port" 2>/dev/null; then
      HF_PROXY="http://127.0.0.1:$port"; break
    fi
  done
fi
if [[ -n "${HF_PROXY:-}" ]]; then
  export HTTPS_PROXY="$HF_PROXY" HTTP_PROXY="$HF_PROXY" https_proxy="$HF_PROXY" http_proxy="$HF_PROXY"
  echo "==> HF 走代理：$HF_PROXY"
else
  echo "==> 未检测到代理，按直连下载（连不上会自动重试失败）"
fi

# ---- 要下哪些文件 ------------------------------------------------------------
# 每条：仓库|文件名|用途（写进 MANIFEST）
SELECTED=()
add() { SELECTED+=("$1|$2|$3"); }

add "$BACKBONE_REPO" "YuE2-3B-Q5_K_M.gguf" "backbone Q5_K_M（推荐档：歌声合成主体）"
add "$BACKBONE_REPO" "YuE2-Vae-F32.gguf"   "VAE F32（48 kHz 立体声解码，永远 F32）"
if [[ "$F32_TRANSCRIBER" == 1 ]]; then
  add "$BACKBONE_REPO" "MERT-v2-FullSong-F32.gguf" "转谱器底座 MERT-v2 F32"
  add "$BACKBONE_REPO" "SheetSage2-F32.gguf"       "转谱头 SheetSage2 F32（音频→ABC 谱）"
else
  add "$BACKBONE_REPO" "MERT-v2-FullSong-Q8_0.gguf" "转谱器底座 MERT-v2 Q8_0"
  add "$BACKBONE_REPO" "SheetSage2-Q8_0.gguf"       "转谱头 SheetSage2 Q8_0（音频→ABC 谱）"
fi
add "$BACKBONE_REPO" "yue2-mothersuperior-realaudio-tokenizer-v4-Q8_0.gguf" "音频 tokenizer Q8_0（音频→语义码，可选能力）"
add "$LORA_REPO"     "nar_lora_joint_v9.safetensors" "realaudio NAR 适配器 v9（音色解码，可选）"
if [[ "$ALL_QUANTS" == 1 ]]; then
  add "$BACKBONE_REPO" "YuE2-3B-Q6_K.gguf" "backbone Q6_K"
  add "$BACKBONE_REPO" "YuE2-3B-Q8_0.gguf" "backbone Q8_0"
  add "$BACKBONE_REPO" "YuE2-3B-BF16.gguf" "backbone BF16（最高质量）"
fi

if ((${#ONLY_PATTERNS[@]})); then
  filtered=()
  for row in "${SELECTED[@]}"; do
    for pat in "${ONLY_PATTERNS[@]}"; do
      [[ "$row" == *"$pat"* ]] && { filtered+=("$row"); break; }
    done
  done
  SELECTED=("${filtered[@]}")
  ((${#SELECTED[@]})) || { echo "==> --only 没匹配到任何文件" >&2; exit 1; }
fi

# ---- 上游元数据（大小 + LFS oid = sha256）-------------------------------------
say() { printf '%s\n' "$*"; }
tmp_meta="$(mktemp -d)"
trap 'rm -rf "$tmp_meta"' EXIT

tree_of() {  # repo rev → 本地 json 路径
  local repo="$1" rev="$2" out="$tmp_meta/$(echo "$repo" | tr '/' '_').json"
  if [[ ! -s "$out" ]]; then
    curl -fsSL --max-time 90 \
      "https://huggingface.co/api/models/$repo/tree/$rev?recursive=false" -o "$out" \
      || { echo "取 $repo 的文件列表失败（网络/代理）" >&2; exit 1; }
  fi
  echo "$out"
}

say ""
say "==> 核对上游（大小 + LFS sha256）"
declare -A META   # "repo|file" → "size|sha256"
total=0
for row in "${SELECTED[@]}"; do
  IFS='|' read -r repo name role <<<"$row"
  rev="$HF_REV"; [[ "$repo" == "$LORA_REPO" ]] && rev="$LORA_REV"
  json="$(tree_of "$repo" "$rev")"
  meta="$(python - "$json" "$name" <<'PY'
import json, sys
path, want = sys.argv[1], sys.argv[2]
for item in json.load(open(path, encoding="utf-8")):
    if item.get("path") == want:
        lfs = item.get("lfs") or {}
        size = lfs.get("size") or item.get("size") or 0
        oid = lfs.get("oid") or item.get("oid") or ""
        print(f"{size}|{oid}")
        break
else:
    print("MISSING|")
PY
)"
  if [[ "$meta" == MISSING* ]]; then
    echo "  ✗ $repo/$name 在上游找不到（提交 $rev）" >&2; exit 1
  fi
  META["$repo|$name"]="$meta"
  bytes="${meta%%|*}"
  total=$((total + bytes))
  printf '  ✓ %-58s %12s 字节\n' "$name" "$bytes"
done
say ""
printf '==> 合计 %d 个文件，%.2f GiB\n' "${#SELECTED[@]}" "$(awk -v t="$total" 'BEGIN{print t/1073741824}')"
[[ "$CHECK_ONLY" == 1 ]] && { say "==> --check 结束（未下载）"; exit 0; }

# ---- 落盘位置：必须在 .tmp/ 下且被 git 忽略 ------------------------------------
case "$DEST" in
  "$ROOT"/.tmp/*) ;;
  *) echo "警告：$DEST 不在 $ROOT/.tmp/ 下，请自己确认不会进 git" >&2 ;;
esac
mkdir -p "$DEST"
if git -C "$ROOT" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  if git -C "$ROOT" check-ignore -q "$DEST" 2>/dev/null; then
    say "==> git 已忽略：${DEST#$ROOT/}"
  else
    echo "警告：$DEST 没有被 .gitignore 忽略，提交前别 git add -A" >&2
  fi
fi

# hf_transfer 装了就用（多线程分块，国内代理下快很多）
if python -c "import hf_transfer" >/dev/null 2>&1; then
  export HF_HUB_ENABLE_HF_TRANSFER=1
  say "==> 已启用 hf_transfer（多线程下载）"
fi

# ---- 下载（每个文件一条进度条 + 汇总进度）------------------------------------
started=$(date +%s)
done_bytes=0
index=0
for row in "${SELECTED[@]}"; do
  IFS='|' read -r repo name role <<<"$row"
  index=$((index + 1))
  rev="$HF_REV"; [[ "$repo" == "$LORA_REPO" ]] && rev="$LORA_REV"
  IFS='|' read -r bytes sha <<<"${META["$repo|$name"]}"

  if [[ -f "$DEST/$name" ]]; then
    have="$(sha256sum "$DEST/$name" | cut -d' ' -f1)"
    if [[ "$have" == "$sha" ]]; then
      say "==> [$index/${#SELECTED[@]}] 已就位（sha256 一致）：$name"
      done_bytes=$((done_bytes + bytes))
      continue
    fi
    say "==> [$index/${#SELECTED[@]}] $name 校验不符，重下"
    rm -f "$DEST/$name"
  fi

  say ""
  say "==> [$index/${#SELECTED[@]}] $repo/$name  ($(awk -v b="$bytes" 'BEGIN{printf "%.2f GiB", b/1073741824}'))  —— $role"
  hf download "$repo" "$name" --revision "$rev" --local-dir "$DEST"

  have="$(sha256sum "$DEST/$name" | cut -d' ' -f1)"
  if [[ "$have" != "$sha" ]]; then
    echo "  ✗ sha256 不符：期望 $sha，实际 $have" >&2
    exit 1
  fi
  done_bytes=$((done_bytes + bytes))
  pct="$(awk -v d="$done_bytes" -v t="$total" 'BEGIN{printf "%.1f", (t ? d*100/t : 100)}')"
  say "  ✓ sha256 一致。总进度 ${pct}%  （$(awk -v d="$done_bytes" 'BEGIN{printf "%.2f", d/1073741824}') / $(awk -v t="$total" 'BEGIN{printf "%.2f GiB", t/1073741824}')）"
done

# ---- 许可证 / README / 魔搭元数据 --------------------------------------------
say ""
say "==> 收尾：许可证、README、魔搭用的 configuration.json"
hf download "$BACKBONE_REPO" LICENSE README.md --revision "$HF_REV" --local-dir "$DEST" >/dev/null
mv -f "$DEST/README.md" "$DEST/README.upstream.md"
hf download "$LORA_REPO" README.md --revision "$LORA_REV" --local-dir "$DEST" >/dev/null
mv -f "$DEST/README.md" "$DEST/README.lora-upstream.md"
echo '{"framework": "pytorch", "task": "text-to-audio", "allow_remote": true}' > "$DEST/configuration.json"

# 整理落盘清单（上传后对症核对用）
{
  echo "# YuE2 权重镜像清单"
  echo "# 生成于 $(date '+%Y-%m-%d %H:%M:%S')"
  echo "# backbone/转谱/VAE/tokenizer：$BACKBONE_REPO@$HF_REV"
  echo "# NAR 适配器：$LORA_REPO@$LORA_REV"
  echo "# 许可：CC BY-NC 4.0（上游 LICENSE 见本目录 LICENSE）"
  echo "# 文件\t字节\tsha256\t用途"
  for row in "${SELECTED[@]}"; do
    IFS='|' read -r repo name role <<<"$row"
    IFS='|' read -r bytes sha <<<"${META["$repo|$name"]}"
    printf '%s\t%s\t%s\t%s\n' "$name" "$bytes" "$sha" "$role"
  done
} > "$DEST/MANIFEST.tsv"

say ""
say "==> 完成。目录：${DEST#$ROOT/}"
ls -lh "$DEST" | sed 's/^/    /'
say ""
say "下一步（手动传到你的魔搭仓库）："
say "    pip install -U modelscope"
say "    export MODELSCOPE_API_TOKEN=<https://modelscope.cn/my/myaccesstoken>"
say "    python - <<'PY'"
say "    import os; from modelscope.hub.api import HubApi"
say "    api = HubApi(); api.login(os.environ['MODELSCOPE_API_TOKEN'])"
say "    api.create_model('你的账号/YuE2-GGUF', visibility=5, license='CC-BY-NC-4.0')"
say "    api.push_model('你的账号/YuE2-GGUF', '$DEST', license='CC-BY-NC-4.0')"
say "    PY"
say ""
say "或者直接跑：MS_REPO=你的账号/YuE2-GGUF bash tools/mirror-yue2-modelscope.sh（会复用这里的文件）"
