import { createApp } from "vue";
import { createPinia } from "pinia";
import App from "./App.vue";
import router from "./router";
import { appReady } from "./api/tauri";
import { useTheme } from "./composables/useTheme";
import "./assets/main.css";
import "./assets/fonts.css";

// 主题必须在挂载前应用：否则「上次选了深色」这件事在重新打开应用时会失效，
// 一直要等到用户进一次设置页才生效。
useTheme();

createApp(App).use(createPinia()).use(router).mount("#app");

// 首帧画完再告诉后端「可以显示窗口 / 装载音频宿主了」。
// 用定时器而不是 `requestAnimationFrame`：窗口在 `app_ready` 之前还是隐藏的，
// 隐藏的页面不保证跑 rAF（会被浏览器节流），那样会谁也等不到谁。
// 250 ms 足够让挂载后的第一帧真的提交到合成器。
setTimeout(() => {
  void appReady().catch(() => {
    // 老版本后端没有这个命令 / 预览环境：忽略即可（窗口由后端兜底定时器显示）。
  });
}, 250);
