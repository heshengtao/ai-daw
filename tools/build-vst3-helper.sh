#!/usr/bin/env bash
# 编译 vst3-host 的隔离 helper，按 Tauri sidecar 的命名规则放进 src-tauri/binaries/。
#
# 为什么需要这一步：
# - `vst3-host` 的进程隔离要一个独立的 `vst3-host-helper` 进程来加载插件；
# - Cargo 不会构建依赖 crate 的 bin target，所以本仓库把它抄成 workspace 成员
#   `crates/vst3-helper`（逐字一致，见该目录与 NOTICE）；
# - `bundle.externalBin` 要求文件名带目标三元组后缀，打包时 Tauri 会把后缀去掉、
#   把二进制放到主程序旁边（macOS 即 `AI DAW.app/Contents/MacOS/`），
#   `vst3-host` 查找 helper 时正好优先看主程序旁边。
#
# 交叉编译时设 `AIDAW_TARGET=<triple>`（与 build-aidaw.sh 一致）。
#
# ⚠️ 变量一律写 `${VAR}`，**不要**写成 `$VAR`：macOS 自带 bash 3.2 在 UTF-8 locale 下
# 会把紧跟变量名的多字节字符吃进变量名。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

TARGET_SUBDIR=""
CARGO_TARGET_ARG=""
if [ -n "${AIDAW_TARGET:-}" ]; then
  TRIPLE="${AIDAW_TARGET}"
  TARGET_SUBDIR="${AIDAW_TARGET}"
  CARGO_TARGET_ARG="--target ${AIDAW_TARGET}"
elif TRIPLE="$(rustc --print host-tuple 2>/dev/null)" && [ -n "$TRIPLE" ]; then
  :
else
  TRIPLE="$(rustc -Vv | sed -n 's/^host: //p')"
fi
[ -n "$TRIPLE" ] || {
  echo "无法确定目标三元组：请确认 rustc 在 PATH 上" >&2
  exit 1
}

EXE=""
case "$TRIPLE" in
  *windows*) EXE=".exe" ;;
esac

echo "==> 构建 vst3-host helper（${TRIPLE}）"
(cd "$ROOT" && cargo build -p ai-daw-vst3-helper --release ${CARGO_TARGET_ARG})

if [ -n "${TARGET_SUBDIR}" ]; then
  SRC="$ROOT/target/${TARGET_SUBDIR}/release/vst3-host-helper${EXE}"
else
  SRC="$ROOT/target/release/vst3-host-helper${EXE}"
fi
DST="$ROOT/src-tauri/binaries/vst3-host-helper-${TRIPLE}${EXE}"
[ -f "$SRC" ] || {
  echo "找不到构建产物：${SRC}" >&2
  exit 1
}

echo "==> 放置 sidecar：${DST}"
mkdir -p "$(dirname "$DST")"
cp -f "$SRC" "$DST"
chmod +x "$DST"
echo "==> 完成"
