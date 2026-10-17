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
# 交叉编译时（例如在 arm64 macOS 上出 x86_64 的包）设 `AIDAW_TARGET=<triple>`：此时按该
# 三元组命名 sidecar 并用 `cargo build --target` 编译。不设的话会拿**运行机**的 host-tuple
# 命名，与 Tauri 要找的文件名对不上。CI 由 workflow 传入，本地构建无需设置。
#
# ⚠️ 变量一律写 `${VAR}`，**不要**写成 `$VAR`：macOS 自带的是 bash 3.2，在 UTF-8 locale 下
# （如 `LANG=zh_CN.UTF-8`）它会把紧跟变量名的**多字节字符吃进变量名**——`"$TRIPLE）"` 会被
# 当成变量 `TRIPLE）`，于是 `set -u` 直接报 `TRIPLE?: unbound variable`。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# 目标三元组：`AIDAW_TARGET` 优先（交叉编译），否则用 rustc 的 host-tuple（Rust 1.84+），
# 旧版本再回退到 -Vv 的输出。
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

echo "==> 构建 aidaw（${TRIPLE}）"
(cd "$ROOT" && cargo build -p aidaw --release ${CARGO_TARGET_ARG})

if [ -n "${TARGET_SUBDIR}" ]; then
  SRC="$ROOT/target/${TARGET_SUBDIR}/release/aidaw${EXE}"
else
  SRC="$ROOT/target/release/aidaw${EXE}"
fi
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
