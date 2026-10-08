import { ref } from "vue";
import { createRouter, createWebHashHistory } from "vue-router";

/** 工作区视图路由（/arrange /edit /mix）、设置（/settings）与合成器独立窗口（/synth）。 */
const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: "/", redirect: "/arrange" },
    { path: "/arrange", name: "arrange", component: () => import("@/views/ArrangeView.vue") },
    { path: "/edit", name: "edit", component: () => import("@/views/EditView.vue") },
    { path: "/mix", name: "mix", component: () => import("@/views/MixView.vue") },
    { path: "/settings", name: "settings", component: () => import("@/views/SettingsView.vue") },
    // 内置合成器的独立窗口：由后端用 `index.html#/synth?track=N` 打开。
    { path: "/synth", name: "synth", component: () => import("@/views/SynthWindowView.vue") },
    // 宿主给「只能嵌入」的插件开窗时加载的空页：插件视图会盖在它上面。
    { path: "/blank", name: "blank", component: () => import("@/views/BlankView.vue") },
  ],
});

/** 最近一个非设置页的工作区路径，供「从设置返回」使用。 */
export const lastWorkspace = ref("/arrange");

router.afterEach((to) => {
  if (!to.path.startsWith("/settings")) lastWorkspace.value = to.path;
});

export default router;
