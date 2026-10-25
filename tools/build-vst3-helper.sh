#!/usr/bin/env bash
# 编译插件宿主用的隔离 helper 与扫描探针，按 Tauri sidecar 的命名规则放进 src-tauri/binaries/。
#
# 为什么需要这一步：
# - `vst3-host` 的进程隔离要一个独立的 `vst3-host-helper` 进程来加载插件；
# - 崩溃隔离的 VST3 扫描要一个 `vst3-host-probe` 进程，每个插件单独探测一次；
# - 崩溃隔离的 CLAP 扫描要一个 `clap-scan-probe` 进程（`crates/host-vst3` 的 bin target）；
# - Cargo 不会构建依赖 crate 的 bin target，所以本仓库把 VST3 的两个抄成 workspace 成员
#   `crates/vst3-helper`（逐字一致，见该目录与 NOTICE）；
# - `bundle.externalBin` 要求文件名带目标三元组后缀，打包时 Tauri 会把后缀去掉、
#   把二进制放到主程序旁边（macOS 即 `AI DAW.app/Contents/MacOS/`），
#   `vst3-host` 查找 helper / 探针时正好优先看主程序旁边。
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

echo "==> 构建 vst3-host helper / 探针（${TRIPLE}）"
(cd "$ROOT" && cargo build -p ai-daw-vst3-helper --release ${CARGO_TARGET_ARG})

echo "==> 构建 CLAP 扫描探针（${TRIPLE}）"
(cd "$ROOT" && cargo build -p ai-daw-host-vst3 --bin clap-scan-probe --release ${CARGO_TARGET_ARG})

if [ -n "${TARGET_SUBDIR}" ]; then
  OUT_DIR="$ROOT/target/${TARGET_SUBDIR}/release"
else
  OUT_DIR="$ROOT/target/release"
fi

for NAME in vst3-host-helper vst3-host-probe clap-scan-probe; do
  SRC="${OUT_DIR}/${NAME}${EXE}"
  DST="$ROOT/src-tauri/binaries/${NAME}-${TRIPLE}${EXE}"
  [ -f "${SRC}" ] || {
    echo "找不到构建产物：${SRC}" >&2
    exit 1
  }

  echo "==> 放置 sidecar：${DST}"
  mkdir -p "$(dirname "${DST}")"
  cp -f "${SRC}" "${DST}"
  chmod +x "${DST}"
done

echo "==> 完成"
