# 乐谱导出 / 窗格 / 导入 — 设计

> **状态（2026-10-29）**：
> - 第一阶段：量化 + MusicXML + 导出入口（完成）。
> - 第二阶段：下窗口乐谱窗格（OSMD）+ 播放跟随 + 总谱 / 分谱（完成）。
> - 第三阶段：导入对话框（音频 / MIDI / MusicXML）+ MIDI、MusicXML 解析器（完成）。

## 1 背景

工程模型里只有「拍」为单位的浮点音符（`Note.start/duration`），没有任何符号化表示，
所以「看谱 / 交稿 / 给乐手分谱」这条路是完全断的。ROADMAP 早先把「记谱」列为非目标，
现在补上第一块：**把工程导成 MusicXML**。

选 MusicXML 而不是直接接某个渲染器，是因为 OSMD / Verovio / alphaTab / MuseScore **全都吃**
这个格式。先把这一层写对，导出的文件丢进 MuseScore 就能当场判断谱面质量，再决定用哪个渲染器
都不会返工；反过来先接渲染器，等于把「量化 / 声部 / 连音线」和「渲染器的排版假设」绑在一起调。

## 2 目标与非目标

**目标**

- `Project` → 合法 MusicXML 4.0（partwise）：多轨总谱、变拍号、变速度、歌词、和弦标记、段落。
- 记谱质量够「能看、能交」：量化到可记谱时值，跨小节拆连音线，同轨重叠分声部，每小节每声部补满休止。
- 导出入口走既有的「导出」对话框，不新增顶栏图标。

**非目标（本阶段）**

- 不做应用内谱面渲染（下一阶段接 OSMD）、不做播放光标。
- 不做谱面编辑（点音符改音高 → 回写 MIDI）。
- 不导出 MIDI 文件；不做 MusicXML **导入**。
- 不为鼓组做可编辑的鼓映射表（先用 GM 常用映射 + 兜底循环）。

## 3 方案

### 3.1 分两层，渲染器无关

- `src/score/notation.ts`：纯数据。小节网格（`buildMeasures`）、量化（`quantizeBeat`）、
  时值分解（`decompose`，含附点与三连音）、声部分配、休止补齐。
- `src/score/musicxml.ts`：只做序列化（音名拼写、谱号、和弦解析、鼓映射、XML 转义）。

上层只依赖 `buildMusicXML(project, opts) → { xml, parts, measures, notes, warnings }`，
`tracks` 传全部 = 总谱，传一个 = 分谱。

### 3.2 量化的取舍

- 起点 / 终点**各自**吸附到网格，再相减得时值——相邻音符共享边界，不会因为先量时值再累加而漂移。
- 网格同时认二连与三连（`grid * 2/3`），但三连要「明显更近」才采用，免得手弹的 1/16 被吸到 1/24。
- 时值按「节奏家族」贪婪分解：整拍进二连，1/3 的倍数进三连，于是 4/3 拍会记成**一个三连二分**，
  而不是「四分 + 三连八分」这种看不出三连的组合。
- 跨小节的长音在小节线处切开，逐段用 `<tie>` 接起来。
- `divisions = 960`：三 / 五 / 六连音的 tick 都是整数；七连音需要 3360。

### 3.3 声部与休止

同起点的音（和弦）一定同声部；重叠才另开一个（最多 4 个）。分完声部后，**每小节每个声部
必须填满**——这是谱面排版最容易被忽略的一条：少一个休止符，后面的音会整体左移。

### 3.4 与工程模型的对应

| 工程字段 | MusicXML |
| --- | --- |
| `note.start/duration` | `<note>` 的位置与时值（`offset` 不参与：记谱位置 ≠ 发声位置） |
| `note.lyric`（`-` = 连音） | `<lyric>` + `<extend/>`；声调数字 `1..5` 剥掉 |
| `track.meta.role === "drums"` | 打击乐谱号 + `<unpitched>` |
| `track.meta.pitch_range` | 谱号选择（中位 < A3 → 低音谱号） |
| `transport.keyframes` | `<time>` / `<sound tempo>`（关键点不在小节线上时截成残小节） |
| `project.harmony` | `<harmony>`（解析失败退化成 `<words>`，并进 warnings） |
| `project.sections` | `<rehearsal>` |

工程里**没有调号**，所以导出对话框让用户手选（默认 C）——选对调号，升降号能少一大半。

## 4 验收标准

1. `npx vue-tsc --noEmit` 与 `npx vite build` 通过。
2. `cargo check --manifest-path src-tauri/Cargo.toml` 通过（新增 `export_score` 命令）。
3. `tools/score-selftest.ts` 全绿，覆盖：多轨、变拍号、变速度、跨小节连音线、三连音、
   第二声部（`<backup>`）、歌词连音、鼓组、和弦解析失败、分谱、空工程。
4. 自检里的不变量：每小节各声部时值相等、每小节时值 = 拍号长度、`<note>` 子元素顺序符合 DTD。
5. 导出的 `.musicxml` 能被 MuseScore 打开且排版合理。

## 5 验证方式

```
npx vue-tsc --noEmit -p tsconfig.json
npx vite build
cargo check --manifest-path src-tauri/Cargo.toml
npm run selftest:score
npm run selftest:import
```

自检产物：`tools/out/score-full.musicxml`、`tools/out/score-part.musicxml`。

## 6 风险

| 风险 | 缓解 |
| --- | --- |
| 量化后的谱面与听到的不一致（swing / humanize / 扫弦） | 「记谱量化」独立于播放；对话框可关量化；说明文案写清楚这是谱面近似 |
| 变拍号落在小节中间 | 截成残小节（MusicXML 允许），时间轴不错位 |
| 鼓组映射不准 | 先覆盖 GM 常用音，其余按五位置循环；后续做可编辑鼓映射表 |
| 和弦标记解析失败 | 退化成 `<words>` 文字标记，不丢用户数据，并进 warnings |
| 大工程 XML 体积 | 只影响文件大小，不影响渲染（渲染阶段再分页） |
| `pnpm install` 因 `gl`（OSMD 的可选依赖）的构建脚本直接失败 | `pnpm-workspace.yaml` 里 `allowBuilds: gl: false`：那是它 Node 端无头渲染用的，WebView 里不碰 |

## 7 乐谱窗格（下窗口）与乐谱视图

谱面是「边听边看」的东西，和控制器泳道是同一个位置诉求，所以**不新开视图**，而是和下窗口共用
一个「目标」下拉：

- `LaneTargetSelect`（新）把下拉选项抽出：**乐谱** / 力度 / 压力 / 通道 / 自动化目标。
  「乐谱」放在最前面、单独归到「视图」组——它是切换窗格内容，不是参数。
  `ControllerLane` 与 `ScorePane` 共用它，否则选了「乐谱」就再也切不回来。
- `EditView` 按 `controllerTargetKey === "score"` 二选一渲染 `ScorePane` / `ControllerLane`；
  两者共用 `layout.showControllerLane` 与同一个 `PanelResizer`，表头 30px + 1px 边框逐像素对齐，
  切换时下窗口不跳。
- `ScorePane` 只画**当前片段**（`selection.clip`），和上面的钢琴卷帘是同一条 MIDI：
  `range = { start: clip.start, end: clip.start + clip.length }`。片段起点不一定压在小节线上，
  所以网格从**包含它的那一小节**起排（`sliceMeasures`），首小节用休止符补齐。
- `ScoreView`（`/score`，顶栏夹在编辑与混音之间）画整首或单条轨道，像 Guitar Pro 那样通读。

`ScoreCanvas`（新）是两者共用的画布：`tracks` / `range` 决定画什么，渲染、光标、滚动逻辑
只有一份。它按需加载（`defineAsyncComponent` + 路由懒加载）：OSMD 的包 1.4 MB，不进首包。

**下窗口的三条硬约束**（都是用户实测后提的）：

1. 从「力度」切到「乐谱」时**高度不变**——早期版本会自动把窗格撑到 320px，切换时会跳，删掉了；
2. 下拉按钮的尺寸样式**只写在 `LaneTargetSelect` 里**（`.md3-select` 120×22、11px），
   两个宿主因此逐像素一致；
3. 目标写在 `layout.controllerTarget`（持久化），切到别的视图再回来仍然是乐谱。

**主题**：`useTheme()` 暴露 `isDark`。谱面**不画底色**（`pageBackgroundColor` 留空，默认透明），
让容器的 `--app-bg` 透出来——这样和 App 其他页面完全同色，不会出现一块白纸或黑砖。
音符 / 歌词 / 标题的颜色用 `defaultColorMusic` 等按主题给。这些都是构造参数，所以主题一变就
整块重建实例（顺便清掉播放指示）。

**播放指示（GTP 那种）**：当前小节淡底（`.bar-highlight`）+ 当前拍色带（`.beat-band`）+
播放头竖线。三者都从同一份小节包围盒算出来，播放头不在当前这一段里就整体隐藏。
**还没做**：逐个音符头的点亮——那需要拿到 OSMD 内部 `GraphicalNote` 的位置，等光标在真机上
校准之后再补，避免把没验证过的坐标写死进去。

## 7.5 钢琴卷帘的「可见范围」

左上角原来的网格标签（1/16）换成了可见范围下拉（`layout.rollScope`，持久化）：

- **片段**：只看当前片段（默认，行为同以前）；
- **轨道**：当前轨道所有片段，当前片段之外的画成 ghost；
- **全部**：所有轨道，当前轨道之外的画成 ghost。

ghost 是只读参考层：`pointer-events: none`、28% 不透明度、按轨道色相着色，按视口横向裁剪
（`timeline.viewportWidth`）并封顶 3000 个。网格选择仍在控制器泳道表头——用户明确说过那里
已经显示了，不需要再占一格。

**播放光标不用 OSMD 自带的 cursor**：它按「谱面事件」步进，和宿主的 transport 对不上
（seek / loop 会错位）。改为自己算：`buildMusicXML` 额外返回 `barStarts` / `barLengths`，
把连续的拍换算成「第几小节、小节内第几拍」，再用渲染出来的小节包围盒（`osmd.Graphic` →
`MusicPages → MusicSystems → StaffLines[0].Measures`）插值出 x 坐标，画一条自己的竖线。
播放头不在当前这一段里就不画；坐标取不到（NaN / 数量对不上）也不画——宁可不显示，不能画错。

## 8 谱式（五线谱 / 吉他谱 / 贝斯谱 / 鼓谱）

| 谱式 | 实现 |
| --- | --- |
| 五线谱 | OSMD（MusicXML） |
| 鼓谱 | OSMD + `asDrums`：强制打击乐谱号 + `<unpitched>`，不看 `meta.role` |
| 吉他谱 / 贝斯谱 | **自绘** `TabCanvas`（OSMD 不支持六线谱） |

`TabCanvas` 直接读工程音符 + `buildMeasures`，不经过 MusicXML——进度指示、主题、横向滚动都在
我们自己手里。调弦固定为标准调弦（吉他 EADGBE、贝斯 EADG），音高按「品数最小、同品选粗弦」
分配；谱面是一条横向长轴（和钢琴卷帘同一套心智，不分页不换行）。

谱式是「一条轨道」的属性：在总谱下选六线谱 / 鼓谱会自动切到第一条有音符的轨道；切回总谱时
谱式落回五线谱。

## 8.5 导入（音频 / MIDI / MusicXML）

导入按钮从「直接弹文件框」改成先开模态框选来源——同一个过滤器里混着 wav 和 mid，用户不知道
点开以后会发生什么。三种来源的落地方式不同，卡片上写清楚：

| 来源 | 行为 |
| --- | --- |
| 音频 | 插到当前轨道的播放头处，**不动**现有内容（原有行为） |
| MIDI | 解析成工程 → **整体替换**当前工程 |
| MusicXML | 同上（`.mxl` 压缩包暂不支持，明确拒绝而不是当文本硬解） |

解析放在前端（`src/import`），后端只加两条命令：

- `read_import_file(path) -> Response`：回原始字节（和 `render_preview` 一样走二进制，
  不按 JSON 数组传一兆的谱面）。
- `import_project(project) -> Project`：先把当前工程收进历史，再 `Engine::restore` 整体替换，
  重载音频图、关掉插件窗口、**清空会话路径**（导入的是新内容，保存时该重新选位置）。
  撤销栈会重建，但旧工程在「历史」里，可找回。

MIDI：手写 SMF 解析（`src/import/midi.ts`）——`MThd`/`MTrk`、running status、变长 delta、
note on/off 配对（同音高重叠按栈）、tempo(0x51)、拍号(0x58)、轨道名(0x03)、program。
`beat = ticks / division`。format 0 的多通道按通道拆成多条轨。SMPTE 时间码直接拒绝。

MusicXML：先用自带的极简 XML 解析器（`src/import/xml.ts`，不引依赖、能在 Node 里跑自检），
再按 part / measure / attributes / note / backup 走一遍；连音线按 `tie` 合并回一个音，
`<unpitched>` 走鼓位置反查表。反复记号、装饰音、跨谱连线这些**明确不支持**，跳过并记 warning。

## 9 后续

1. 调号推断（从 `analyze_harmony` 的结果猜），减少导出时的手选。
2. 可编辑谱面（点音符 → 回写 MIDI）；需要先把「谱面位置 ↔ 拍」的映射存进事件里。
3. `.mxl` 解压导入、MIDI 的 CC / 弯音曲线导入（要先把逐音表达补进音符模型）。
4. 导入时选择「替换 / 合并到当前工程」——合并需要处理速度图与轨道 id 重排。
