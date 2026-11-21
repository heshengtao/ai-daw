/**
 * 风格预设：点一下就把一整串英文 tag 插进提示词。
 *
 * 为什么要「整串」而不是一个词：YuE2 的 `style` 是英文 tag 分布，乐器与 BPM 是风格的
 * 主要锚点——只写「jazz」，模型会退回到自己的流行歌先验（实测生成出 Bb 调的标准流行歌、
 * intro/verse/chorus 三段式）。`label` 是 i18n key（芯片上显示中文/英文短名），
 * `tags` 永远是英文，直接进请求体。
 */
export interface StylePreset {
  id: string;
  /** i18n key：`inspire.stylePreset.<id>`。 */
  label: string;
  /** 插进提示词的英文 tag 串（含乐器与 BPM）。 */
  tags: string;
}

export const STYLE_PRESETS: StylePreset[] = [
  {
    id: "jazz",
    label: "inspire.stylePreset.jazz",
    tags: "swing jazz, walking upright bass, brushed drums, comping piano, tenor saxophone, 96 BPM",
  },
  {
    id: "pop",
    label: "inspire.stylePreset.pop",
    tags: "contemporary pop, bright acoustic guitar, punchy drums, warm bass, catchy hook, 104 BPM",
  },
  {
    id: "rock",
    label: "inspire.stylePreset.rock",
    tags: "indie rock, overdriven electric guitar, live drums, driving bass, 128 BPM",
  },
  {
    id: "lofi",
    label: "inspire.stylePreset.lofi",
    tags: "lo-fi hip hop, dusty vinyl crackle, mellow Rhodes piano, soft boom-bap drums, 78 BPM",
  },
  {
    id: "citypop",
    label: "inspire.stylePreset.citypop",
    tags: "city pop, slap bass, chorus electric guitar, gated reverb drums, dreamy pads, 112 BPM",
  },
  {
    id: "edm",
    label: "inspire.stylePreset.edm",
    tags: "melodic house, four-on-the-floor kick, sidechained pads, plucky synth arp, 124 BPM",
  },
  {
    id: "hiphop",
    label: "inspire.stylePreset.hiphop",
    tags: "boom bap hip hop, sampled soul horns, dusty drums, deep sub bass, 88 BPM",
  },
  {
    id: "cnfolk",
    label: "inspire.stylePreset.cnfolk",
    tags: "Chinese folk, guzheng, dizi flute, erhu, modern arrangement, 90 BPM",
  },
  {
    id: "cinematic",
    label: "inspire.stylePreset.cinematic",
    tags: "cinematic orchestral, string swells, epic brass, timpani, choir pads, 72 BPM",
  },
  {
    id: "ambient",
    label: "inspire.stylePreset.ambient",
    tags: "ambient, warm analog pads, slow evolving textures, no drums, 60 BPM",
  },
];
