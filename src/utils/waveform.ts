/** 画一个周期所需的参数子集（从合成器参数表里取）。 */
export interface WaveParams {
  wave: number;
  sub: number;
  pwm: number;
  cutoff: number;
  resonance: number;
}

function waveSample(wave: number, phase: number, pulseWidth: number): number {
  const p = phase - Math.floor(phase);
  switch (Math.round(wave)) {
    case 1: return p < pulseWidth ? 1 : -1;
    case 2: return Math.sin(p * Math.PI * 2);
    case 3: return 4 * Math.abs(p - 0.5) - 1;
    default: return 2 * p - 1;
  }
}

/**
 * 一个基频周期的波形采样（解析生成，不是真实音频）。
 *
 * 跟随真实 DSP 的形状：主振 + 低八度方波副振 + 由「亮度/尖锐度」推导的一阶低通。
 * 于是转这些旋钮时屏幕上的波形会真实地变形——这正是给小白看的教学意图。
 */
export function sampleOnePeriod(params: WaveParams, points = 256): number[] {
  const n = Math.max(16, Math.floor(points));
  const pwm = 0.5 + (params.pwm - 0.5) * 0.9;
  const raw: number[] = new Array(n);
  for (let i = 0; i < n; i += 1) {
    const phase = i / n;
    let s = waveSample(params.wave, phase, pwm);
    if (params.sub > 0) s += params.sub * waveSample(1, phase * 0.5, 0.5);
    raw[i] = s;
  }
  // 「亮度」：截止越低越平滑；「尖锐度」让峰更尖（视觉上的共振）。
  const k = Math.min(1, Math.max(0.02, 0.03 + params.cutoff * 0.97));
  let prev = raw[0]!;
  for (let i = 0; i < n; i += 1) {
    prev += k * (raw[i]! - prev);
    raw[i] = prev;
  }
  let peak = 1e-6;
  for (const v of raw) peak = Math.max(peak, Math.abs(v));
  const gain = 0.92 + 0.06 * (1 - params.resonance);
  return raw.map((v) => (v / peak) * gain);
}
