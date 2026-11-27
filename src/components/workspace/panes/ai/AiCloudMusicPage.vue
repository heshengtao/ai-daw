<script setup lang="ts">
/**
 * **云端音乐生成**子页：选中的服务 + 描述 / 歌词 / 时长 / 纯音乐开关。
 *
 * 进度走 `cloudMusic` store 的 `cloud-music-progress`；命令在成品落盘并加入工程后才返回，
 * 所以这里只需要在 `generate()` 返回时提示一次。页面本身不碰 HTTP，也不碰工程。
 */
import { computed, onMounted, ref, watch } from "vue";
import { ArrowLeftIcon } from "@heroicons/vue/24/outline";
import NumberField from "@/components/common/NumberField.vue";
import Md3Switch from "@/components/common/Md3Switch.vue";
import { useCloudMusicStore } from "@/stores/cloudMusic";
import { useLayoutStore } from "@/stores/layout";
import { useI18n } from "@/i18n";

const props = defineProps<{ providerId: string }>();
const emit = defineEmits<{ (e: "back"): void }>();

const store = useCloudMusicStore();
const layout = useLayoutStore();
const i18n = useI18n();

const prompt = ref("");
const lyrics = ref("");
const duration = ref(120);
const instrumental = ref(false);

const provider = computed(
  () => store.providers.find((item) => item.id === props.providerId) ?? null,
);
const maxSeconds = computed(() => {
  const preset = store.presets.find((item) => item.kind === provider.value?.kind);
  return preset?.max_seconds ?? 300;
});
/** Mureka / 阿里 / 腾讯没有时长参数：整首长度由歌词与模型决定。 */
const durationApplies = computed(
  () => !["mureka", "alibaba", "tencent"].includes(provider.value?.kind ?? ""),
);

watch(maxSeconds, (max) => {
  if (duration.value > max) duration.value = max;
});

const statusLine = computed(() => {
  if (store.running) {
    return store.progress?.message || i18n.t("ai.cloudMusicBusy");
  }
  if (store.error) return store.error;
  if (store.result) {
    return i18n.t("ai.cloudMusicDone", {
      provider: store.result.provider,
      seconds: store.result.duration.toFixed(1),
    });
  }
  return i18n.t("ai.cloudMusicHint");
});

function start(): void {
  const current = provider.value;
  if (!current) return;
  void store
    .generate({
      provider_id: current.id,
      prompt: prompt.value,
      lyrics: instrumental.value ? "" : lyrics.value,
      duration_seconds: duration.value,
      instrumental: instrumental.value,
    })
    .then((result) => {
      if (result) {
        layout.notify(
          i18n.t("ai.cloudMusicDone", {
            provider: result.provider,
            seconds: result.duration.toFixed(1),
          }),
        );
      }
    });
}

function stop(): void {
  void store.cancel();
}

onMounted(async () => {
  await store.load();
  await store.loadPresets();
  store.watch();
  void store.refreshBusy();
});
</script>

<template>
  <section class="ai-subpage">
    <div class="sub-head">
      <button class="sub-icon" :aria-label="i18n.t('common.back')" @click="emit('back')">
        <ArrowLeftIcon class="w-[15px] h-[15px] rtl-flip" />
      </button>
      <h2 class="sub-title">{{ i18n.t("ai.cloudMusicPageTitle") }}</h2>
      <span v-if="provider" class="sub-badge on">{{ provider.name }}</span>
    </div>

    <p v-if="!provider" class="sub-status">{{ i18n.t("ai.cloudMusicNeedsProvider") }}</p>

    <template v-else>
      <div class="sub-card">
        <label class="sub-field">
          <span class="sub-name">{{ i18n.t("ai.cloudMusicPrompt") }}</span>
          <textarea
            v-model="prompt"
            class="sub-input sub-textarea"
            :placeholder="i18n.t('ai.cloudMusicPromptPlaceholder')"
            :disabled="store.running"
          />
        </label>

        <label class="sub-field">
          <span class="sub-name">{{ i18n.t("ai.cloudMusicLyrics") }}</span>
          <textarea
            v-model="lyrics"
            class="sub-input sub-textarea"
            :placeholder="i18n.t('ai.cloudMusicLyricsPlaceholder')"
            :disabled="store.running || instrumental"
          />
        </label>
        <p class="sub-hint">{{ i18n.t("ai.cloudMusicLyricsHint") }}</p>

        <div class="sub-row">
          <label v-if="durationApplies" class="sub-field">
            <span class="sub-name">{{ i18n.t("ai.cloudMusicDuration") }}</span>
            <NumberField
              v-model="duration"
              :min="30"
              :max="maxSeconds"
              :step="30"
              :decimals="0"
              :sr-label="i18n.t('ai.cloudMusicDuration')"
              :disabled="store.running"
            />
          </label>
          <div class="sub-field">
            <span class="sub-name">{{ i18n.t("ai.cloudMusicInstrumental") }}</span>
            <div class="sub-inline">
              <Md3Switch
                v-model="instrumental"
                :label="i18n.t('ai.cloudMusicInstrumental')"
                :disabled="store.running"
              />
              <span class="sub-hint">{{ i18n.t("ai.cloudMusicInstrumentalHint") }}</span>
            </div>
          </div>
        </div>

        <p v-if="!durationApplies" class="sub-hint">{{ i18n.t("ai.cloudMusicDurationAuto") }}</p>
        <p class="sub-hint">{{ i18n.t("ai.cloudMusicCostHint") }}</p>

        <div class="sub-actions">
          <button v-if="store.running" class="sub-ghost" @click="stop">
            {{ i18n.t("ai.stop") }}
          </button>
          <button
            class="sub-primary"
            :disabled="store.running || (!prompt.trim() && !lyrics.trim())"
            @click="start"
          >
            {{ store.running ? i18n.t("ai.cloudMusicBusy") : i18n.t("ai.cloudMusicGenerate") }}
          </button>
        </div>

        <div v-if="store.running" class="sub-progress">
          <div
            class="sub-progress-fill"
            :class="{ indeterminate: store.progressPct === null }"
            :style="store.progressPct === null ? undefined : { width: `${store.progressPct}%` }"
          />
        </div>
      </div>

      <p class="sub-status" :class="{ error: store.error }">{{ statusLine }}</p>
      <p v-if="store.result" class="sub-hint path">{{ store.result.path }}</p>
    </template>
  </section>
</template>

<style scoped>
.sub-status.error {
  color: rgb(var(--md-error));
}
.path {
  word-break: break-all;
}
</style>
