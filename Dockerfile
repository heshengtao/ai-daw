# AI DAW 无头后端（aidawd）+ 浏览器界面
#
# 一条命令就能用：
#   docker run -p 28430:28430 -v ai-daw-data:/data ailm32442/ai-daw
# 然后浏览器打开 http://<host>:28430/。
#
# **镜像内容必须与桌面安装包对齐**：桌面随包发的东西（ONNX Runtime 基线、三个小模型、
# 奏法映射库、插件扫描探针 / 隔离 helper），镜像里也必须有——否则就是"桌面能用、
# Docker 里悄悄失效"。两边唯一的差别应该只是"有没有声卡 / 有没有窗口"。
#
# 体积：二进制约 20 MB + 前端 3.5 MB + 随包 AI（ORT 11 MB + 模型 6.6 MB）
# + 插件宿主（3.8 MB）+ Debian slim 底座。
# 关键在 `--no-default-features`：关掉 crate 的 `desktop` 特性后 `tauri` 不进依赖图，
# WebKit/GTK 那一坨（约 612 MB）不会被链进来。

# syntax=docker/dockerfile:1

# 随包的 ONNX Runtime 版本：与 `crates/local-ai/src/backend/runtime.rs` 的
# `ORT_VERSION` / `libonnxruntime.so.1.30.0` 必须一致（`installed()` 逐文件判定）。
ARG ORT_VERSION=1.30.0
# 目标架构：Docker 自动注入。arm64 必须取 `onnxruntime-linux-aarch64-*`，
# 否则镜像里装的是 x86_64 的 .so，`ort` 加载时会直接失败（`ldd` 报 not a dynamic executable）。
ARG TARGETARCH

# ---- 前端 ----
FROM node:22-slim AS web
RUN corepack enable
WORKDIR /web
# 先只拷清单：依赖不变时这一层能命中缓存。
COPY package.json pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile
COPY index.html vite.config.ts tsconfig.json tsconfig.node.json postcss.config.js tailwind.config.js env.d.ts ./
COPY src ./src
COPY public ./public
RUN pnpm build

# ---- 后端 ----
FROM rust:1-bookworm AS builder
# 全局 ARG 只在 FROM 里可见；RUN 里要用必须在本阶段重新声明。
ARG ORT_VERSION
ARG TARGETARCH

# pkg-config（找 libssl / alsa）、openssl 头、alsa 头；curl 用来取随包的 ONNX Runtime。
RUN apt-get update && apt-get install -y --no-install-recommends \
        pkg-config \
        libssl-dev \
        libasound2-dev \
        curl \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src

# `include_str!` 把出厂预设、MCP 技能文档、插件注册表编进二进制，所以这些目录
# 必须在编译时就位；其余（node_modules / target / docs）由 `.dockerignore` 挡掉。
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY skills ./skills
COPY src/data ./src/data
COPY src-tauri/resources ./src-tauri/resources

# 主程序 + 插件宿主 sidecar。桌面包里这四个二进制都在主程序旁边
# （`bundle.externalBin`），镜像里也必须同目录：`vst3-host` 先找 exe 同级再找 PATH。
RUN cargo build --release -p ai-daw-daemon --no-default-features --bin aidawd \
    && cargo build --release -p ai-daw-vst3-helper -p ai-daw-host-vst3 --bins \
    && strip /src/target/release/aidawd \
             /src/target/release/vst3-host-helper \
             /src/target/release/vst3-host-probe \
             /src/target/release/clap-scan-probe

# 随包的 ONNX Runtime 基线。桌面用 `npm run fetch:ort`，镜像用同一条官方发布包、
# 同一个版本、同一个文件名（`installed()` 按名字判定）。
# 架构必须跟目标一致：官方包名里 amd64 → `x64`、arm64 → `aarch64`。
#
# `TARGETARCH` **不能当唯一来源**：它是 BuildKit 的自动参数，而本机只能用经典构建器
# （BuildKit 拉 `auth.docker.io` 会超时）。经典构建器不会设置它 → 静默回落到 x64，
# 镜像里就躺着一份架构不对的 .so，运行期只报 "dlopen failed"。
# 这里以构建阶段的真实架构为准，再用 ELF 头自检（62 = x86_64，183 = aarch64）。
RUN set -eux; \
    case "${TARGETARCH:-$(dpkg --print-architecture)}" in \
        arm64|aarch64) ORT_ARCH=aarch64; WANT_EM=183 ;; \
        *)             ORT_ARCH=x64;     WANT_EM=62  ;; \
    esac; \
    curl -fsSL --retry 3 \
        "https://github.com/microsoft/onnxruntime/releases/download/v${ORT_VERSION}/onnxruntime-linux-${ORT_ARCH}-${ORT_VERSION}.tgz" \
      | tar xz -C /opt; \
    mv "/opt/onnxruntime-linux-${ORT_ARCH}-${ORT_VERSION}" /opt/ort-bundle; \
    test -f "/opt/ort-bundle/lib/libonnxruntime.so.${ORT_VERSION}"; \
    GOT_EM=$(od -An -tu2 -j18 -N2 "/opt/ort-bundle/lib/libonnxruntime.so.${ORT_VERSION}" | tr -d ' \n'); \
    echo "ORT arch check: e_machine=$GOT_EM (want $WANT_EM, TARGETARCH=${TARGETARCH:-unset})"; \
    [ "$GOT_EM" = "$WANT_EM" ] || { echo "ONNX Runtime 架构不匹配：下载到 e_machine=$GOT_EM，期望 $WANT_EM"; exit 1; }

# ---- 运行 ----
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        libasound2 \
        libssl3 \
        # ONNX Runtime 官方 Linux 构建依赖 libstdc++ / libgomp。
        libstdc++6 \
        libgomp1 \
        # 插件宿主（VST3/CLAP）的 GUI 走 X11，哪怕无头也要求 libxcb 存在。
        libxcb1 \
        libxcb-shm0 \
        libx11-6 \
    && rm -rf /var/lib/apt/lists/* \
    && find /usr/share/doc /usr/share/man /usr/share/locale -mindepth 1 -maxdepth 1 -exec rm -rf {} + 2>/dev/null || true

COPY --from=builder /src/target/release/aidawd /usr/local/bin/aidawd
# 插件宿主：必须与 aidawd 同目录（`host-vst3/src/scan.rs` 先看主程序旁边）。
COPY --from=builder /src/target/release/vst3-host-helper /usr/local/bin/vst3-host-helper
COPY --from=builder /src/target/release/vst3-host-probe /usr/local/bin/vst3-host-probe
COPY --from=builder /src/target/release/clap-scan-probe /usr/local/bin/clap-scan-probe

# 随包 AI 资源：与桌面 `bundle.resources` 同一份清单。
# 注意**不要**放进 /data：`VOLUME /data` 会在运行时遮住镜像内文件。
COPY --from=builder /src/src-tauri/resources/onnx/ /opt/models/
COPY --from=builder /src/src-tauri/resources/articulation-maps /opt/articulation-maps
COPY --from=builder /opt/ort-bundle/lib/libonnxruntime.so.1.30.0 /opt/ort/
# 兼容裸名：Linux 官方包只有带版本号的那份，某些加载路径仍按 `libonnxruntime.so` 找。
RUN ln -sf /opt/ort/libonnxruntime.so.1.30.0 /opt/ort/libonnxruntime.so

# 前端产物：浏览器打开根路径就是它。
COPY --from=web /web/dist /web

# 数据目录（会话、预设、音色记忆库）——挂卷持久化。
VOLUME ["/data"]
ENV AIDAW_DATA_DIR=/data
ENV AIDAW_WEB_DIR=/web
# 随包资源的位置。`AIDAW_MODELS_EXTRA` 排在下载落点之后：用户装进 /data/models 的
# 同名文件会覆盖镜像自带的那一份。
ENV AIDAW_MODELS_EXTRA=/opt/models
ENV AIDAW_ORT_BUNDLED=/opt/ort
ENV AIDAW_ARTICULATION_MAPS=/opt/articulation-maps

EXPOSE 28430

# `--lan` 在容器里是**必须**的：容器内的 127.0.0.1 从外面到不了。
ENTRYPOINT ["aidawd", "--data-dir", "/data", "--lan", "--port", "28430"]

HEALTHCHECK --interval=15s --timeout=3s --start-period=20s --retries=3 \
    CMD curl -fsS http://127.0.0.1:28430/health || exit 1
