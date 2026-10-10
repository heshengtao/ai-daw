/**
 * 参数页（模态）的打开状态：是否打开、针对哪条轨道、要不要预选某个插件。
 *
 * 极轻的一个 store，**不持久化**（关掉就该是关着的）。放进 store 是因为它有两个**跨视图**的
 * 入口——车道表头的「＋ 添加参数…」与检查器插件卡片的「参数…」——而挂载点在 `App.vue` 根部：
 * 两条路都往这里放一个「打开哪条轨道」，模态自己读。
 */
import { defineStore } from "pinia";
import { ref } from "vue";

export const useParamBrowserStore = defineStore("paramBrowser", () => {
  /** 模态是否打开。 */
  const isOpen = ref(false);
  /** 作用域轨道（参数页永远是「某一条轨道上的全部插件参数」）。 */
  const trackId = ref<number | null>(null);
  /** 打开时预选的插件 id（检查器入口会给；车道入口不给 → 筛选停在「全部」）。 */
  const pluginId = ref<string | null>(null);

  /** 打开参数页。 */
  function open(opts: { trackId: number; pluginId?: string | null }): void {
    trackId.value = opts.trackId;
    pluginId.value = opts.pluginId ?? null;
    isOpen.value = true;
  }

  /** 关闭参数页（作用域留着无所谓：下次 `open` 会覆盖）。 */
  function close(): void {
    isOpen.value = false;
  }

  return { isOpen, trackId, pluginId, open, close };
});
