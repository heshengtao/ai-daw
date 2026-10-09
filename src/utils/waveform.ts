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

/** 主振 + 低八度方波副振在相位 `i/n` 处的原始样本。 */
function oscSample(wave: number, sub: number, pwm: number, i: number, n: number): number {
  const phase = i / n;
  let s = waveSample(wave, phase, pwm);
  if (sub > 0) s += sub * waveSample(1, phase * 0.5, 0.5);
  return s;
}

/** 夹到 [lo, hi]；非有限值（NaN/±Inf）回落到 lo，避免脏输入污染滤波。 */
function clamp(v: number, lo: number, hi: number): number {
  if (!Number.isFinite(v)) return lo;
  return v < lo ? lo : v > hi ? hi : v;
}

/**
 * 一个基频周期的波形采样（解析生成，不是真实音频）。
 *
 * 跟随真实 DSP 的形状：主振 + 低八度方波副振 → 一阶低通（由「亮度」定截止）
 * 再接一层共振反馈（由「尖锐度」定 Q），最后按「尖锐度」做峰值锐化。
 * 于是转这些旋钮时屏幕上的波形都会真实地变形——这正是给小白看的教学意图。
 */
export function sampleOnePeriod(params: WaveParams, points = 256): number[] {
  const n = Number.isFinite(points) ? Math.max(16, Math.floor(points)) : 256;
  const pwm = 0.5 + (params.pwm - 0.5) * 0.9;
  const sub = clamp(params.sub, -8, 8);
  const wave = params.wave;

  // 「亮度」：截止越低越平滑。k 是低通系数，越小越暗。
  const k = clamp(0.03 + params.cutoff * 0.97, 0.02, 1);
  // 「尖锐度」：共振反馈量 fb。fb 越大，截止附近的过渡沿越冲过头、越振铃，
  // 峰越尖——这正是屏幕上要看得见的「共振」。fb 始终 < 1，滤波极点必落在
  // 单位圆内，所以任何参数下输出都稳定有限（无 NaN/Inf、无发散）。
  const res = clamp(params.resonance, 0, 1);
  const fb = res * 0.95;

  // 带共振反馈的低通：
  //   y[n] = y[n-1] + k·(x[n] − y[n-1] + fb·(y[n-1] − y[n-2]))
  // fb = 0 退化为一阶低通；fb 越大极点越接近单位圆 → 过渡沿出现过冲/振铃。
  let y1 = 0;
  let y2 = 0;
  // 先环形预热一个周期，让滤波器进入「周期稳态」：这样周期接缝处的跳变
  // （例如锯齿回折）也会真实地激起共振，屏幕上才看得到接缝处的过冲。
  for (let i = 0; i < n; i += 1) {
    const x = oscSample(wave, sub, pwm, i, n);
    const y = y1 + k * (x - y1 + fb * (y1 - y2));
    y2 = y1;
    y1 = y;
  }
  const data: number[] = new Array(n);
  for (let i = 0; i < n; i += 1) {
    const x = oscSample(wave, sub, pwm, i, n);
    const y = y1 + k * (x - y1 + fb * (y1 - y2));
    y2 = y1;
    y1 = y;
    data[i] = y;
  }

  // 归一化到约 [-1,1] 并留边距。
  let peak = 1e-6;
  for (const v of data) peak = Math.max(peak, Math.abs(v));
  // 光靠共振反馈对正弦这类「无跳变」的波形只能改幅度、形状不变（归一化后等于没变），
  // 所以再按「尖锐度」叠一层峰值锐化，让任何波形都肉眼可见地更尖。
  const sharp = 1 / (1 + res);
  for (let i = 0; i < n; i += 1) {
    const v = (data[i]! / peak) * 0.95;
    data[i] = Math.sign(v) * Math.pow(Math.abs(v), sharp);
  }
  return data;
}
