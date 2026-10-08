import { createRouter, createWebHashHistory } from "vue-router";

/** 工作区视图路由（/arrange /edit /mix /browse）。 */
const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: "/", redirect: "/arrange" },
    { path: "/arrange", name: "arrange", component: () => import("@/views/ArrangeView.vue") },
    { path: "/edit", name: "edit", component: () => import("@/views/EditView.vue") },
    { path: "/mix", name: "mix", component: () => import("@/views/MixView.vue") },
    { path: "/browse", name: "browse", component: () => import("@/views/BrowseView.vue") },
  ],
});

export default router;
