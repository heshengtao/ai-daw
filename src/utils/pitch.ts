/**
 * 音高偏移的「人话」换算。
 *
 * 引擎内部一律用音分（cents，100 = 一个半音）存储，但界面上把 25¢ / 50¢ 摆给用户
 * 并不友好——所有输入都以**半音**为单位，这里负责双向换算与音程命名。
 */

/** 半音数 → i18n key（0..11，12 单独走八度）。 */
const INTERVAL_KEYS = [
  "pitch.unison",
  "pitch.minor2",
  "pitch.major2",
  "pitch.minor3",
  "pitch.major3",
  "pitch.perfect4",
  "pitch.tritone",
  "pitch.perfect5",
  "pitch.minor6",
  "pitch.major6",
  "pitch.minor7",
  "pitch.major7",
] as const;

/** 音分 → 半音（保留到音分精度）。 */
export function centsToSemitones(cents: number): number {
  return Math.round(cents) / 100;
}

/** 半音 → 音分（四舍五入回整数音分）。 */
export function semitonesToCents(semitones: number): number {
  return Math.round(semitones * 100);
}

/** 把音分显示成带符号的半音数：`+2` / `-0.35` / `0`。 */
export function formatSemitones(cents: number): string {
  const semis = centsToSemitones(cents);
  if (semis === 0) return "0";
  return `${semis > 0 ? "+" : "-"}${Math.abs(semis)}`;
}

/** 数值对应的音程名（i18n key）；没有通用名称时返回空串。 */
export function intervalKey(cents: number): string {
  const semis = centsToSemitones(cents);
  if (semis === 0) return "pitch.unison";
  const abs = Math.abs(semis);
  if (abs === 0.5) return "pitch.quarterTone";
  if (abs === 12) return "pitch.octave";
  if (!Number.isInteger(abs) || abs > 12) return "";
  return INTERVAL_KEYS[abs % 12] ?? "";
}
