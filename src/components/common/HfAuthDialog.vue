<script setup lang="ts">
/**
 * **Hugging Face 授权框**：模型是 gated 仓库，匿名下载被 `403 GatedRepo` 挡住。
 *
 * 三步走，正好对应 HF 那边的流程：
 *   1. 「去同意许可」→ 打开仓库页（用户点 Agree and access repository）；
 *   2. 「创建 read token」→ 打开 HF 的 token 页（`tokenType=read`）；
 *   3. 把 token 粘进来 → 保存并重试刚才那次下载。
 *
 * 为什么必须让用户手动粘：HF 只对 OAuth 应用回传 token，桌面端应用没有 client_id；
 * 而网页上点「同意」并不会把凭据给第三方。token 存进本机设置（`store.json`），
 * 只发给 `huggingface.co` 官方域。
 *
 * 视觉与 `ConfirmDialog` 同一套外壳（Teleport → 遮罩 → 圆角对话框）。
 */
import { computed, ref, watch } from "vue";
import { XMarkIcon } from "@heroicons/vue/24/outline";
import { useI18n } from "@/i18n";
import { useModelsStore } from "@/stores/models";
import { openExternal } from "@/utils/external";

const i18n = useI18n();
const models = useModelsStore();

const token = ref("");
const saving = ref(false);
const failure = ref<string | null>(null);

const open = computed(() => models.pendingAuth !== null);
const auth = computed(() => models.pendingAuth?.auth ?? null);

/** 打开就清空上一次的输入 / 报错，避免「上次粘错的 token」留在框里。 */
watch(open, (value) => {
  if (value) {
    token.value = "";
    failure.value = null;
    saving.value = false;
  }
});

function close(): void {
  models.dismissAuth();
}

function openPage(): void {
  const url = auth.value?.page_url;
  if (url) void openExternal(url);
}

function openTokenPage(): void {
  const url = auth.value?.token_url;
  if (url) void openExternal(url);
}

async function submit(): Promise<void> {
  const value = token.value.trim();
  if (!value || saving.value) return;
  saving.value = true;
  failure.value = null;
  try {
    await models.authorizeAndRetry(value);
  } catch (err) {
    // 重试仍失败（token 没该仓库权限 / 还没点同意）时留在框里，别把用户踢出去。
    failure.value = err instanceof Error ? err.message : String(err);
  } finally {
    saving.value = false;
  }
}

function onKey(e: KeyboardEvent): void {
  if (e.key === "Escape") {
    e.stopPropagation();
    close();
  } else if (e.key === "Enter" && !e.isComposing) {
    void submit();
  }
}
</script>

<template>
  <Teleport to="body">
    <transition name="fade">
      <div v-if="open" class="scrim" @pointerdown.self="close">
        <div
          class="dialog"
          role="dialog"
          aria-modal="true"
          :aria-label="i18n.t('models.auth.title')"
          tabindex="-1"
          @keydown="onKey"
        >
          <header class="head">
            <h2 class="title">{{ i18n.t("models.auth.title") }}</h2>
            <button class="icon-btn" :aria-label="i18n.t('common.close')" @click="close">
              <XMarkIcon class="w-[18px] h-[18px]" />
            </button>
          </header>

          <div class="body">
            <p class="message">
              {{ i18n.t("models.auth.message", { repo: auth?.repo ?? "" }) }}
            </p>

            <ol class="steps">
              <li>
                <button class="link" @click="openPage">{{ auth?.page_url }}</button>
                <span class="step-note">{{ i18n.t("models.auth.stepAgree") }}</span>
              </li>
              <li>
                <button class="link" @click="openTokenPage">{{ i18n.t("models.auth.tokenLink") }}</button>
                <span class="step-note">{{ i18n.t("models.auth.stepToken") }}</span>
              </li>
            </ol>

            <label class="field">
              <span class="label">{{ i18n.t("models.auth.tokenLabel") }}</span>
              <input
                v-model="token"
                type="password"
                class="input"
                autocomplete="off"
                spellcheck="false"
                :placeholder="i18n.t('models.auth.tokenPlaceholder')"
              />
            </label>

            <p v-if="failure" class="failure">{{ failure }}</p>
            <p class="hint">{{ i18n.t("models.auth.hint") }}</p>
          </div>

          <footer class="foot">
            <button class="btn-text" @click="close">{{ i18n.t("common.cancel") }}</button>
            <button class="btn-filled" :disabled="!token.trim() || saving" @click="submit">
              {{ saving ? i18n.t("models.auth.saving") : i18n.t("models.auth.retry") }}
            </button>
          </footer>
        </div>
      </div>
    </transition>
  </Teleport>
</template>

<style scoped>
.scrim {
  position: fixed;
  inset: 0;
  z-index: 140;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 24px;
  background-color: rgb(var(--md-scrim) / 0.42);
}
.dialog {
  width: 440px;
  max-width: 100%;
  display: flex;
  flex-direction: column;
  border-radius: 24px;
  background-color: rgb(var(--md-surface-container-high));
  box-shadow: var(--md-elevation-3);
  overflow: hidden;
}
.head {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 16px 16px 8px 22px;
}
.title {
  flex: 1 1 auto;
  margin: 0;
  font-size: 16px;
  font-weight: 600;
  color: rgb(var(--md-on-surface));
}
.icon-btn {
  width: 34px;
  height: 34px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 0;
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  cursor: pointer;
}
.icon-btn:hover {
  background-color: rgb(var(--md-on-surface-variant) / var(--md-state-hover-alpha));
  color: rgb(var(--md-on-surface));
}
.body {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 4px 22px 10px;
}
.message {
  margin: 0;
  font-size: 14px;
  line-height: 1.5;
  color: rgb(var(--md-on-surface-variant));
}
.steps {
  margin: 0;
  padding-left: 18px;
  display: flex;
  flex-direction: column;
  gap: 8px;
  font-size: 13px;
  line-height: 1.5;
  color: rgb(var(--md-on-surface-variant));
}
.link {
  border: 0;
  padding: 0;
  background: transparent;
  color: rgb(var(--md-primary));
  font-family: inherit;
  font-size: 13px;
  text-align: start;
  text-decoration: underline;
  cursor: pointer;
  word-break: break-all;
}
.step-note {
  display: block;
}
.field {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.label {
  font-size: 12px;
  color: rgb(var(--md-on-surface-variant));
}
.input {
  height: 38px;
  padding: 0 12px;
  border: 1px solid rgb(var(--md-outline) / 0.5);
  border-radius: 10px;
  background-color: rgb(var(--md-surface-container-lowest));
  color: rgb(var(--md-on-surface));
  font-size: 13px;
  font-family: inherit;
}
.input:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
.failure {
  margin: 0;
  font-size: 12px;
  line-height: 1.5;
  color: rgb(var(--md-error));
}
.hint {
  margin: 0;
  font-size: 12px;
  line-height: 1.5;
  color: rgb(var(--md-on-surface-variant) / 0.8);
}
.foot {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 8px;
  padding: 12px 18px 16px;
}
.btn-text {
  height: 36px;
  padding: 0 16px;
  border: 0;
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-primary));
  font-size: 13px;
  font-weight: 500;
  cursor: pointer;
}
.btn-text:hover {
  background-color: rgb(var(--md-primary) / 0.08);
}
.btn-filled {
  height: 36px;
  padding: 0 20px;
  border: 0;
  border-radius: 999px;
  background-color: rgb(var(--md-primary));
  color: rgb(var(--md-on-primary));
  font-size: 13px;
  font-weight: 500;
  cursor: pointer;
}
.btn-filled:disabled {
  opacity: 0.45;
  cursor: default;
}
.btn-filled:hover:not(:disabled) {
  filter: brightness(1.06);
}
</style>
