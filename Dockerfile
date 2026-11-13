# AI DAW 无头后端（ai-daw）
#
# 只跑后端 + HTTP `/rpc`：不开窗口、不要显示器、不链 WebView。
#
# 构建：
#   docker build -t ai-daw .
# 运行：
#   docker run --rm -p 28430:28430 -v ai-daw-data:/data ai-daw
# 首次启动会把访问 token 打印到日志（同时落在数据卷的 `web-token` 里）。
# 验证：
#   curl http://127.0.0.1:28430/health
#
# 体积：二进制约 20 MB，运行层只要 libasound2 + libssl3 + ca-certificates，
# 整镜像落在 120 MB 上下。**关键在于 `--no-default-features`**：它关掉 crate 的
# `desktop` 特性，`tauri` 不进依赖图，WebKit/GTK 那一坨（约 612 MB）就不会被链进来。

# syntax=docker/dockerfile:1

# ---- 构建 ----
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

RUN cargo build --release -p ai-daw-daemon --no-default-features --bin ai-daw \
    && strip /src/target/release/ai-daw

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

COPY --from=builder /src/target/release/ai-daw /usr/local/bin/ai-daw

# 数据目录（会话、预设、音色记忆库、token）——挂卷持久化。
VOLUME ["/data"]
ENV AIDAW_DATA_DIR=/data

EXPOSE 28430

# `--lan` 在容器里是**必须**的：容器内的 127.0.0.1 从外面到不了。
ENTRYPOINT ["ai-daw", "--data-dir", "/data", "--lan", "--port", "28430"]

HEALTHCHECK --interval=15s --timeout=3s --start-period=20s --retries=3 \
    CMD curl -fsS http://127.0.0.1:28430/health || exit 1
