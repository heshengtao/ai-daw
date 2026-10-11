#!/usr/bin/env bash
# 把根 workspace 的 `aidaw` CLI 编译出来，按 Tauri sidecar 的命名规则放进 src-tauri/binaries/。
#
# 为什么需要这一步：Tauri 的 `bundle.externalBin` 要求文件名带**目标三元组**后缀
# （macOS `aidaw-aarch64-apple-darwin`、Windows `aidaw-x86_64-pc-windows-msvc.exe`…），
# 打包时它会把后缀去掉、把二进制放到主程序旁边（macOS 即 `AI DAW.app/Contents/MacOS/`）。
#
# 注意：`aidaw` 在**根** workspace，而 `src-tauri` 是独立的 workspace——
# 直接 `tauri build` 不会编译它，所以必须在仓库根显式 `cargo build -p aidaw`。
#
# ⚠️ 变量一律写 `${VAR}`，**不要**写成 `$VAR`：macOS 自带的是 bash 3.2，在 UTF-8 locale 下
# （如 `LANG=zh_CN.UTF-8`）它会把紧跟变量名的**多字节字符吃进变量名**——`"$TRIPLE）"` 会被
# 当成变量 `TRIPLE）`，于是 `set -u` 直接报 `TRIPLE?: unbound variable`。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# 目标三元组：优先 rustc 的 host-tuple（Rust 1.84+），旧版本回退到 -Vv 的输出。
if TRIPLE="$(rustc --print host-tuple 2>/dev/null)" && [ -n "$TRIPLE" ]; then
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

echo "==> 构建 aidaw（${TRIPLE}）"
(cd "$ROOT" && cargo build -p aidaw --release)

SRC="$ROOT/target/release/aidaw${EXE}"
DST="$ROOT/src-tauri/binaries/aidaw-${TRIPLE}${EXE}"
[ -f "$SRC" ] || {
  echo "找不到构建产物：${SRC}" >&2
  exit 1
}

echo "==> 放置 sidecar：${DST}"
mkdir -p "$(dirname "$DST")"
cp -f "$SRC" "$DST"
chmod +x "$DST"
echo "==> 完成"
