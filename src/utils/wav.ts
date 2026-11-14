/**
 * 浏览器录音 → WAV 编码。
 *
 * 为什么不用 MediaRecorder 直接产出的 webm：容器里的音频导入走 `crates/asset`
 * （symphonia），webm/opus 不一定在每个构建里都可用；WAV 是两边都必然支持的格式，
 * 而且 16-bit PCM 足够做录音素材（桌面端录音也是写 WAV）。
 */

/** 把一个 `AudioBuffer` 编码成 16-bit PCM 的 WAV（多声道交错）。 */
export function encodeWav(buffer: AudioBuffer): ArrayBuffer {
  const channels = Math.max(1, buffer.numberOfChannels);
  const frames = buffer.length;
  const sampleRate = buffer.sampleRate;
  const bytesPerSample = 2;
  const blockAlign = channels * bytesPerSample;
  const dataBytes = frames * blockAlign;
  const out = new ArrayBuffer(44 + dataBytes);
  const view = new DataView(out);

  const writeText = (offset: number, text: string): void => {
    for (let i = 0; i < text.length; i += 1) view.setUint8(offset + i, text.charCodeAt(i));
  };

  writeText(0, "RIFF");
  view.setUint32(4, 36 + dataBytes, true);
  writeText(8, "WAVE");
  writeText(12, "fmt ");
  view.setUint32(16, 16, true); // PCM 头长度
  view.setUint16(20, 1, true); // format = PCM
  view.setUint16(22, channels, true);
  view.setUint32(24, sampleRate, true);
  view.setUint32(28, sampleRate * blockAlign, true); // 字节率
  view.setUint16(32, blockAlign, true);
  view.setUint16(34, bytesPerSample * 8, true);
  writeText(36, "data");
  view.setUint32(40, dataBytes, true);

  const datas: Float32Array[] = [];
  for (let channel = 0; channel < channels; channel += 1) {
    datas.push(buffer.getChannelData(channel));
  }
  let offset = 44;
  for (let frame = 0; frame < frames; frame += 1) {
    for (let channel = 0; channel < channels; channel += 1) {
      // 夹到 [-1, 1]：削波比静音更糟，宁可硬限幅。
      const value = Math.max(-1, Math.min(1, datas[channel][frame] ?? 0));
      view.setInt16(offset, value < 0 ? value * 0x8000 : value * 0x7fff, true);
      offset += 2;
    }
  }
  return out;
}
