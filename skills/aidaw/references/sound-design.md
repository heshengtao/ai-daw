# 域：音色设计（sound-design）

**只讲内置合成器 `builtin.synth` 怎么捏。** 采样乐器只有 `gain` / `tone`，挑它们看 `plugin.md`；
外部 VST3 / CLAP 的参数表用 `describe_plugin` 拿，本文的参数名不通用。

内置合成器是**一台 16 复音减法合成器**，信号链固定：

```
主振（锯齿/方波/正弦/三角）
  + unison 副本（1..4 个，detune 控制展开量）
  + 低八度方波 sub
  + 白噪声
  + FM（正弦调制器，相位调制）
→ 一阶高通 → 2 极点状态变量滤波（LP/BP/HP，包络可独立）
→ 位深压碎 → VCA（线性 ADSR）→ gain → 硬限幅 ±1
→ 声部分布（spread：0 号声部居中，其余左右交替）
```

LFO 默认是**全局**三角波（所有声部共用）；`lfoMode = 1` 时每个声部用自己的相位与延迟。
声部内部可选 **2x 过采样**（`oversample = 1`），对振荡器/FM/滤波整条路径过采样后再半带抽取。

## 一步到位的工作流

1. **先读这条轨道现在是什么**：`get_project` 看 `instrument` / `params`，别在别人的音色上盲调。
2. **从配方开始**：下面「配方」给的是**补丁**（只列要改的参数）。第一次落地请带 `reset=true`，
   否则上一条音色独有的参数会残留。
3. **先离线试听，再写工程**：
   ```
   preview_synth({ "params": {...}, "note": 60, "duration_ms": 900, "reset": true })
   ```
   返回 `timbre`（频谱重心 / 平坦度 / 波峰因数 / 分段能量）、`peak_dbfs` / `rms_dbfs` / `integrated_lufs`、
   以及**生效后的完整 params**。这一步不写工程、不进撤销历史，也**不含效果链**——听到的是乐器干声。
4. **一次只改一个维度**，再 preview，比较数字（见「怎么读指标」）。
5. **满意后落地**：
   ```
   set_params({ "track": 3, "params": {...}, "reset": true })
   ```
   `set_params` 是**一个撤销步**；不要循环 `set_param`，也不要用 `apply_preset`（它会把效果链整体替换/清空）。
6. **要宽度/空间再挂效果器**：`builtin.chorus` / `builtin.reverb` / `builtin.delay`，别指望合成器自己出立体声。
7. **满意后问用户要不要存进音色库**：只有用户明确要求或确认后，才
   `save_sound_memory(track, name, description, tags)`；默认绝不自动保存。
   下次做同类音色先 `list_sound_memories(query=...)`。

> 用 `track` 参数可以让 preview 从**轨道当前音色**出发（`{"track": 3, "params": {"cutoff": 0.55}}`），
> 适合「把现在这个音色调暗一点」。要从干净默认开始就加 `"reset": true`。

**界面上的 36 条出厂音色**（8-bit / 电钢 / 吉他 / 贝斯 / 鼓 / 弦乐 / 管乐 / 铺底 / 音效）是同一台
合成器的参数补丁。三种用法：

- `list_synth_presets(category?, query?)` → 取 id / 中文名 / 描述 / **完整参数补丁**。
- 用户已经在界面选了一条 → `get_project` 读 `track.instrument.params`，那是完整参数，可以在此基础上继续捏。
- 从零开始 → 用下面的「配方」，或按风格自己组合；`describe_plugin("builtin.synth")` 的
  `params` 是参数表（id / 范围 / 默认值），不是出厂音色列表。

## 参数语义（不是猜的，按 DSP 映射）

| 参数 | 范围 | 实际含义 / 听感 |
| --- | --- | --- |
| `wave` | 0..3 | 0 锯齿（谐波最全）、1 方波/脉冲、2 正弦（干净）、3 三角（柔和）。步长 1。 |
| `pwm` | 0..1 | 只对 `wave=1` 有效：占空比 = `0.5 + (pwm-0.5)*0.9`，即 5%..95%。0.5 = 标准方波。 |
| `sub` | 0..1 | **低八度方波**（不是正弦）直接加到主振上；越大越厚、也越容易低频过载。 |
| `detune` | 0..1 | 失谐量：满值 = ±2%（约 ±34 音分）。`unison = 1` 时沿用老行为（主振 + 一个 +2% 副本，各 0.5）；`unison > 1` 时作为多副本的展开量。 |
| `unison` | 1..4 | 每声部的振荡器副本数。1 = 老行为；2..4 = 对称展开的多副本叠加，单声部更厚。不是 7 声部 supersaw；副本越多 CPU 越高。 |
| `spread` | 0..1 | 声部分布：0 = 单声道；>0 时 0 号声部仍居中，和弦的其余声部左右交替展开。单音不会因此变宽。 |
| `noise` | 0..1 | 与振荡器**交叉淡化**：1 = 纯白噪声。在滤波之前。 |
| `fm` | 0..1 | 相位调制深度，0..4 弧度；`fmRatio` 0.5..8 是调制器/载波频率比。没有调制器包络——全程常开。 |
| `pitchEnv` | 0..1 | 起音音高偏移，最多 **+24 半音**。方向由 `pitchEnvDir` 决定：0 = 从 +amount 下扫到 0，1 = 从 0 上扫到 +amount。 |
| `pitchDecay` | 0..1 | 音高扫描时长：`0.005 × 400^t` 秒 ≈ 5 ms..2 s。 |
| `pitchEnvDir` | 0/1 | 0 = 下扫（老行为），1 = 上扫。做 riser / 上滑音时用 1。 |
| `cutoff` | 0..1 | 基础截止：`20 × 900^cutoff` Hz。0.5≈600 Hz，0.7≈2.3 kHz，0.85≈6.5 kHz，1.0≈18 kHz（≈旁通）。**+0.1 ≈ +1 个八度**。 |
| `resonance` | 0..1 | 滤波器阻尼 `k = 2 - 1.9×res`；0.6 以上开始有明显峰值，1.0 接近自激。 |
| `filterEnv` | 0..1 | 滤波包络量，最多 **+7 个八度**。默认乘幅度包络；`filterEnvMode=1` 时乘独立的滤波包络。 |
| `keyFollow` | 0..1 | 键跟随：以 C4(60) 为 0，每高一个八度截止 +1 个八度 × keyFollow。1.0 = 完全跟随音高。 |
| `hpf` | 0..1 | 一阶高通：`20 × 2^(7×hpf)` Hz ≈ 20 Hz..2.5 kHz。去闷、去直流。 |
| `filterType` | 0..2 | 0 低通 / 1 带通 / 2 高通（2 极点 SVF）。 |
| `filterEnvMode` | 0/1 | 0 = 滤波包络跟随幅度 ADSR（老行为）；1 = 使用独立的 fADSR，可以「滤波快关、音量长延音」。 |
| `fAttack` / `fDecay` / `fSustain` / `fRelease` | 同 ADSR | 独立滤波包络的四段；默认值与幅度包络相同，所以 `filterEnvMode=1` 且不改它们时听感接近老行为。 |
| `attack` | 0..1 s | 线性起音。0 = 立即。拨弦/鼓用 0.001–0.01。 |
| `decay` | 0..2 s | 线性衰减到 sustain。 |
| `sustain` | 0..1 | 按住时的电平。0 = 打击型（衰减完就静音）。 |
| `release` | 0..4 s | 松键后的线性释音。 |
| `lfoRate` | 0..1 | 三角 LFO：`0.05 × 400^r` Hz ≈ 0.05..20 Hz。0.5≈2 Hz，0.75≈12.6 Hz。 |
| `lfoDelay` | 0..1 | LFO 淡入延迟 0..2 s（每次按键重新计时；全局模式相位不重置）。 |
| `lfoPitch` | 0..1 | 颤音深度，最多 **±1 半音**。 |
| `lfoFilter` | 0..1 | 滤波摆动，最多 **±2 个八度**。 |
| `lfoMode` | 0/1 | 0 = 全局 LFO（老行为）；1 = 每声部独立相位/延迟，和弦里每个音可以各自揉弦。 |
| `crush` | 0..1 | 位深量化：16 bit → 4 bit（拉满 16 档）。放在滤波之后，硬边不会被抹平。 |
| `gain` | 0..1 | 最终电平（默认 0.8）。输出再做 ±1 硬限幅。 |
| `oversample` | 0/1 | 0 = 关闭；1 = 2x 过采样（内部 2 倍采样 + 半带抽取）。高 FM / 高音区建议开；CPU 约翻倍。 |

### 必须知道的交互与限制

- **滤波包络默认跟随幅度包络**（老行为）。要「滤波快速衰减 + 音量长延音」的经典 pluck，
  开 `filterEnvMode = 1`，再把 `fAttack/fDecay/fSustain/fRelease` 设短；幅度用 `attack` 等单独设。
- **velocity 只影响音量**，不映射到滤波/包络。想要「力度越亮」只能靠自动化或换音色。
- **音高包络方向可切**：`pitchEnvDir = 0` 下扫（老行为），`1` 上扫。上扫从 0 开始，不会先跳一下。
- **每声部 LFO 可选**：`lfoMode = 1`。默认仍是全局 LFO（老行为），`lfoDelay` 在全局模式只重置淡入计时；
  每声部模式从三角波中点起振，不会把起音音高拉偏。
- **unison 是单声部的多副本叠加**（1..4），不是 7 声部 supersaw，也不会把副本分别摆到左右；
  单音宽度仍要 `builtin.chorus` / 声像 / 发送。`spread` 只展开和弦的声部，单音保持居中。
- **2x 过采样是可选**（`oversample = 1`），不是默认。高 `fm` + 高 `fmRatio` + 高音区建议开；
  不开时会有混叠，听感像「沙沙的毛刺」，不是亮度。
- **没有调制矩阵 / 振荡器同步 / 环形调制 / 波表 / 粒子 / 物理建模**。FM 可以近似金属感与铃声。
- **16 复音，满了会抢 0 号声部**（没有平滑抢占）；密集和弦要控制同时发声数。
- **参数是归一值**，不是 Hz/秒；用上面的映射换算，不要直接传 440。

## 怎么读 `preview_synth` 的指标

| 指标 | 低 | 中 | 高 | 怎么调 |
| --- | --- | --- | --- | --- |
| `spectral_centroid_hz` | <300 暗/闷/低 | 300–1500 温暖/正常 | >3000 亮/薄/噪 | 调 `cutoff`（±0.05 一档）、`hpf`、`filterEnv` |
| `spectral_flatness` | <0.05 乐音 | 0.05–0.2 气声/弓弦 | >0.4 噪声主导 | 调 `noise`、`cutoff`、`filterType` |
| `crest_factor_db` | <8 持续/压扁 | 10–18 有动态 | >20 很尖/很敲 | 调 `attack` / `decay` / `filterEnv` / `pitchEnv` |
| `bands.sub_db` | 相对其它段 ≥ -6 dB 才算有超低 | | | `sub`、`pitchEnv`、`hpf`、`cutoff` |
| `bands.air_db` | 与 `presence_db` 差 10 dB 内 = 有空气感 | | | `noise`、`cutoff`、`filterType` |
| `clipped` | true 说明削波 | | | 降 `gain` / `sub` / `noise`，或降 `filterEnv` |

**调整规则：一次只动一个参数/一个维度，再 preview 对比。** 例如：
「太闷」→ `cutoff +0.05`；「太刺」→ `cutoff -0.05` 或 `resonance -0.1`；
「太扁」→ `detune +0.15` 或 `lfoFilter +0.1`；「不够冲击」→ `attack 0.001`、`decay` 缩短、`filterEnv +0.1`。

## 配方（补丁）

每条都假设 `reset=true`；只写需要偏离默认值的参数。先 `preview_synth({"reset":true,"params":{...}})`，
再把同一份 `params` 喂给 `set_params({track, reset:true, params:{...}})`。

### 808 底鼓（正弦 + 快速下扫）
```json
{"wave":2,"gain":0.85,"attack":0.0,"decay":0.4,"sustain":0.0,"release":0.05,
 "cutoff":1.0,"filterEnv":0.0,"keyFollow":0.0,"pitchEnv":0.35,"pitchDecay":0.3}
```
- 音高写 36 附近；`pitchEnv` 每 0.1 ≈ 2.4 半音（最多 24）。
- 想要更「808」：`decay 0.6–0.9`、`sub 0.2`；想要更「techno」：`pitchDecay 0.12`、`decay 0.22`。

### 军鼓
```json
{"wave":3,"noise":0.65,"gain":0.8,"attack":0.0,"decay":0.16,"sustain":0.0,"release":0.06,
 "cutoff":0.75,"resonance":0.15,"filterEnv":0.2,"hpf":0.2,"pitchEnv":0.2,"pitchDecay":0.12}
```
- 写法：两个音高（如 50 与 55）各写一轨/一音，或同轨快速两个音，能更厚。

### 闭镲 / 开镲
```json
{"wave":0,"noise":1.0,"gain":0.45,"attack":0.0,"decay":0.045,"sustain":0.0,"release":0.02,
 "cutoff":0.9,"filterType":2,"hpf":1.0}
```
- 开镲把 `decay` 拉到 0.25–0.5、`release 0.1`。`filterType=2` + 高 `hpf` 是「薄而亮」的关键。

### Sub 贝斯（干净、可控）
```json
{"wave":2,"sub":0.25,"gain":0.75,"attack":0.005,"decay":0.6,"sustain":0.8,"release":0.12,
 "cutoff":0.42,"resonance":0.0,"filterEnv":0.05,"keyFollow":0.2,"hpf":0.0}
```
- 音域 28–48；不要叠加和弦。想要一点「齿」：`wave=0`、`cutoff 0.5`、`sub 0.15`。

### Acid 贝斯（303 味道）
```json
{"wave":0,"gain":0.7,"attack":0.002,"decay":0.22,"sustain":0.15,"release":0.06,
 "cutoff":0.38,"resonance":0.72,"filterEnv":0.5,"keyFollow":0.25,"glide":0.06}
```
- 单声部写法：音符**不重叠**，或在同音高之间用 `legato`；滑音才会明显。
- 想要「咬」：`resonance 0.8`、`filterEnv 0.6`；想要「闷」：`cutoff 0.3`、`filterEnv 0.35`。

### Reese 贝斯（宽、有拍频）
```json
{"wave":0,"detune":0.55,"unison":3,"sub":0.2,"gain":0.65,"attack":0.01,"decay":0.8,
 "sustain":0.75,"release":0.2,"cutoff":0.45,"resonance":0.2,"filterEnv":0.2,"keyFollow":0.2}
```
- `unison=3` 让单音本身更厚；真正宽度要加 `builtin.chorus`（rate 0.3 / depth 0.4 / mix 0.3）
  或两条不同声像的轨道。`spread` 只展开和弦声部，对单音无效。

### 拨弦 / Pluck（独立滤波包络）
```json
{"wave":0,"gain":0.68,"attack":0.01,"decay":0.9,"sustain":0.25,"release":0.25,
 "cutoff":0.42,"resonance":0.18,"filterEnv":0.65,"keyFollow":0.35,"noise":0.05,
 "filterEnvMode":1,"fAttack":0.001,"fDecay":0.18,"fSustain":0.0,"fRelease":0.12}
```
- 这是老版本做不到的经典 pluck：滤波在 180 ms 内关掉，音量却还能延续。
- 更木质：`wave=3`、`cutoff 0.5`；更「电」：`detune 0.15`、`lfoFilter 0.1`。
- 只想要极短拨弦：把 `sustain` 也设成 0。

### 暖垫 / Pad
```json
{"wave":0,"detune":0.35,"unison":3,"spread":0.45,"gain":0.55,"attack":0.45,"decay":2.0,
 "sustain":0.8,"release":1.2,"cutoff":0.58,"resonance":0.1,"filterEnv":0.12,"keyFollow":0.25,
 "lfoRate":0.22,"lfoFilter":0.15}
```
- 更空灵：`wave=3`、`cutoff 0.65`、`noise 0.03`；更暗：`cutoff 0.45`、`hpf 0.1`。

### 弦乐合奏
```json
{"wave":0,"detune":0.35,"unison":3,"spread":0.5,"gain":0.55,"attack":0.18,"decay":1.5,
 "sustain":0.85,"release":0.45,"cutoff":0.72,"resonance":0.08,"filterEnv":0.1,"keyFollow":0.3,
 "lfoRate":0.8,"lfoDelay":0.42,"lfoPitch":0.08,"lfoMode":1}
```
- `lfoMode=1` 让和弦里每个音独立揉弦，群感更自然。
- 要真实弦乐优先用采样乐器 `builtin.strings` / `builtin.slow_strings`；这个配方只是「合成弦乐」的兜底。

### 铜管（那记「嗷」）
```json
{"wave":0,"gain":0.55,"attack":0.08,"decay":0.7,"sustain":0.8,"release":0.18,
 "cutoff":0.45,"resonance":0.15,"filterEnv":0.55,"keyFollow":0.3,
 "filterEnvMode":1,"fAttack":0.09,"fDecay":0.5,"fSustain":0.55,"fRelease":0.15}
```
- 上行滤波扫是铜管的关键：独立的 `fAttack` 稍慢、`filterEnv` 大、`cutoff` 低。
- 想要更亮：`cutoff 0.52`；更厚：`detune 0.2`、`sub 0.1`、`unison 2`。

### FM 钟 / 金属
```json
{"wave":2,"fm":0.5,"fmRatio":3.5,"gain":0.55,"attack":0.001,"decay":1.6,"sustain":0.0,
 "release":0.8,"cutoff":0.9,"filterEnv":0.0,"keyFollow":0.2,"oversample":1}
```
- 非整数比（1.41 / 2.76 / 3.5）更像钟；整数比（2 / 3 / 4）更像木琴/电钢。
- 高音区容易混叠：开 `oversample=1`，或把 `fm` 降到 0.6 以下、`cutoff` 收到 0.8。

### FM 电钢
```json
{"wave":2,"fm":0.28,"fmRatio":1.0,"gain":0.7,"attack":0.003,"decay":1.5,"sustain":0.25,
 "release":0.35,"cutoff":0.88,"keyFollow":0.3}
```
- 音头金属味：再叠一点 `fmRatio 14` 的版本（两轨，一个 1:1、一个 14:1）；本合成器没有调制器包络，
  所以 14:1 那条要单独写成短音符。

### 8-bit 主音
```json
{"wave":1,"pwm":0.222,"hpf":0.15,"gain":0.45,"attack":0.002,"decay":0.25,"sustain":0.6,
 "release":0.08,"cutoff":1.0,"resonance":0.0,"filterEnv":0.0,"keyFollow":0.0,
 "lfoRate":0.8,"lfoDelay":0.35,"lfoPitch":0.16}
```
- NES 的四个占空比：`pwm` = 0.083 / 0.222 / 0.5 / 0.778（对应 12.5 / 25 / 50 / 75%）。
- 硬件没有滤波器：`cutoff=1`、`resonance=0`、`filterEnv=0`。

### 上升音 / Riser
```json
{"wave":0,"noise":0.2,"gain":0.5,"attack":0.05,"decay":2.0,"sustain":0.85,"release":0.6,
 "cutoff":0.18,"resonance":0.4,"filterEnv":0.65,"keyFollow":0.0,
 "pitchEnv":1.0,"pitchDecay":0.7,"pitchEnvDir":1,
 "filterEnvMode":1,"fAttack":0.8,"fDecay":1.0,"fSustain":0.9,"fRelease":0.3}
```
- `pitchEnvDir=1` 让音高从 0 上扫 +24 半音；`filterEnvMode=1` + 长 `fAttack` 让滤波也慢慢打开。
- 想更「噪声」：`noise 0.4`、`wave=1`；想更「音乐」：`pitchEnv 0.5`（+12 半音）。

### Wobble / 噪扫
```json
{"wave":0,"noise":0.35,"gain":0.5,"attack":0.01,"decay":0.8,"sustain":0.7,"release":0.2,
 "cutoff":0.42,"resonance":0.6,"filterEnv":0.3,"keyFollow":0.2,
 "lfoRate":0.52,"lfoFilter":0.75,"lfoMode":1}
```
- LFO 最高 20 Hz，不能锁 BPM；要跟拍就用 `set_automation` 给 `inst:cutoff` 画点，
  或在音符上写 `expr`（颤音）配合。`lfoMode=1` 让和弦里的每个音各自摆动。

## 常见坑

- **换音色不 reset**：上一条的 `noise` / `fm` / `detune` 会残留，听起来像「两个音色叠在一起」。
  `set_params` 带 `reset: true`，或 `preview_synth` 也带。
- **拿 `apply_preset` 调音色**：它的 `effects` 是**整体替换**，不传就清空整条效果链。只调音色用 `set_params`。
- **`cutoff` 当 Hz 用**：它是归一值，0.7 ≈ 2.3 kHz，不是 700 Hz。
- **`sub` 当正弦**：它是低八度**方波**，加多了会有低频谐波，不是纯 sub。
- **`lfoPitch` 当半音**：最大只有 ±1 半音（100 音分），颤音够用，做不了大滑音。
- **开了 `filterEnvMode=1` 却只改 `fDecay`**：还要把 `filterEnv`（包络量）设大，滤波才听得出来。
- **`unison=1` + `detune>0` 仍是老行为**（主振 + 一个 +2% 副本）；想要真正的多副本要显式设 `unison=2..4`。
- **`spread` 对单音无效**：它按声部槽位交替展开，0 号声部永远居中；单音宽度用 `builtin.chorus`。
- **密集和弦 + 长 release**：16 复音会被抢，尾音会断；减少同时发声或缩短 release。
- **高 FM + 高音**：混叠会变「沙」，不是「亮」。开 `oversample=1`，或降 `fm` / `fmRatio` / `cutoff`。

## 做不到的事（用 DAW 的其他能力补）

| 想要 | 这个合成器 | 替代做法 |
| --- | --- | --- |
| 自由调制矩阵 / LFO 波形选择 / 速度同步 | 没有 | 自动化 `inst:*`，或外部 VST3 |
| 力度→亮度 | 没有 | 自动化，或拆成两轨（轻/重） |
| 真正的 7 声部 supersaw / 每副本独立声像 | 部分：`unison 1..4` + `spread`（spread 只管和弦声部） | `builtin.chorus` + 多轨声像 |
| 波表 / 粒子 / 物理建模 | 没有 | 换内置采样乐器或外部 VST3 |
| 音色内建混响/延迟 | 没有 | 效果链：`builtin.reverb` / `builtin.delay` |
