# AI DAW 无头后端（aidawd）+ 浏览器界面
#
# 一条命令就能用：
#   docker run -p 28430:28430 -v ai-daw-data:/data ailm32442/ai-daw
# 然后浏览器打开 http://<host>:28430/#token=<日志里的 token>。
#
# 镜像里**自带前端产物**（构建阶段用 node 打包，运行阶段只有静态文件 + 20 MB 二进制），
# 所以不需要 compose、不需要挂载 dist。
#
# 体积：二进制约 20 MB + 前端 3.5 MB + Debian slim 底座与四个运行库。
# 关键在 `--no-default-features`：关掉 crate 的 `desktop` 特性后 `tauri` 不进依赖图，
# WebKit/GTK 那一坨（约 612 MB）不会被链进来。

# syntax=docker/dockerfile:1

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

# 无头构建只需要三样：pkg-config（找 libssl / alsa）、openssl 头、alsa 头。
RUN apt-get update && apt-get install -y --no-install-recommends \
        pkg-config \
        libssl-dev \
        libasound2-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src

# `include_str!` 把出厂预设、MCP 技能文档、插件注册表编进二进制，所以这些目录
# 必须在编译时就位；其余（node_modules / target / docs）由 `.dockerignore` 挡掉。
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY skills ./skills
COPY src/data ./src/data
COPY src-tauri/resources ./src-tauri/resources

RUN cargo build --release -p ai-daw-daemon --no-default-features --bin aidawd \
    && strip /src/target/release/aidawd

# ---- 运行 ----
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        libasound2 \
        libssl3 \
        # 插件宿主（VST3/CLAP）的 GUI 走 X11，哪怕无头也要求 libxcb 存在。
        libxcb1 \
        libx11-6 \
    && rm -rf /var/lib/apt/lists/* \
    && find /usr/share/doc /usr/share/man /usr/share/locale -mindepth 1 -maxdepth 1 -exec rm -rf {} + 2>/dev/null || true

COPY --from=builder /src/target/release/aidawd /usr/local/bin/aidawd
# 前端产物：浏览器打开根路径就是它。
COPY --from=web /web/dist /web

# 数据目录（会话、预设、音色记忆库、token）——挂卷持久化。
VOLUME ["/data"]
ENV AIDAW_DATA_DIR=/data
ENV AIDAW_WEB_DIR=/web

EXPOSE 28430

# `--lan` 在容器里是**必须**的：容器内的 127.0.0.1 从外面到不了。
ENTRYPOINT ["aidawd", "--data-dir", "/data", "--lan", "--port", "28430"]

HEALTHCHECK --interval=15s --timeout=3s --start-period=20s --retries=3 \
    CMD curl -fsS http://127.0.0.1:28430/health || exit 1
