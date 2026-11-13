# AI DAW 无头后端（aidawd）
#
# 只跑后端 + HTTP `/rpc`：不开窗口、不要显示器、不需要 WebView 运行期。
#
# 构建：
#   docker build -t aidawd .
# 运行：
#   docker run --rm -p 28430:28430 -v aidaw-data:/data aidawd
# 首次启动会把访问 token 打印到日志（同时落在数据卷的 `web-token` 里）。
# 验证：
#   curl http://127.0.0.1:28430/health
#
# 说明：M1 过渡期 `ai-daw-daemon` 仍依赖 `tauri`（只为 `#[tauri::command]` 宏），
# 所以构建层需要 webkit2gtk 头文件、运行层需要它的运行库。等命令层与 Tauri 彻底
# 解耦后，这两层都能瘦一大圈。

# syntax=docker/dockerfile:1

# ---- 构建 ----
FROM rust:1-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
        pkg-config \
        libssl-dev \
        libasound2-dev \
        libwebkit2gtk-4.1-dev \
        libgtk-3-dev \
        libsoup-3.0-dev \
        libjavascriptcoregtk-4.1-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src

# `include_str!` 把出厂预设、MCP 技能文档、插件注册表编进二进制，所以这些目录
# 必须在编译时就位；其余（node_modules / target / docs）由 `.dockerignore` 挡掉。
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY skills ./skills
COPY src/data ./src/data
COPY src-tauri/resources ./src-tauri/resources

RUN cargo build --release -p ai-daw-daemon --bin aidawd

# ---- 运行 ----
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        libasound2 \
        libssl3 \
        libwebkit2gtk-4.1-0 \
        libgtk-3-0 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /src/target/release/aidawd /usr/local/bin/aidawd

# 数据目录（会话、预设、音色记忆库、token）——挂卷持久化。
VOLUME ["/data"]
ENV AIDAW_DATA_DIR=/data

EXPOSE 28430

# `--lan` 在容器里是**必须**的：容器内的 127.0.0.1 从外面到不了。
ENTRYPOINT ["aidawd", "--data-dir", "/data", "--lan", "--port", "28430"]

HEALTHCHECK --interval=15s --timeout=3s --start-period=25s --retries=3 \
    CMD curl -fsS http://127.0.0.1:28430/health || exit 1
