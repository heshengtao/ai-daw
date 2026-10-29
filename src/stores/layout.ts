/**
 * 甯冨眬鍋忓ソ涓庤交閲忛€氱煡锛坰nackbar锛夛細闈㈡澘鍙鎬т笌瀹藉害銆佸彸渚ч潰鏉跨殑鏍囩椤碉紝鎸佷箙鍖栧埌 localStorage銆?
 */
import { defineStore } from "pinia";
import { ref, watch } from "vue";
import type { Notation } from "@/utils/notation";

const KEY = "ai-daw-layout";

/** 鍙充晶闈㈡澘鍙墦寮€鐨勭獥鍙ｇ被鍨嬶紱鍚庣画鏂板闈㈡澘鍙渶鍦ㄦ鐧昏銆?*/
export type PaneId =
  | "inspector"
  | "mixer"
  | "plugins"
  | "manager"
  | "sounds"
  | "editor"
  | "agent"
  | "ai"
  | "ops";

/**
 * 榧犳爣妯″紡锛堥《鏍忎笁閫変竴锛夛細
 * - `draw`锛氬湪绌虹櫧澶勫乏閿嫋鍔?= 鐢诲嚭鐗囨 / 闊崇锛堝垱寤?/ 鎷栧姩锛夛紱
 * - `select`锛氬乏閿嫋鍔?= 妗嗛€夛紱
 * - `split`锛氬乏閿偣鍑荤墖娈?= 鍦ㄨ澶勫垏寮€锛坮azor锛夈€?
 *
 * 涓夎€呭叡鐢ㄥ悓涓€涓墜鍔匡紝鍙兘闈犳ā寮忓尯鍒嗭紝鍥犳蹇呴』鐢辩敤鎴锋樉寮忛€夋嫨銆?
 */
export type Tool = "draw" | "select" | "split" | "lyric";

/** 鍏ㄩ儴榧犳爣妯″紡锛堥『搴忓嵆椤舵爮閲岀殑灞曠ず椤哄簭锛夈€?*/
export const TOOLS: Tool[] = ["draw", "select", "split", "lyric"];

/** 鍏ㄩ儴鍙敤闈㈡澘鐨勫厓鏁版嵁锛堜緵"鏂板缓鏍囩"閫夋嫨鍣ㄥ睍绀猴級銆傛枃妗堟槸 i18n key锛屾覆鏌撴椂缈昏瘧銆?*/
export const PANES: { id: PaneId; labelKey: string; hintKey: string }[] = [
  { id: "inspector", labelKey: "panes.inspector", hintKey: "panes.inspectorHint" },
  { id: "mixer", labelKey: "panes.mixer", hintKey: "panes.mixerHint" },
  { id: "editor", labelKey: "panes.editor", hintKey: "panes.editorHint" },
  { id: "plugins", labelKey: "panes.plugins", hintKey: "panes.pluginsHint" },
  { id: "manager", labelKey: "panes.manager", hintKey: "panes.managerHint" },
  { id: "sounds", labelKey: "panes.sounds", hintKey: "panes.soundsHint" },
  { id: "ops", labelKey: "panes.ops", hintKey: "panes.opsHint" },
  { id: "agent", labelKey: "panes.agent", hintKey: "panes.agentHint" },
  { id: "ai", labelKey: "panes.ai", hintKey: "panes.aiHint" },
];

/**
 * 鍙互**鎵嬪姩**鎵撳紑鐨勯潰鏉匡紙銆?銆嶉€夋嫨鍣ㄤ笌銆屽皻鏈墦寮€浠讳綍闈㈡澘銆嶇┖鎬佸叡鐢級銆?
 *
 * 涓嶅惈銆屾彃浠剁晫闈€嶏細閭ｄ釜闈㈡澘鏄?*璺熺潃鎻掍欢璧?*鐨勶紝鍏ュ彛鍦ㄦ彃浠惰嚜宸辩殑鎸夐挳涓婏紙鎻掍欢娓呭崟椤?/
 * 妫€鏌ュ櫒閲岄偅鏉¤建閬撶殑銆屾彃浠剁晫闈€嶆寜閽級銆傚嚟绌烘墦寮€涓€涓病鏈夌洰鏍囩殑鎻掍欢鐣岄潰鍙細璁╀汉鍥版儜銆?
 */
export const PICKABLE_PANES = PANES.filter((p) => p.id !== "editor");

/** 宸︿晶闊宠建鏍忓搴︿笂闄愶細绐楀彛瀹藉害鐨?1/4銆?*/
export const MAX_TRACK_WIDTH_RATIO = 0.25;
/** 鍙充晶闈㈡澘瀹藉害涓婇檺锛氭墸闄ゅ乏渚у悗锛屽墿浣欏尯鍩熺殑 2/3銆?*/
export const MAX_INSPECTOR_RATIO = 2 / 3;

/** 宸︿晶闊宠建鏍忓搴︿笅闄愶紙px锛夛細鍐嶇獎鍐呴儴鍏冪礌锛堟棆閽€佹寜閽锛夊氨浼氳鍘嬪彉褰€?*/
const MIN_TRACK_WIDTH = 200;
/** 鍙充晶闈㈡澘瀹藉害涓嬮檺锛坧x锛夛細鎻掍欢琛屻€佸弬鏁板瓧娈甸渶瑕佺殑鏈€灏忓懠鍚哥┖闂淬€?*/
const MIN_INSPECTOR_WIDTH = 300;

interface Persisted {
  /** 甯冨眬鐗堟湰锛氭柊澧為潰鏉挎椂鑷锛岀敤鏉ョ粰鏃у竷灞€鍋氫竴娆℃€ц縼绉汇€?*/
  version: number;
  showTrackHeaders: boolean;
  showInspector: boolean;
  trackWidth: number;
  inspectorWidth: number;
  tabs: PaneId[];
  activeTab: PaneId;
  tool: Tool;
  /** 搴曢儴绐楁牸鐨勯珮搴︼紙px锛夛細缂栨帓鐨勮嚜鍔ㄥ寲杞﹂亾涓庣紪杈戠殑鎺у埗鍣ㄦ吵閬?*鍏辩敤**涓€浠姐€?*/
  laneHeight: number;
  /** 缂栨帓瑙嗗浘搴曢儴鐨勮嚜鍔ㄥ寲鏇茬嚎缂栬緫鍣ㄦ槸鍚︽墦寮€銆?*/
  showAutomation: boolean;
  /** 閽㈢惔鍗峰笜涓嬫柟鐨勬帶鍒跺櫒锛堝姏搴?/ 闊抽噺锛夋吵閬撴槸鍚︽墦寮€銆?*/
  showControllerLane: boolean;
  /** 缃戞牸缁嗗垎锛氬嵎甯樼殑鍚搁檮姝ラ暱 + 缂栨帓瑙嗗浘鐨勫叏灞€鍚搁檮锛?*鍚屼竴浠借缃?*銆?*/
  noteGrid: NoteGrid;
  /** 搴曢儴绐楁牸鐨勫綋鍓嶇洰鏍囷紙`velocity` / 鑷姩鍖栧弬鏁伴敭 / `score`锛夈€?*/
  controllerTarget: string;
  /** 閽㈢惔鍗峰笜鐨勫彲瑙佽寖鍥达細褰撳墠鐗囨 / 鏁存潯杞ㄩ亾 / 鍏ㄩ儴杞ㄩ亾銆?*/
  rollScope: RollScope;
  /** 姣忔潯杞ㄩ亾鐨勮氨寮忥紙鏄剧ず鍋忓ソ锛屼笉杩涘伐绋嬫枃浠讹級銆?*/
  trackNotations: Record<string, string>;
  /** 姣忔潯杞ㄩ亾鎵嬪姩鎸囧畾鐨勮氨鍙凤紙`G` / `F` / `C` / `percussion`锛夛紱娌¤褰?= 鎸夐煶鍩熻嚜鍔ㄣ€?*/
  trackClefs: Record<string, string>;
  /** 姣忔潯杞ㄩ亾鎵嬪姩鎸囧畾鐨勮皟寮︼紙MIDI 闊抽珮锛屼綆 鈫?楂橈級锛涙病璁板綍 = 鏍囧噯璋冨鸡銆?*/
  trackTunings: Record<string, number[]>;
}

/**
 * 閽㈢惔鍗峰笜鐨勫彲瑙佽寖鍥淬€?
 *
 * - `clip`锛氬彧鐪嬪綋鍓嶇墖娈碉紙榛樿锛屽拰浠ュ墠涓€鏍凤級锛?
 * - `track`锛氬綋鍓嶈建閬撴墍鏈夌墖娈电殑闊崇锛堝綋鍓嶇墖娈典箣澶栫殑鐢绘垚 ghost锛屽彧璇伙級锛?
 * - `all`锛氭墍鏈夎建閬撶殑闊崇锛堝叾浠栬建閬?ghost锛屽彧璇伙級銆?
 *
 * 鍚庝袱妗ｆ槸銆岀湅涓婁笅鏂囥€嶇敤鐨勶細鍜屽０銆佸浣嶃€佽妭濂忓挰鍚堥兘闈犲瀭鐩存柟鍚戠湅锛屼絾缂栬緫鏉冧粛鐒跺彧鍦ㄥ綋鍓嶇墖娈点€?
 */
/**
 * 閽㈢惔鍗峰笜鐨勫彲瑙佽寖鍥淬€?
 *
 * 鍙湁涓ゆ。锛歚track`锛堝綋鍓嶈建閬擄紝鍏朵粬鐗囨鐢绘垚 ghost锛夊拰 `all`锛堝叏閮ㄨ建閬擄紝鍒殑杞ㄩ亾鐢绘垚 ghost锛夈€?
 * 鏃╁厛杩樻湁 `clip`锛堝彧鐪嬪綋鍓嶇墖娈碉級锛屼絾瀹冨拰銆岃建閬撱€嶇殑宸埆鍙槸灏戠敾鍑犲潡 ghost锛屽涓€涓€夐」鍙嶈€?
 * 瑕佸湪涓ゅ涓嬫媺閲岃В閲娿€岀墖娈靛拰杞ㄩ亾鍒板簳宸湪鍝€嶁€斺€旂洿鎺ュ幓鎺変簡銆?
 */
export type RollScope = "track" | "all";

/** 鑼冨洿閫夐」锛堥『搴忓嵆涓嬫媺閲岀殑椤哄簭锛夈€?*/
export const ROLL_SCOPES: RollScope[] = ["track", "all"];

/**
 * 缃戞牸缁嗗垎锛?*涓や釜瑙嗗浘鍏辩敤**銆?
 *
 * 閽㈢惔鍗峰笜鐢ㄥ畠鍐冲畾鐢?/ 鎷栭煶绗︾殑鍚搁檮姝ラ暱涓庣粏鍒嗙嚎锛涚紪鎺掕鍥剧敤瀹冨喅瀹氭嫋鐗囨銆佹爣灏哄畾浣嶃€佽嚜鍔ㄥ寲鐐?
 * 鐨勫惛闄勬闀裤€傛墍浠ュ湪涓€澶勬崲鎴?1/8锛屽彟涓€澶勭殑鎵嬫劅绔嬪埢璺熺潃鍙樷€斺€旇繖姝ｆ槸銆屽悓涓€涓綉鏍笺€嶈鏈夌殑琛屼负銆?
 */
export type NoteGrid = "1/4" | "1/8" | "1/12" | "1/16" | "1/24" | "1/32" | "free";

/** 缃戞牸閫夐」锛堥『搴忓嵆涓嬫媺閲岀殑椤哄簭锛氫粠绮楀埌缁嗭紝鏈€鍚庢槸鑷敱锛夈€備笁杩為煶绱ц窡鍦ㄥ畠鏇夸唬鐨勪簩鍒嗙綉鏍煎悗闈€?*/
export const NOTE_GRIDS: NoteGrid[] = ["1/4", "1/8", "1/12", "1/16", "1/24", "1/32", "free"];


/**
 * 缃戞牸鍦ㄤ竴銆屾媿銆嶏紙鎷嶅彿閲岀殑閭ｄ竴鎷嶏紝4/4 鍗冲洓鍒嗛煶绗︼級閲岀殑绛夊垎鏁帮細`1/16` 鈫?4 绛夊垎銆?
 *
 * 涓夊垎 / 鍏垎锛堜笁杩為煶锛変笉鏄?2 鐨勫箓锛屾墍浠ョ洿鎺ヨ绛夊垎鏁帮紝鑰屼笉鏄€屽嚑鍒嗕箣鍑犮€嶃€?
 * `free` = 涓嶅惛闄勩€?
 */
const GRID_DIVISIONS: Record<Exclude<NoteGrid, "free">, number> = {
  "1/4": 1,
  "1/8": 2,
  "1/16": 4,
  "1/12": 3,
  "1/24": 6,
  "1/32": 8,
};

/** 璇ョ綉鏍煎湪鏌愩€屾媿銆嶉暱搴︿笅鐨勪竴姝ワ紙鎷嶏級锛涜嚜鐢辨ā寮忚繑鍥?`null`銆?*/
export function gridStepFor(grid: NoteGrid, meterBeat: number): number | null {
  if (grid === "free") return null;
  return meterBeat / GRID_DIVISIONS[grid];
}

/** 鏃х増鏈妸搴曢儴绐楁牸鐨勯珮搴﹀垎鎴愪袱浠藉瓨锛宍automationHeight` 鍙敤浜庝竴娆℃€ц縼绉汇€?*/
type Saved = Partial<Persisted> & { automationHeight?: number };

/** 褰撳墠甯冨眬鐗堟湰锛坴2 = 鏂板銆屾搷浣滆褰曘€嶉潰鏉匡級銆?*/
const LAYOUT_VERSION = 2;

function load(): Saved {
  try {
    return JSON.parse(localStorage.getItem(KEY) ?? "{}") as Saved;
  } catch {
    return {};
  }
}

const VALID_PANES: PaneId[] = PANES.map((p) => p.id);

function sanitizeTabs(value: unknown): PaneId[] {
  if (!Array.isArray(value)) return ["inspector"];
  const tabs = value.filter((v): v is PaneId => VALID_PANES.includes(v as PaneId));
  return tabs.length > 0 ? tabs : ["inspector"];
}

/**
 * 搴曢儴绐楁牸鐨勯珮搴﹁寖鍥达紙px锛夛細涓や釜瑙嗗浘鍏辩敤锛屾墍浠ヨ寖鍥村彇涓よ€呯殑骞堕泦锛堟洸绾跨紪杈戝櫒瑕佹洿楂樹竴浜涳級銆?
 */
const MIN_LANE_HEIGHT = 56;
const MAX_LANE_HEIGHT = 360;

function clampLaneHeight(value: number): number {
  if (!Number.isFinite(value)) return 120;
  return Math.min(MAX_LANE_HEIGHT, Math.max(MIN_LANE_HEIGHT, Math.round(value)));
}

export const useLayoutStore = defineStore("layout", () => {
  const saved = load();
  const showTrackHeaders = ref(saved.showTrackHeaders ?? true);
  const showInspector = ref(saved.showInspector ?? true);
  const trackWidth = ref(saved.trackWidth ?? 220);
  // 榛樿缁欏涓€鐐癸細澶嶆潅鎻掍欢锛堝ぇ閲忓弬鏁?/ 灏嗘潵鐨勫師鐢熺晫闈級闇€瑕佸湴鏂广€?
  const inspectorWidth = ref(saved.inspectorWidth ?? 380);
  const tabs = ref<PaneId[]>(sanitizeTabs(saved.tabs));
  // v2锛氭搷浣滆褰曢潰鏉夸笂绾裤€傛棫甯冨眬涓€娆℃€цˉ涓婅繖涓爣绛撅紱鐢ㄦ埛涔嬪悗鎵嬪姩鍏虫帀灏变笉浼氬啀濉炲洖鏉ャ€?
  if ((saved.version ?? 1) < LAYOUT_VERSION && !tabs.value.includes("ops")) {
    tabs.value.push("ops");
  }
  const activeTab = ref<PaneId>(
    saved.activeTab && tabs.value.includes(saved.activeTab) ? saved.activeTab : tabs.value[0],
  );
  // 榛樿銆岀粯鍒躲€嶏細鍒涘缓鏄渶甯哥敤鐨勫姩浣滐紝妗嗛€?/ 鍒囧垎闅忔椂鍙互鍒囷紙椤舵爮鎸夐挳 / 蹇嵎閿?1 / 2 / 3锛夈€?
  const tool = ref<Tool>(TOOLS.includes(saved.tool as Tool) ? (saved.tool as Tool) : "draw");
  // 涓や釜瑙嗗浘鐨勫簳閮ㄧ獥鏍煎叡鐢ㄤ竴浠介珮搴︼細鍒囬〉闈㈡椂绐楁牸锛堜互鍙婂畠涓婃柟鐨勬诞鍔ㄥ伐鍏峰矝锛変笉浼氳烦銆?
  // 鏃х増鏈妸涓や唤楂樺害鍒嗗紑瀛橈紙`laneHeight` / `automationHeight`锛夛紝杩欓噷鍏煎璇讳竴娆℃棫鍊笺€?
  const laneHeight = ref(clampLaneHeight(saved.laneHeight ?? saved.automationHeight ?? 120));
  // 涓や釜搴曢儴绐楁牸榛樿閮芥墦寮€锛氬畠浠槸鍚勮嚜瑙嗗浘鐨勫父瑙勫伐浣滃尯锛屼笉鏄彲閫夌殑璋冭瘯闈㈡澘銆?
  const showAutomation = ref(saved.showAutomation ?? true);
  const showControllerLane = ref(saved.showControllerLane ?? true);
  // 榛樿 1/16锛氫笌缂栨帓瑙嗗浘姝ゅ墠鐨勫惛闄勪竴鑷达紝涔熸槸鏃嬪緥/浼村鏈€甯哥敤鐨勭粏鍒嗐€?
  const noteGrid = ref<NoteGrid>(
    NOTE_GRIDS.includes(saved.noteGrid as NoteGrid) ? (saved.noteGrid as NoteGrid) : "1/16",
  );
  // 搴曢儴绐楁牸鐨勭洰鏍囷細鍒囧埌涔愯氨銆佸垏璧拌鍥惧啀鍥炴潵锛屽簲璇ヨ繕鏄箰璋憋紙鐢ㄦ埛鐨勯€夋嫨鏄寔涔呯殑锛夈€?
  const controllerTarget = ref(typeof saved.controllerTarget === "string" ? saved.controllerTarget : "velocity");
  // 鍗峰笜鍙鑼冨洿锛氶粯璁ゅ彧鐪嬪綋鍓嶇墖娈点€?
  const rollScope = ref<RollScope>(
    // 鏃х増鏈瓨杩?`clip`锛氬畠宸茬粡涓嶅瓨鍦ㄤ簡锛岃惤鍒伴粯璁ょ殑銆岃建閬撱€嶃€?
    ROLL_SCOPES.includes(saved.rollScope as RollScope) ? (saved.rollScope as RollScope) : "track",
  );
  // 姣忔潯杞ㄩ亾鐢ㄥ摢绉嶈氨寮忥紙鍚変粬璋?/ 璐濇柉璋?/ 榧撹氨 / 浜旂嚎璋憋級銆傛病璁板綍鐨勬寜涔愬櫒鐚溿€?
  const trackNotations = ref<Record<string, Notation>>(
    (saved.trackNotations as Record<string, Notation> | undefined) ?? {},
  );
  const trackClefs = ref<Record<string, string>>((saved.trackClefs as Record<string, string> | undefined) ?? {});
  const trackTunings = ref<Record<string, number[]>>(
    (saved.trackTunings as Record<string, number[]> | undefined) ?? {},
  );
  const devDrawer = ref(false);

  /** 褰撳墠鎻愮ず鏂囨锛堣嚜鍔ㄦ秷澶憋級銆?*/
  const toast = ref<string | null>(null);
  let toastTimer: number | null = null;

  watch(
    [
      showTrackHeaders,
      showInspector,
      trackWidth,
      inspectorWidth,
      tabs,
      activeTab,
      tool,
      laneHeight,
      showAutomation,
      showControllerLane,
      noteGrid,
      controllerTarget,
      rollScope,
      trackNotations,
      trackClefs,
      trackTunings,
    ],
    () => {
      localStorage.setItem(
        KEY,
        JSON.stringify({
          version: LAYOUT_VERSION,
          showTrackHeaders: showTrackHeaders.value,
          showInspector: showInspector.value,
          trackWidth: trackWidth.value,
          inspectorWidth: inspectorWidth.value,
          tabs: tabs.value,
          activeTab: activeTab.value,
          tool: tool.value,
          laneHeight: laneHeight.value,
          showAutomation: showAutomation.value,
          showControllerLane: showControllerLane.value,
          noteGrid: noteGrid.value,
          controllerTarget: controllerTarget.value,
          rollScope: rollScope.value,
          trackNotations: trackNotations.value,
          trackClefs: trackClefs.value,
          trackTunings: trackTunings.value,
        } satisfies Persisted),
      );
    },
    { deep: true },
  );

  /** 鍒囨崲榧犳爣妯″紡銆?*/
  function setTool(next: Tool): void {
    tool.value = next;
  }

  /** 璋冩暣搴曢儴绐楁牸楂樺害锛堜袱涓鍥惧叡鐢紝鎷栨嫿鍒嗛殧绾挎椂璋冪敤锛夈€?*/
  function resizeLane(delta: number): void {
    laneHeight.value = clampLaneHeight(laneHeight.value + delta);
  }

  /** 璁剧疆鏌愭潯杞ㄩ亾鐨勮氨寮忥紙鏄剧ず鍋忓ソ锛屽瓨鍦ㄦ湰鏈猴級銆?*/
  function setTrackNotation(trackId: number, notation: Notation): void {
    trackNotations.value = { ...trackNotations.value, [String(trackId)]: notation };
  }

  /** 鏌愭潯杞ㄩ亾鎵嬪姩璁剧疆杩囩殑璋卞紡锛沗null` = 娌¤杩囷紝鎸変箰鍣ㄧ寽銆?*/
  function trackNotationOf(trackId: number): Notation | null {
    return trackNotations.value[String(trackId)] ?? null;
  }

  /** 璁剧疆璋卞彿锛坄null` = 鎭㈠鑷姩锛夈€?*/
  function setTrackClef(trackId: number, clef: string | null): void {
    const next = { ...trackClefs.value };
    if (clef === null) delete next[String(trackId)];
    else next[String(trackId)] = clef;
    trackClefs.value = next;
  }

  /** 璁剧疆璋冨鸡锛坄null` = 鏍囧噯璋冨鸡锛夈€?*/
  function setTrackTuning(trackId: number, tuning: number[] | null): void {
    const next = { ...trackTunings.value };
    if (tuning === null) delete next[String(trackId)];
    else next[String(trackId)] = [...tuning];
    trackTunings.value = next;
  }

  // ---- 闈㈡澘瀹藉害锛氬乏鍙冲悇鏈夎嚜宸辩殑涓婇檺 ----

  /** 褰撳墠瑙嗗彛瀹藉害銆?*/
  function viewportWidth(): number {
    return typeof window === "undefined" ? 1280 : window.innerWidth;
  }

  /** 宸︿晶闊宠建鏍忕殑瀹藉害涓婇檺锛氱獥鍙ｇ殑 1/4銆?*/
  function maxTrackWidth(): number {
    return Math.max(MIN_TRACK_WIDTH, Math.round(viewportWidth() * MAX_TRACK_WIDTH_RATIO));
  }

  /** 鍙充晶闈㈡澘鐨勫搴︿笂闄愶細鎵ｆ帀宸︿晶鍚庯紝鍓╀綑鍖哄煙鐨?2/3銆?*/
  function maxInspectorWidth(): number {
    const used = showTrackHeaders.value ? trackWidth.value : 0;
    return Math.max(MIN_INSPECTOR_WIDTH, Math.round((viewportWidth() - used) * MAX_INSPECTOR_RATIO));
  }

  function clampTrackWidth(value: number): number {
    if (!Number.isFinite(value)) return 220;
    return Math.min(maxTrackWidth(), Math.max(MIN_TRACK_WIDTH, Math.round(value)));
  }

  function clampInspectorWidth(value: number): number {
    if (!Number.isFinite(value)) return 380;
    return Math.min(maxInspectorWidth(), Math.max(MIN_INSPECTOR_WIDTH, Math.round(value)));
  }

  /** 鎷栨嫿宸︿晶鍒嗛殧绾匡細`delta` 涓烘琛ㄧず鍙樺銆?*/
  function resizeTrack(delta: number): void {
    trackWidth.value = clampTrackWidth(trackWidth.value + delta);
    // 宸︿晶鍙樺浼氭尋鍘嬪彸渚у彲鐢ㄧ┖闂达紝椤烘墜鎶婂彸渚т篃澶逛竴涓嬨€?
    inspectorWidth.value = clampInspectorWidth(inspectorWidth.value);
  }

  /** 鎷栨嫿鍙充晶鍒嗛殧绾匡細`delta` 涓烘琛ㄧず鍙樺銆?*/
  function resizeInspector(delta: number): void {
    inspectorWidth.value = clampInspectorWidth(inspectorWidth.value + delta);
  }

  /** 瑙嗗彛鍙樺寲鍚庨噸鏂板す绱т袱渚у搴︼細绐楀彛鍙樺皬涓嶈鐣欎笅瓒呭闈㈡澘銆?*/
  function clampPanes(): void {
    trackWidth.value = clampTrackWidth(trackWidth.value);
    inspectorWidth.value = clampInspectorWidth(inspectorWidth.value);
  }

  /** 鎵撳紑锛堟垨鑱氱劍锛変竴涓潰鏉挎爣绛俱€?*/
  function openTab(id: PaneId): void {
    if (!tabs.value.includes(id)) tabs.value.push(id);
    activeTab.value = id;
    showInspector.value = true;
  }

  /** 鍙妸鏍囩鍔犺繘鏉ワ紙涓嶅垏鎹㈠綋鍓嶆爣绛撅級鈥斺€斻€?銆嶉€夋嫨鍣ㄧ敤瀹冦€?*/
  function addTab(id: PaneId): void {
    if (!tabs.value.includes(id)) tabs.value.push(id);
    showInspector.value = true;
  }

  /** 鍏抽棴涓€涓爣绛撅紱鍏虫帀褰撳墠鏍囩鏃舵縺娲荤浉閭荤殑涓€涓€傚叏閮ㄥ叧鎺夊悗闈㈡澘鏄剧ず"鍙墦寮€闈㈡澘"鍒楄〃銆?*/
  function closeTab(id: PaneId): void {
    const idx = tabs.value.indexOf(id);
    if (idx < 0) return;
    tabs.value.splice(idx, 1);
    if (tabs.value.length > 0 && activeTab.value === id) {
      activeTab.value = tabs.value[Math.min(idx, tabs.value.length - 1)];
    }
  }

  /** 寮€鍏冲紑鍙戣€呮娊灞夈€?*/
  function toggleDevDrawer(): void {
    devDrawer.value = !devDrawer.value;
  }

  /** 寮瑰嚭涓€鏉℃彁绀猴紙3.5 绉掑悗鑷姩娑堝け锛夈€?*/
  function notify(message: string): void {
    toast.value = message;
    if (toastTimer !== null) window.clearTimeout(toastTimer);
    toastTimer = window.setTimeout(() => {
      toast.value = null;
      toastTimer = null;
    }, 3500);
  }

  return {
    showTrackHeaders,
    showInspector,
    trackWidth,
    inspectorWidth,
    tabs,
    activeTab,
    tool,
    laneHeight,
    showAutomation,
    showControllerLane,
    noteGrid,
    controllerTarget,
    rollScope,
    trackNotations,
    setTrackNotation,
    trackNotationOf,
    trackClefs,
    trackTunings,
    setTrackClef,
    setTrackTuning,
    setTool,
    resizeLane,
    resizeTrack,
    resizeInspector,
    clampPanes,
    maxTrackWidth,
    maxInspectorWidth,
    devDrawer,
    toast,
    openTab,
    addTab,
    closeTab,
    toggleDevDrawer,
    notify,
  };
});

