import { createApp } from "vue";
import { createPinia } from "pinia";
import App from "./App.vue";
import router from "./router";
import { useTheme } from "./composables/useTheme";
import "./assets/main.css";
import "./assets/fonts.css";

// 主题必须在挂载前应用：否则「上次选了深色」这件事在重新打开应用时会失效，
// 一直要等到用户进一次设置页才生效。
useTheme();

createApp(App).use(createPinia()).use(router).mount("#app");
