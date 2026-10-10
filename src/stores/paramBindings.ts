/**
 * 参数绑定：哪些自动化目标键显示在车道下拉里。**纯 UI 偏好**，不进工程模型。
 *
 * 形状：`scopeKey -> 目标键数组`（键是 `inst:<id>` / `fx<i>:<id>` 那种，数组顺序即下拉顺序）。
 * 作用域带工程标识（`scopeKey`），因为轨道 id 是后端按 1..N 发的、每个工程都一样，
 * 不带工程就换工程会串。
 *
 * **`undefined` 与 `[]` 是两回事**：键不存在 = 「没有显式记录 → 用默认精选」；
 * `[]` = 「用户显式全解绑」。判断一律**直接读值**看是不是 `undefined`（`[]` 是真值，能区分开），
 * 绝不用 `?? []`；**也不要改用 `Object.hasOwn`**——那个 trap 不被 Vue 追踪，见 `explicit`。
 *
 * 与 `settings` 同款：初始化读一次 localStorage，`watch(deep)` 写回。
 */
import { defineStore } from "pinia";
import { ref, watch } from "vue";
import { useProjectStore } from "@/stores/project";
import { defaultBoundKeys } from "@/utils/automation";

/** 持久化键。 */
const KEY = "ai-daw-param-bindings";
/** 工程路径未知（未保存）时用的作用域前缀。 */
const UNTITLED = "untitled";

type Bindings = Record<string, string[]>;

function load(): Bindings {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return {};
    const parsed: unknown = JSON.parse(raw);
    // 合法 JSON 但形状不对（手工写进 `"null"` / 数组 / 标量）一律当空记录：否则
    // `bindings.value[scope]` 会在调用方的 computed 里对着 `null` 抛。
    if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) return {};
    return parsed as Bindings;
  } catch {
    return {};
  }
}

export const useParamBindingsStore = defineStore("paramBindings", () => {
  /** scopeKey -> 目标键数组（顺序即列表顺序）；键不存在 = 用默认精选。 */
  const bindings = ref<Bindings>(load());

  watch(
    bindings,
    () => {
      try {
        localStorage.setItem(KEY, JSON.stringify(bindings.value));
      } catch {
        // 存不住不影响本次会话（与 settings 同款）。
      }
    },
    { deep: true },
  );

  /**
   * 当前作用域键：工程路径（未保存时用 `untitled`）+ 轨道 id。
   *
   * `useProjectStore()` 在**函数里**取（不在 store 顶层）：`project.ts` 的保存路径要调
   * `rekeyProject`，两边互相 import——只有延迟取用才不成环。
   */
  function scopeKey(trackId: number): string {
    return `${useProjectStore().currentPath ?? UNTITLED}::${trackId}`;
  }

  /** 该作用域**显式绑定**的键；**没有记录时返回 `null`**（调用方据此走默认精选）。 */
  function explicit(trackId: number): string[] | null {
    const scope = scopeKey(trackId);
    // 必须**无条件读一次**：`Object.hasOwn` 走 `[[GetOwnProperty]]`，Vue 的响应式代理不追踪
    // 这个 trap——只 `hasOwn` 不 `get` 的话，调用方（computed）拿不到依赖，「记录从无到有」
    // 时不会重算：参数页里刚绑上的参数不会出现在车道下拉里，直到刷新页面。
    const stored = bindings.value[scope];
    return stored ? [...stored] : null;
  }

  /**
   * 是否**已经绑定**（生效状态）：有显式记录就看记录；没有记录则看该轨道的默认精选。
   *
   * 车道下拉里能看到的目标（增益 / 声像除外）都会返回 `true`——参数页的「已绑定」按钮
   * 与车道下拉必须一致，否则点一下会「反向」切换（toggle 会把默认精选实体化再增删）。
   */
  function isBound(trackId: number, key: string): boolean {
    const list = explicit(trackId);
    if (list) return list.includes(key);
    const track = useProjectStore().project?.tracks.find((t) => t.id === trackId) ?? null;
    return defaultBoundKeys(track).includes(key);
  }

  /**
   * 绑定 / 解绑（幂等切换）。
   *
   * 没有显式记录时，先把 `fallbackBound`（默认精选）**实体化**再改，否则「解绑一个精选参数」
   * 会变成「整组精选都消失」。显式记录为 `[]` 时保留空数组（**不要删键**），否则又会回落默认。
   */
  function toggle(trackId: number, key: string, fallbackBound: string[]): void {
    const scope = scopeKey(trackId);
    // 与 `explicit()` 同一个坑：**无条件读一次**（键不存在时也要读），写路径也保持一致、把依赖
    // 建立起来。`[]` 是真值 → 显式全解绑保持 `[]`；`undefined` = 没有记录 → 实体化 `fallbackBound`。
    const stored = bindings.value[scope];
    const current = stored === undefined ? [...fallbackBound] : stored;
    const at = current.indexOf(key);
    bindings.value[scope] = at >= 0
      ? [...current.slice(0, at), ...current.slice(at + 1)]
      : [...current, key];
  }

  /**
   * **单条原语**：把一个作用域键的记录搬到另一个。`rekeyProject` 对旧基下的每条轨道记录各调它一次。
   *
   * 新键已有记录则**不覆盖**（只丢掉旧的），否则用户已经保存过的绑定会被未保存的桶冲掉；随后删掉旧键。
   */
  function rekey(oldScope: string, newScope: string): void {
    if (oldScope === newScope) return;
    if (!Object.hasOwn(bindings.value, oldScope)) return;
    if (!Object.hasOwn(bindings.value, newScope)) {
      // 目标键已有记录则**不覆盖**——这个 tie-break 是有意的：另存为时优先保留（目标）新路径下的记录。
      bindings.value[newScope] = bindings.value[oldScope] ?? [];
    }
    delete bindings.value[oldScope];
  }

  /**
   * 保存工程后把**旧工程基下**的**每一条**轨道记录搬到**新工程基**——`project.ts` 的保存路径调用它。
   *
   * 未保存的工程用 `untitled` 当工程标识（见 `scopeKey`）：用户一保存，作用域键就从
   * `untitled::<trackId>` 变成 `<path>::<trackId>`，不搬的话刚绑好的参数会「变回默认」。
   * 另存为（`<旧 path>` → `<新 path>`）同理。
   *
   * 遍历**本 store 自己的键**（调用方只给两个工程基，不列轨道）：把 `${fromBase}::<trackId>` 逐条
   * 交给 `rekey`（新基已有同名轨道记录则**不覆盖**，只丢旧的），把该基下的所有轨道一次搬完。
   * 走的是私有常量 `UNTITLED` —— 调用方不需要知道「未保存」的基叫什么。
   */
  function rekeyProject(fromPath: string | null, toPath: string): void {
    const fromBase = fromPath ?? UNTITLED;
    if (fromBase === toPath) return;
    const prefix = `${fromBase}::`;
    // `Object.keys` 先取快照再改：搬进来的新键（`${toPath}::…`）前缀不同，不会被这轮再遍历到。
    for (const key of Object.keys(bindings.value)) {
      if (key.startsWith(prefix)) rekey(key, `${toPath}::${key.slice(prefix.length)}`);
    }
  }

  return { bindings, scopeKey, explicit, isBound, toggle, rekey, rekeyProject };
});
