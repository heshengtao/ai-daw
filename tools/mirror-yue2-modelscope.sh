#!/usr/bin/env bash
#
# 把 `tools/fetch-yue2-models.sh` 下好的 YuE2 权重推到**你自己的魔搭仓库**。
#
#   bash tools/fetch-yue2-models.sh                 # 先下载（含 LICENSE / README）
#   MS_REPO=你的账号/YuE2-GGUF bash tools/mirror-yue2-modelscope.sh
#
# 只做上传：不重新下载、不改本地文件；缺文件时会提示你先跑 fetch 脚本。
# 需要 `export MODELSCOPE_API_TOKEN=<token>`（https://modelscope.cn/my/myaccesstoken）。
#
# 上传后 sha256 不变，所以清单里补一条 modelscope 源就行：
#   ModelSource { kind: "modelscope", region: "cn",
#                 url: "https://modelscope.cn/models/<MS_REPO>/resolve/master/<文件名>" }
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST="${DEST:-$ROOT/.tmp/yue2-mirror}"
case "$DEST" in
  /*) ;;
  [A-Za-z]:[\\/]*) ;;
  *) DEST="$ROOT/$DEST" ;;
esac

if [[ -z "${MS_REPO:-}" ]]; then
  echo "缺 MS_REPO。用法：MS_REPO=你的账号/YuE2-GGUF bash $0" >&2
  exit 2
fi
if [[ ! -d "$DEST" ]]; then
  echo "找不到 $DEST —— 先跑：bash tools/fetch-yue2-models.sh" >&2
  exit 2
fi

missing=()
for need in YuE2-3B-Q5_K_M.gguf YuE2-Vae-F32.gguf configuration.json LICENSE; do
  [[ -f "$DEST/$need" ]] || missing+=("$need")
done
if ((${#missing[@]})); then
  echo "目录里缺文件：${missing[*]} —— 先跑：bash tools/fetch-yue2-models.sh" >&2
  exit 2
fi

echo "==> 待上传（${DEST#$ROOT/}）："
ls -1sh "$DEST" | sed 's/^/    /'
echo

python - "$MS_REPO" "$DEST" <<'PY'
import os, sys
from modelscope.hub.api import HubApi

model_id, local_dir = sys.argv[1], sys.argv[2]
token = (os.environ.get("MODELSCOPE_API_TOKEN") or "").strip()
if not token:
    sys.exit(
        "没有 MODELSCOPE_API_TOKEN。\n"
        "  到 https://modelscope.cn/my/myaccesstoken 复制 token，然后：\n"
        "    export MODELSCOPE_API_TOKEN=<token>"
    )

api = HubApi()          # 构造函数不带凭据，必须显式 login(token)
api.login(token)

# 许可证是 CC BY-NC 4.0；create_model 的枚举里没有它，失败就跳过（网页上建好再推送即可）
for license_id in ("CC-BY-NC-4.0", "cc-by-nc-4.0"):
    try:
        api.create_model(model_id, visibility=5, license=license_id,
                         original_model_id="m-a-p/YuE2-3B")
        print(f"==> 已在魔搭创建 {model_id}（license={license_id}）")
        break
    except Exception as exc:
        print(f"==> create_model({license_id}) 没成功：{exc}")
else:
    print("==> 跳过创建（仓库已存在就行）")

api.push_model(model_id, local_dir, license="CC-BY-NC-4.0",
               commit_message="mirror: YuE2-3B GGUF (backbone + VAE + transcriber + tokenizer)")
print(f"==> 已推送：https://modelscope.cn/models/{model_id}")
PY
