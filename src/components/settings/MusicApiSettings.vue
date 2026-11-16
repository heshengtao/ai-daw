<script setup lang="ts">
/**
 * 设置 →「音乐 API」：管理云端音乐生成服务（Lyria / ElevenLabs / Mureka / Treblo）。
 *
 * 一家服务一个条目；密钥明文保存在应用配置目录的 `cloud-music.json`（与 `ai.json`
 * 同一套惯例）。保存后右栏「云端音乐生成」的下拉列表立即能看到，不需要重启。
 */
import { computed, onMounted, ref } from "vue";
import { ArrowTopRightOnSquareIcon } from "@heroicons/vue/24/outline";
import ConfirmDialog from "@/components/common/ConfirmDialog.vue";
import Md3Select from "@/components/common/Md3Select.vue";
import Md3Switch from "@/components/common/Md3Switch.vue";
import { cloudMusicSetConfig, cloudMusicTest } from "@/api/cloudMusic";
import type {
  CloudMusicKind,
  CloudMusicPreset,
  CloudMusicProvider,
} from "@/api/types";
import { useCloudMusicStore } from "@/stores/cloudMusic";
import { copyText } from "@/utils/clipboard";
import { useI18n } from "@/i18n";

const store = useCloudMusicStore();
const i18n = useI18n();

type Draft = CloudMusicProvider;

const draft = ref<Draft | null>(null);
const editingId = ref("");
const saving = ref(false);
const testing = ref(false);
const probe = ref("");
const probeOk = ref(false);
const formError = ref("");
const deleteOpen = ref(false);
const deleteTarget = ref<CloudMusicProvider | null>(null);

const kindOptions = computed(() =>
  store.presets.map((preset) => ({
    id: preset.kind,
    label: i18n.t(`settings.music.kind.${preset.kind}`),
    hint: preset.note,
  })),
);

const selectedPreset = computed<CloudMusicPreset | null>(
  () => store.presets.find((preset) => preset.kind === draft.value?.kind) ?? null,
);
const kindNote = computed(() => {
  const kind = selectedPreset.value?.kind;
  if (!kind) return "";
  const key = `settings.music.note.${kind}`;
  const text = i18n.t(key);
  return text === key ? selectedPreset.value?.note ?? "" : text;
});

function presetOf(kind: CloudMusicKind): CloudMusicPreset | null {
  return store.presets.find((preset) => preset.kind === kind) ?? null;
}

function newProvider(): void {
  const first = store.presets[0];
  if (!first) return;
  editingId.value = "";
  draft.value = {
    id: "",
    name: first.name,
    kind: first.kind,
    enabled: true,
    base_url: first.base_url,
    api_key: "",
    api_secret: "",
    model: first.model,
  };
  probe.value = "";
  formError.value = "";
}

function editProvider(provider: CloudMusicProvider): void {
  editingId.value = provider.id;
  draft.value = { ...provider };
  probe.value = "";
  formError.value = "";
}

/** 切换类型时，只覆盖「还是上一档默认值」的字段，避免把用户改过的名字/地址冲掉。 */
function onKindPick(kind: string): void {
  const current = draft.value;
  if (!current) return;
  const nextKind = kind as CloudMusicKind;
  const oldPreset = presetOf(current.kind);
  const nextPreset = presetOf(nextKind);
  current.kind = nextKind;
  if (!nextPreset) return;
  if (!current.name.trim() || current.name === oldPreset?.name) current.name = nextPreset.name;
  if (!current.base_url.trim() || current.base_url === oldPreset?.base_url) {
    current.base_url = nextPreset.base_url;
  }
  if (!current.model.trim() || current.model === oldPreset?.model) current.model = nextPreset.model;
}

async function save(): Promise<void> {
  const current = draft.value;
  if (!current || saving.value) return;
  if (!current.name.trim()) {
    formError.value = i18n.t("settings.music.nameRequired");
    return;
  }
  if (!current.base_url.trim()) {
    formError.value = i18n.t("settings.music.baseUrlRequired");
    return;
  }
  formError.value = "";
  probe.value = "";
  probeOk.value = false;
  saving.value = true;
  try {
    const next = [...store.providers];
    const index = next.findIndex((item) => item.id === current.id);
    const payload: CloudMusicProvider = {
      ...current,
      name: current.name.trim(),
      base_url: current.base_url.trim().replace(/\/+$/, ""),
      api_key: current.api_key.trim(),
      model: current.model.trim(),
    };
    if (index >= 0) next[index] = payload;
    else next.push(payload);
    const config = await cloudMusicSetConfig(next);
    store.providers = config.providers;
    store.loaded = true;
    if (!config.providers.some((item) => item.id === store.selectedId)) {
      store.select(config.providers[0]?.id ?? "");
    }
    const created =
      (payload.id
        ? config.providers.find((item) => item.id === payload.id)
        : config.providers[config.providers.length - 1]) ?? null;
    if (created) {
      editProvider(created);
      if (!store.selectedId) store.select(created.id);
    }
    probe.value = i18n.t("settings.music.saved");
    probeOk.value = true;
  } catch (err) {
    formError.value = err instanceof Error ? err.message : String(err);
  } finally {
    saving.value = false;
  }
}

async function copyDocs(): Promise<void> {
  const url = selectedPreset.value?.docs_url;
  if (!url) return;
  if (await copyText(url)) {
    probe.value = i18n.t("settings.music.docsCopied", { url });
    probeOk.value = true;
  }
}

async function test(): Promise<void> {
  const current = draft.value;
  if (!current || testing.value) return;
  testing.value = true;
  probe.value = "";
  probeOk.value = false;
  try {
    const detail = await cloudMusicTest({
      ...current,
      name: current.name.trim() || selectedPreset.value?.name || "Music API",
      base_url: current.base_url.trim().replace(/\/+$/, ""),
      api_key: current.api_key.trim(),
      model: current.model.trim(),
    });
    probe.value = detail;
    probeOk.value = true;
  } catch (err) {
    probe.value = err instanceof Error ? err.message : String(err);
    probeOk.value = false;
  } finally {
    testing.value = false;
  }
}

function askDelete(provider: CloudMusicProvider): void {
  deleteTarget.value = provider;
  deleteOpen.value = true;
}

async function confirmDelete(): Promise<void> {
  const target = deleteTarget.value;
  deleteOpen.value = false;
  deleteTarget.value = null;
  if (!target) return;
  try {
    const next = store.providers.filter((item) => item.id !== target.id);
    const config = await cloudMusicSetConfig(next);
    store.providers = config.providers;
    if (!config.providers.some((item) => item.id === store.selectedId)) {
      store.select(config.providers[0]?.id ?? "");
    }
    if (editingId.value === target.id) {
      draft.value = null;
      editingId.value = "";
    }
    if (store.selectedId === target.id) store.select(config.providers[0]?.id ?? "");
  } catch (err) {
    formError.value = err instanceof Error ? err.message : String(err);
  }
}

onMounted(async () => {
  await store.load(true);
  await store.loadPresets();
  if (store.providers[0]) editProvider(store.providers[0]);
  else newProvider();
});
</script>

<template>
  <div class="music-api">
    <div class="group">
      <div class="group-title">{{ i18n.t("settings.music.providers") }}</div>
      <div class="card">
        <p class="lead">{{ i18n.t("settings.music.lead") }}</p>
        <p class="lead scope">{{ i18n.t("settings.music.scopeNote") }}</p>
        <p v-if="store.loading" class="lead">{{ i18n.t("common.loading") }}</p>
        <p v-else-if="store.loadError" class="lead error">{{ store.loadError }}</p>
        <p v-else-if="store.providers.length === 0" class="lead">
          {{ i18n.t("settings.music.empty") }}
        </p>
        <div
          v-for="provider in store.providers"
          :key="provider.id"
          class="provider-row"
          :class="{ on: editingId === provider.id }"
          @click="editProvider(provider)"
        >
          <div class="provider-main">
            <span class="provider-name">{{ provider.name }}</span>
            <span class="provider-sub">
              {{ i18n.t(`settings.music.kind.${provider.kind}`) }} ·
              {{ provider.model || i18n.t("settings.music.defaultModel") }}
            </span>
          </div>
          <span class="provider-state" :class="{ on: provider.enabled }">
            {{
              provider.enabled
                ? i18n.t("settings.music.enabledOn")
                : i18n.t("settings.music.enabledOff")
            }}
          </span>
          <button class="scan-btn" @click.stop="askDelete(provider)">
            {{ i18n.t("common.delete") }}
          </button>
        </div>
        <div class="actions">
          <button class="scan-btn primary" @click="newProvider">
            {{ i18n.t("settings.music.add") }}
          </button>
        </div>
      </div>
    </div>

    <div v-if="draft" class="group">
      <div class="group-title">
        {{ i18n.t(editingId ? "settings.music.edit" : "settings.music.new") }}
      </div>
      <div class="card">
        <div class="row">
          <div class="row-text">
            <span class="row-title">{{ i18n.t("settings.music.name") }}</span>
            <span class="row-sub">{{ i18n.t("settings.music.nameHint") }}</span>
          </div>
          <input v-model="draft.name" class="net-input" spellcheck="false" />
        </div>

        <div class="row">
          <div class="row-text">
            <span class="row-title">{{ i18n.t("settings.music.kind") }}</span>
            <span class="row-sub">{{ kindNote }}</span>
          </div>
          <Md3Select
            :model-value="draft.kind"
            :options="kindOptions"
            :label="i18n.t('settings.music.kind')"
            @update:model-value="onKindPick"
          />
        </div>

        <div class="row">
          <div class="row-text">
            <span class="row-title">{{ i18n.t("settings.music.enabled") }}</span>
            <span class="row-sub">{{ i18n.t("settings.music.enabledHint") }}</span>
          </div>
          <Md3Switch
            v-model="draft.enabled"
            :label="i18n.t('settings.music.enabled')"
          />
        </div>

        <div class="row">
          <div class="row-text">
            <span class="row-title">{{ i18n.t("settings.music.baseUrl") }}</span>
            <span class="row-sub">
              {{ i18n.t("settings.music.baseUrlHint") }}
              <template v-if="draft.kind === 'alibaba'">
                {{ i18n.t("settings.music.alibabaBaseHint") }}
              </template>
            </span>
          </div>
          <input
            v-model="draft.base_url"
            class="net-input"
            spellcheck="false"
            :placeholder="selectedPreset?.base_url"
          />
        </div>

        <div class="row">
          <div class="row-text">
            <span class="row-title">{{ i18n.t("settings.music.apiKey") }}</span>
            <span class="row-sub">{{ i18n.t("settings.music.apiKeyHint") }}</span>
          </div>
          <input
            v-model="draft.api_key"
            class="net-input"
            type="password"
            spellcheck="false"
            placeholder="sk-…"
          />
        </div>

        <div v-if="draft.kind === 'volcengine' || draft.kind === 'tencent'" class="row">
          <div class="row-text">
            <span class="row-title">{{ i18n.t("settings.music.apiSecret") }}</span>
            <span class="row-sub">{{ i18n.t("settings.music.apiSecretHint") }}</span>
          </div>
          <input
            v-model="draft.api_secret"
            class="net-input"
            type="password"
            spellcheck="false"
            placeholder="SecretKey"
          />
        </div>

        <div class="row">
          <div class="row-text">
            <span class="row-title">{{ i18n.t("settings.music.model") }}</span>
            <span class="row-sub">{{ i18n.t("settings.music.modelHint") }}</span>
          </div>
          <input
            v-model="draft.model"
            class="net-input"
            spellcheck="false"
            :placeholder="selectedPreset?.model || i18n.t('settings.music.defaultModel')"
          />
        </div>

        <div class="row docs-row">
          <div class="row-text">
            <span class="row-sub">
              {{ i18n.t("settings.music.maxSeconds", { seconds: selectedPreset?.max_seconds ?? 0 }) }}
            </span>
          </div>
          <button v-if="selectedPreset" class="docs-link" @click="copyDocs">
            {{ i18n.t("settings.music.docs") }}
            <ArrowTopRightOnSquareIcon class="w-[12px] h-[12px]" />
          </button>
        </div>

        <div class="row actions">
          <span class="probe" :class="{ ok: probeOk, error: probe && !probeOk }">
            {{ probe || formError || i18n.t("settings.music.testHint") }}
          </span>
          <button class="scan-btn" :disabled="testing || saving" @click="test">
            {{ testing ? i18n.t("settings.music.testing") : i18n.t("settings.music.test") }}
          </button>
          <button class="scan-btn primary" :disabled="saving" @click="save">
            {{ saving ? i18n.t("settings.music.saving") : i18n.t("settings.music.save") }}
          </button>
        </div>
      </div>
    </div>

    <ConfirmDialog
      v-model:open="deleteOpen"
      :title="i18n.t('settings.music.deleteTitle')"
      :message="i18n.t('settings.music.deleteMessage', { name: deleteTarget?.name ?? '' })"
      :confirm-label="i18n.t('common.delete')"
      @confirm="confirmDelete"
    />
  </div>
</template>

<style scoped>
.music-api {
  display: flex;
  flex-direction: column;
}
.group + .group {
  margin-top: 26px;
}
.group-title {
  font-size: 13px;
  font-weight: 600;
  color: rgb(var(--md-on-surface));
  margin: 0 0 10px 2px;
}
.card {
  padding: 4px 16px;
  border-radius: 14px;
  background-color: rgb(var(--md-surface-container));
}
.lead {
  margin: 12px 0;
  font-size: 12px;
  line-height: 1.6;
  color: rgb(var(--md-on-surface-variant));
}
.lead.error {
  color: rgb(var(--md-error));
}
.provider-row {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px 0;
  border-bottom: 1px solid rgb(var(--md-outline-variant) / 0.5);
  cursor: pointer;
}
.provider-row.on {
  background-color: rgb(var(--md-primary) / 0.06);
}
.provider-main {
  flex: 1 1 auto;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 3px;
}
.provider-name {
  font-size: 13px;
  font-weight: 500;
  color: rgb(var(--md-on-surface));
}
.provider-sub {
  font-size: 11px;
  color: rgb(var(--md-on-surface-variant));
}
.provider-state {
  flex: 0 0 auto;
  font-size: 11px;
  color: rgb(var(--md-on-surface-variant));
}
.provider-state.on {
  color: rgb(var(--md-primary));
}
.row {
  display: flex;
  align-items: center;
  gap: 16px;
  padding: 13px 0;
  border-bottom: 1px solid rgb(var(--md-outline-variant) / 0.5);
}
.row:last-child {
  border-bottom: none;
}
.row-text {
  flex: 1 1 auto;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 3px;
}
.row-title {
  font-size: 12.5px;
  color: rgb(var(--md-on-surface));
}
.row-sub {
  font-size: 11px;
  line-height: 1.45;
  color: rgb(var(--md-on-surface-variant));
}
.net-input {
  flex: 0 0 260px;
  max-width: 52%;
  height: 34px;
  padding: 0 10px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.7);
  border-radius: 9px;
  background-color: rgb(var(--md-surface-container-high));
  color: rgb(var(--md-on-surface));
  font-size: 12px;
  font-family: inherit;
}
.net-input:focus {
  outline: none;
  border-color: rgb(var(--md-primary) / 0.8);
}
.actions {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 8px;
  padding: 12px 0;
}
.docs-row {
  align-items: center;
}
.docs-link {
  display: inline-flex;
  border: 0;
  background: transparent;
  cursor: pointer;
  font-family: inherit;
  align-items: center;
  gap: 4px;
  font-size: 11px;
  color: rgb(var(--md-primary));
  text-decoration: none;
}
.probe {
  flex: 1 1 auto;
  min-width: 0;
  font-size: 11px;
  line-height: 1.45;
  color: rgb(var(--md-on-surface-variant));
  overflow-wrap: anywhere;
}
.probe.ok {
  color: rgb(var(--md-primary));
}
.probe.error {
  color: rgb(var(--md-error));
}
.scan-btn {
  flex: 0 0 auto;
  height: 30px;
  padding: 0 12px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.8);
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface));
  font-size: 11.5px;
  cursor: pointer;
}
.scan-btn.primary {
  border-color: transparent;
  background-color: rgb(var(--md-primary));
  color: rgb(var(--md-on-primary));
}
.scan-btn:disabled {
  opacity: 0.45;
  cursor: default;
}
@media (max-width: 720px) {
  .row {
    flex-wrap: wrap;
  }
  .net-input {
    flex: 1 1 100%;
    max-width: none;
  }
}
</style>
