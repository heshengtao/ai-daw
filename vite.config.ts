import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";
import { fileURLToPath, URL } from "node:url";

const host = process.env.TAURI_DEV_HOST;

/**
 * 固定端口。**不要改回 1420**：Windows 上要同时躲开两张表，否则 dev server 起不来
 * （Node 报 `EACCES: permission denied ::1:1420`，`beforeDevCommand` 非零退出）：
 *
 * 1. **排除端口区**（`netsh interface ipv4 show excludedportrange protocol=tcp`）：
 *    Hyper-V / WSL2 / Docker 启动时整段预留，绑定直接 WSAEACCES(10013)。本机覆盖 1112–2061。
 * 2. **动态端口区**（`netsh int ipv4 show dynamicport tcp`）：系统从这个区间发临时端口，
 *    固定服务ports 落进去会偶尔撞车。本机是 1024–15000（默认才是 49152–65535，被改过）。
 *
 * 结论：**20000–49151** 这一段才稳（10xxx–19xxx 也可能被上面的表吃掉，5xxxx 反而常在
 * 排除区或动态区里）。改这里必须同步改 `src-tauri/tauri.conf.json` 的 `build.devUrl`。
 * 兄弟项目 ai-cut 用 28430，两个可以同时起。
 */
const DEV_PORT = 28420;
const HMR_PORT = 28421;

// Vite config tailored for Tauri development:
// 1. don't obscure rust errors, 2. fixed port, 3. don't watch src-tauri.
export default defineConfig({
  plugins: [vue()],
  resolve: {
    alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) },
  },
  clearScreen: false,
  server: {
    port: DEV_PORT,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: HMR_PORT } : undefined,
    // 只盯前端真正会改的东西：`src-tauri/`（Rust，另有 cargo watch）之外，
    // 还要排除 Rust 的 `target/`、构建产物 `dist/` 与临时目录 `.tmp/`。
    // 后三个是本机最容易膨胀的目录（实测 target 2.6 万文件、.tmp 4.8 万文件）：
    // dev 服务器启动时 chokidar 要递归扫一遍，几十万个 fs 事件把 Node 的事件循环
    // 卡住十几秒——表现就是「窗口出来了，页面一片黑，几十秒后才亮」。
    watch: {
      ignored: ["**/src-tauri/**", "**/target/**", "**/dist/**", "**/.tmp/**"],
    },
  },
  envPrefix: ["VITE_", "TAURI_"],
});
