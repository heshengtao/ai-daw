//! AU 后端实现（仅 macOS 编译）。
//!
//! 走苹果的 **`AUAudioUnit`**（macOS 10.11+ 的 ObjC 接口）：它把 AUv2（`.component`）与
//! AUv3（app extension）统一成一套 API，并且**没有自带界面的 AU 也能拿到一个通用界面**
//! （`requestViewControllerWithCompletionHandler:`）——这正是 Logic / Cubase / FL 这些
//! 宿主"AU 都能用"的原因。老式的 `AudioUnit` C API 只覆盖 AUv2，且只能显示自带 CocoaUI 的界面。
//!
//! 对应关系：
//! - 发现：两路来源。① 标准目录里的 `.component`（读 `Info.plist` 的 `AudioComponents` 数组，
//!   Apple 自带 AU 也在这里，信息最全）；② `AudioComponent` 注册表（补上 **AUv3** —— 它们是
//!   app extension，不是 `.component` 文件）。按 id 去重。
//! - 实例化：`AUAudioUnit.initWithComponentDescription:options:error:`（优先进程内，失败退回系统默认）。
//! - 处理：`renderBlock`（拉取式，输入由宿主提供的 `AURenderPullInputBlock` 给）。
//! - MIDI：`scheduleMIDIEventBlock`（内部桥接到 AUv2 的 `MusicDeviceMIDIEvent`）。
//! - 参数：`parameterTree`。
//! - 状态：`fullState`（NSDictionary ↔ 二进制 plist 字节）。
//! - 界面：`requestViewControllerWithCompletionHandler:` → `NSViewController.view` 塞进自建 NSWindow。
//!
//! 线程模型对齐 `vst3.rs`：实例包在 `Arc<Mutex<..>>` 里，音频线程**只用 `try_lock`**（抢不到就
//! 本块静音），开界面 / 读参数 / 存取状态在控制侧线程。实例化与"重配"派发到主线程（同 CLAP）。
//!
//! # 踩过的坑
//!
//! 1. **`setFullState` 只能在主线程调**：在别的线程上 `AUAudioUnitV2Bridge` 会打
//!    "failed to execute on the main queue. Falling back to secondary thread." 并异步落地，
//!    那份工作可能在实例析构之后才跑，直接崩在 ObjC 的类查找里。`Plugin::load_state` 因此
//!    总是派发到主线程。
//! 2. **参数值要在 `allocateRenderResources` 之后读**：之前读会得到 0 并刷
//!    `AudioGetParameter ... err -10867`（未初始化）。所以参数表在 `prepare` 之后补读一次默认值。
//! 3. **`renderBlock` / `scheduleMIDIEventBlock` 必须 `Block_copy` 住**：它们返回的是**借来的**
//!    block。配置跑在主线程的一次派发里，任务一结束主线程的 autorelease pool 就会回收它——
//!    音频线程再去调就是野指针，应用里表现为「选中 AU 插件立刻闪退」。
//!    见 `RenderHandles` 与回归测试 `render_blocks_survive_the_main_thread_autorelease_pool`。
//! 4. **`+dataWithPropertyList:...` 这类工厂方法返回的是 +0（autoreleased）对象**：用
//!    `Retained::from_raw` 接管会多释放一次，主线程的 pool 一收就崩在 `objc_release`
//!    （表现为「打不开应用」）。状态存取一律用 objc2 的绑定，别自己拿裸指针接管。
//! 5. **视图控制器只能要一次**：关掉插件窗口后再 `requestViewController...` 会返回 nil
//!    （表现为「插件界面不存在，无法打开」）。所以控制器/窗口缓存在实例上，关闭只是 `orderOut`。
//! 6. **`NSWindow` 默认 `isReleasedWhenClosed = YES`**：关窗时 AppKit 会替我们 release 一次，
//!    与 `Retained` 的释放撞成过度释放（崩在关闭动画 `_NSWindowTransformAnimation dealloc`）。
//!    建窗后必须显式关掉，并且插件窗口不需要关窗动画（`setAnimationBehavior(None)`）。

use std::collections::HashSet;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::sync::{mpsc, Arc, Mutex};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool};
use objc2::{msg_send, sel, AnyThread, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSBackingStoreType, NSView, NSViewController, NSWindow, NSWindowAnimationBehavior,
    NSWindowStyleMask,
};
use objc2_audio_toolbox::{
    kAudioUnitType_Effect, kAudioUnitType_MusicDevice, kAudioUnitType_MusicEffect, AUAudioFrameCount,
    AUAudioUnit, AUAudioUnitBus, AUAudioUnitStatus, AUEventSampleTimeImmediate, AUParameter,
    AUParameterTree, AURenderPullInputBlock,
    AudioComponent, AudioComponentCopyName, AudioComponentDescription,
    AudioComponentFindNext, AudioComponentGetDescription, AudioComponentInstantiationOptions,
    AudioUnitParameterOptions, AudioUnitRenderActionFlags,
};
use objc2_avf_audio::AVAudioFormat;
use objc2_core_audio_types::{
    kAudioFormatFlagIsNonInterleaved, kAudioFormatLinearPCM, kAudioFormatFlagsNativeFloatPacked,
    AudioBuffer, AudioBufferList, AudioStreamBasicDescription, AudioTimeStamp, AudioTimeStampFlags,
};
use objc2_core_foundation::{CFRetained, CFString};
use objc2_foundation::{
    NSArray, NSData, NSDate, NSDefaultRunLoopMode, NSError, NSPropertyListFormat,
    NSPropertyListMutabilityOptions, NSPropertyListSerialization, NSRect, NSRunLoop, NSString,
};

use ai_daw_plugin_api::{
    pitch_bend_midi_bytes, IoLayout, MidiEvent as HostEvent, ParamId, ParamInfo, Plugin,
    PluginCapabilities, PluginDescriptor, PluginError, PluginKind, ProcessContext,
    TimedMidiEvent,
};

use crate::error::HostError;
use crate::scan::ScannedPlugin;

/// 引擎侧恒为立体声（`Player` 每轨固定送左右两条平面缓冲）。
const ENGINE_CHANNELS: usize = 2;
const NO_ERR: i32 = 0;
const DEFAULT_BLOCK: usize = 512;

// ---------------------------------------------------------------------------
// 4CC / 组件描述
// ---------------------------------------------------------------------------

/// 把 4 字符代码（4CC）转成 OSType；不足 4 位补空格。
fn fourcc(code: &str) -> u32 {
    let bytes = code.as_bytes();
    let mut out = [b' '; 4];
    for (i, b) in bytes.iter().take(4).enumerate() {
        out[i] = *b;
    }
    u32::from_be_bytes(out)
}

/// OSType → 4 字符代码文本。
fn fourcc_str(code: u32) -> String {
    let bytes = code.to_be_bytes();
    String::from_utf8_lossy(&bytes).trim_end().to_string()
}

/// 一个 AU 组件的身份：type / subtype / manufacturer 三个 4CC。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuDesc {
    pub type_: u32,
    pub subtype: u32,
    pub manufacturer: u32,
}

impl AuDesc {
    /// 从 `au:<type>:<subtype>:<manufacturer>` 解析。
    pub fn from_id(id: &str) -> Result<Self, HostError> {
        let rest = id.strip_prefix("au:").unwrap_or(id);
        let parts: Vec<&str> = rest.split(':').collect();
        if parts.len() != 3 {
            return Err(HostError::Load(format!("AU 插件 id 格式不对：{id}")));
        }
        Ok(Self {
            type_: fourcc(parts[0]),
            subtype: fourcc(parts[1]),
            manufacturer: fourcc(parts[2]),
        })
    }

    /// 反向：`au:<type>:<subtype>:<manufacturer>`（工程里持久化的稳定键）。
    pub fn to_id(self) -> String {
        format!(
            "au:{}:{}:{}",
            fourcc_str(self.type_),
            fourcc_str(self.subtype),
            fourcc_str(self.manufacturer)
        )
    }

    fn description(self) -> AudioComponentDescription {
        AudioComponentDescription {
            componentType: self.type_,
            componentSubType: self.subtype,
            componentManufacturer: self.manufacturer,
            componentFlags: 0,
            componentFlagsMask: 0,
        }
    }

    /// AU 类型 → 本项目分类：`aumu`（MusicDevice）= 音源，`aumf`/`aufx` = 效果。
    fn kind(self) -> PluginKind {
        if self.type_ == kAudioUnitType_MusicDevice {
            PluginKind::Instrument
        } else {
            PluginKind::Effect
        }
    }

    /// 类理解 MIDI：音源与 MIDI 效果都能接。
    fn accepts_midi(self) -> bool {
        self.type_ == kAudioUnitType_MusicDevice || self.type_ == kAudioUnitType_MusicEffect
    }
}

/// 只保留能当乐器/效果挂到轨道上的 AU，跳过 output / mixer / converter / generator / panner / codec。
fn is_hostable(type_: u32) -> bool {
    // 不用 `matches!`：这些常量是小写命名的，放进模式会触发 `non_upper_case_globals`。
    type_ == kAudioUnitType_Effect
        || type_ == kAudioUnitType_MusicDevice
        || type_ == kAudioUnitType_MusicEffect
}

// ---------------------------------------------------------------------------
// 扫描
// ---------------------------------------------------------------------------

/// AU 的标准安装目录（用户级在前，系统级在后）。
fn au_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join("Library/Audio/Plug-Ins/Components"));
    }
    dirs.push(PathBuf::from("/Library/Audio/Plug-Ins/Components"));
    // Apple 自带 AU（AUPeakLimiter / AUMatrixReverb / DLSMusicDevice…）都声明在这里。
    dirs.push(PathBuf::from("/System/Library/Components"));
    dirs
}

/// 扫描本机 AU 插件，额外扫描 `extra` 里的目录。
///
/// 两路来源：
/// 1. **标准目录里的 `.component`**：读 `Info.plist`，信息最全（厂商名 + 包路径）。
/// 2. **`AudioComponent` 注册表**：补上目录里看不到的那些——主要是 **AUv3**（app extension，
///    不是 `.component` 文件），以及少数由框架自己注册的 AU。
///
/// 全程**不实例化**插件：`AUAudioUnit` 基类对每个 AU 都能给一个界面（自带或通用参数界面），
/// 所以 `has_editor` 恒为 true，不必为了探测去把几十个 AU 都建一遍（那会让扫描慢二十多秒）。
pub fn scan(extra: &[PathBuf]) -> Vec<ScannedPlugin> {
    let mut out = Vec::new();
    let mut dirs = au_dirs();
    for dir in extra {
        if !dirs.contains(dir) {
            dirs.push(dir.clone());
        }
    }
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("component") {
                continue;
            }
            out.extend(read_component(&path));
        }
    }

    // 第二路：注册表里还有、但目录扫描没覆盖到的（AUv3 等）。按 id 去重。
    let known: HashSet<String> = out.iter().map(|p| p.id.clone()).collect();
    out.extend(registered_units(known));
    out
}

/// 枚举 `AudioComponent` 注册表里能挂轨道的 AU，跳过 `known` 里已有的。
///
/// AUv3 不是独立文件，只能这样发现；这类条目**没有包路径**可给，`path` 留空——于是插件管理器
/// 会把它当「外部（只读）」，与事实一致。
fn registered_units(mut seen: HashSet<String>) -> Vec<ScannedPlugin> {
    let mut out = Vec::new();
    // 三种类型各扫一遍：把类型交给 API 过滤，避免遍历到 codec / output / mixer 那些。
    for type_code in [
        kAudioUnitType_Effect,
        kAudioUnitType_MusicDevice,
        kAudioUnitType_MusicEffect,
    ] {
        let mut component: AudioComponent = std::ptr::null_mut();
        loop {
            // 每轮都给一个「只定 type、其余通配」的描述。`FindNext` 会把找到的组件描述回填进
            // `desc`，不重置的话下一轮就被收窄成「只找这一个组件」了。
            let mut desc = AudioComponentDescription {
                componentType: type_code,
                componentSubType: 0,
                componentManufacturer: 0,
                componentFlags: 0,
                componentFlagsMask: 0,
            };
            // SAFETY: 传合法 desc 指针；`component` 是上一项（首轮为空）。
            component = unsafe { AudioComponentFindNext(component, NonNull::from(&mut desc)) };
            if component.is_null() {
                break;
            }
            let mut actual: AudioComponentDescription = unsafe { std::mem::zeroed() };
            // SAFETY: `component` 非空合法；出参合法。
            if unsafe { AudioComponentGetDescription(component, NonNull::from(&mut actual)) }
                != NO_ERR
            {
                continue;
            }
            let au = AuDesc {
                type_: actual.componentType,
                subtype: actual.componentSubType,
                manufacturer: actual.componentManufacturer,
            };
            if !is_hostable(au.type_) {
                continue;
            }
            let id = au.to_id();
            if !seen.insert(id.clone()) {
                continue;
            }
            let name = component_name(component).unwrap_or_else(|| id.clone());
            out.push(ScannedPlugin {
                id,
                name: name.clone(),
                vendor: vendor_of(&name, au.manufacturer),
                kind: au.kind(),
                format: "au",
                accepts_midi: au.accepts_midi(),
                has_editor: true,
                path: PathBuf::new(),
            });
        }
    }
    out
}

/// 问出一个注册组件的显示名（通常是 `"厂商: 名字"`）。
fn component_name(component: AudioComponent) -> Option<String> {
    let mut name: *const CFString = std::ptr::null();
    // SAFETY: `component` 合法；出参合法。返回的 CFString 是 +1 的。
    let status = unsafe { AudioComponentCopyName(component, NonNull::from(&mut name)) };
    if status != NO_ERR || name.is_null() {
        return None;
    }
    // SAFETY: 属性成功时返回 +1 的 CFString；CFString 与 NSString toll-free bridged。
    let owned = unsafe { CFRetained::from_raw(NonNull::new_unchecked(name as *mut CFString)) };
    let ns: &NSString = unsafe { &*(&*owned as *const CFString as *const NSString) };
    Some(ns.to_string())
}

/// 读出一个 `.component` 里声明的全部 AU。
fn read_component(bundle: &Path) -> Vec<ScannedPlugin> {
    let info_plist = bundle.join("Contents/Info.plist");
    let Ok(value) = plist::Value::from_file(&info_plist) else {
        return Vec::new();
    };
    let Some(dict) = value.as_dictionary() else {
        return Vec::new();
    };
    let Some(components) = dict.get("AudioComponents").and_then(|v| v.as_array()) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for item in components {
        let Some(entry) = item.as_dictionary() else { continue };
        let get = |key: &str| entry.get(key).and_then(|v| v.as_string()).map(str::to_string);
        let (Some(type_str), Some(subtype), Some(manufacturer)) =
            (get("type"), get("subtype"), get("manufacturer"))
        else {
            continue;
        };
        let desc = AuDesc {
            type_: fourcc(&type_str),
            subtype: fourcc(&subtype),
            manufacturer: fourcc(&manufacturer),
        };
        if !is_hostable(desc.type_) {
            continue;
        }
        let name = get("name").unwrap_or_else(|| desc.to_id());
        out.push(ScannedPlugin {
            id: desc.to_id(),
            name: name.clone(),
            vendor: vendor_of(&name, desc.manufacturer),
            kind: desc.kind(),
            format: "au",
            accepts_midi: desc.accepts_midi(),
            // `AUAudioUnit` 基类对**每个** AU 都能给一个界面：插件自带的，或系统给的通用参数界面
            // （Apple 自带的那批就是后者）。所以恒为 true，也不必为了探测去实例化（那会让扫描
            // 慢二十多秒）。真正的失败在打开时才暴露。
            has_editor: true,
            path: bundle.to_path_buf(),
        });
    }
    out
}

/// 从 AU 的显示名里取厂商（`"Apple: AUPeakLimiter"` → `Apple`）；取不到就用 4CC。
fn vendor_of(name: &str, manufacturer: u32) -> String {
    if let Some((vendor, _)) = name.split_once(':') {
        let vendor = vendor.trim();
        if !vendor.is_empty() {
            return vendor.to_string();
        }
    }
    if manufacturer == fourcc("appl") {
        "Apple".to_string()
    } else {
        fourcc_str(manufacturer)
    }
}

// ---------------------------------------------------------------------------
// 组件实例
// ---------------------------------------------------------------------------

/// 已经建好的插件界面。
///
/// **必须长期持有**：`AUAudioUnit` 通常只肯给一次视图控制器，关掉窗口后再去
/// `requestViewController...` 会返回 nil（应用里表现为「插件界面不存在，无法打开」）。所以
/// 窗口/控制器/视图都缓存在实例上，重新打开时复用同一份，只是重新 `makeKeyAndOrderFront`。
struct EditorState {
    /// **只要为了持有**：AU 只肯给一次视图控制器，宿主必须替它把它养着。
    _controller: Retained<NSViewController>,
    window: Retained<NSWindow>,
}

/// 把界面对象送过线程边界的包装（释放必须发生在主线程）。
struct SendEditor(EditorState);
// SAFETY: 只用来把它们搬到主线程上释放，不跨线程并发使用。
unsafe impl Send for SendEditor {}

/// 一个 AU 实例（`AUAudioUnit` + 它上面查好的参数树 + 已建好的界面）。
///
/// **字段顺序有意义**：`tree` 必须声明在 `au` 之前——参数树是实例的一部分，先放掉实例再释放树
/// 有机会踩到悬空引用（实测偶发在进程退出时崩）。
struct AuInstance {
    tree: Option<Retained<AUParameterTree>>,
    au: Option<Retained<AUAudioUnit>>,
    /// 已经打开过的界面（关窗后**不销毁**，留给下次打开复用）。
    editor: Option<EditorState>,
    /// 是否已 `allocateRenderResources`（决定拆除时要不要 deallocate）。
    allocated: bool,
}

impl AuInstance {
    /// 实例本体。**只有 `Drop` 里会把它取走**，其余地方都借。
    fn au_ref(&self) -> &Retained<AUAudioUnit> {
        self.au.as_ref().expect("AU 实例只在 Drop 里被取走")
    }
}

// SAFETY: `AUAudioUnit` 是 ObjC 对象（`!Send`）。我们的用法是把它关在一把 `Mutex` 里：音频线程
// 只 `try_lock` 后调 block 做渲染，控制侧线程用它开界面/读参数/存取状态；AUAudioUnit 本身就允许
// 渲染线程与控制线程并行。跨线程搬运这个句柄因此是安全的。本 crate 是工作区里唯一允许 `unsafe`
// 的地方，这条保证由它承担。
unsafe impl Send for AuInstance {}

/// 把 `Retained` 送过线程边界的包装（`on_main_thread` 要求闭包 `Send`）。
struct SendRetained<T: ?Sized>(Retained<T>);
// SAFETY: 与 `AuInstance` 同：只用来把实例搬到主线程上拆除 / 建好后搬回来。
unsafe impl<T: ?Sized> Send for SendRetained<T> {}

impl Drop for AuInstance {
    fn drop(&mut self) {
        // 拆除（deallocate + 释放）交给**主线程**做：AUv2 桥的拆除偏主线程，在别的线程上放掉
        // 实测会出现偶发的退出崩溃。与 CLAP 后端把 `destroy` 搬回主线程是同一个理由。
        // 界面对象（NSWindow/NSView）同样只能在主线程释放。
        let au = self.au.take().map(SendRetained);
        let editor = self.editor.take().map(SendEditor);
        let allocated = self.allocated;
        let _ = crate::dispatch::on_main_thread(move || {
            if let Some(SendEditor(editor)) = editor {
                editor.window.close();
                drop(editor);
            }
            if let Some(SendRetained(au)) = au {
                if allocated {
                    // SAFETY: 只有在没有别的持有者时才会走到这里（`Arc` 的最后一份）。
                    unsafe { au.deallocateRenderResources() };
                }
                drop(au);
            }
        });
    }
}

/// 建实例的产物：`AUAudioUnit` 与它的参数树。
struct NewInstance {
    au: Retained<AUAudioUnit>,
    tree: Option<Retained<AUParameterTree>>,
}

// SAFETY: 与 `AuInstance` 同：两个 `Retained` 只在 `on_main_thread` 的返回值里跨线程搬一次。
unsafe impl Send for NewInstance {}

/// 实例化一个 `AUAudioUnit` 并读它的参数树。**必须在主线程调用。**
fn build_instance_on_main(desc: AuDesc) -> Result<NewInstance, HostError> {
    let au = instantiate_on_main(desc)?;
    // SAFETY: 实例刚建好，合法。
    let tree = unsafe { au.parameterTree() };
    Ok(NewInstance { au, tree })
}

/// 实例化一个 `AUAudioUnit`。**必须在主线程调用。**
fn instantiate_on_main(desc: AuDesc) -> Result<Retained<AUAudioUnit>, HostError> {
    let cd = desc.description();
    // SAFETY: `cd` 按值传入；`alloc()` 只是分配，初始化在下面完成。
    let sync = unsafe {
        // 先试「进程内」——AUv2 与大多数 AUv3 都能这样跑，且省掉一层 XPC。
        match AUAudioUnit::initWithComponentDescription_options_error(
            AUAudioUnit::alloc(),
            cd,
            AudioComponentInstantiationOptions::LoadInProcess,
        ) {
            Ok(au) => Ok(au),
            // 有些 AUv3 只能在进程外跑；交给系统自己决定。
            Err(_) => AUAudioUnit::initWithComponentDescription_error(AUAudioUnit::alloc(), cd),
        }
    };
    let error = match sync {
        Ok(au) => return Ok(au),
        Err(err) => err,
    };
    // 还有一类 AU 声明了 `RequiresAsyncInstantiation`（不少进程外的 AUv3 如此），同步接口必然
    // 失败——只能走异步接口。
    instantiate_async(desc).ok_or_else(|| {
        HostError::Load(format!("AU 实例化失败（{}）：{error}", desc.to_id()))
    })
}

/// 异步实例化。**必须在主线程调用**：这里泵主线程 run loop 等回调。
///
/// 进程外的 AUv3 只支持这种方式（同步接口会返回「需要异步实例化」）。
fn instantiate_async(desc: AuDesc) -> Option<Retained<AUAudioUnit>> {
    use std::time::{Duration, Instant};

    const TIMEOUT: Duration = Duration::from_secs(5);

    let (tx, rx) = mpsc::channel::<Option<Retained<AUAudioUnit>>>();
    let block = RcBlock::new(move |au: *mut AUAudioUnit, _error: *mut NSError| {
        // SAFETY: 回调给的是 +0 的实例，`Retained::retain` 把它钉住再送回去。
        let owned = unsafe { Retained::retain(au) };
        let _ = tx.send(owned);
    });
    // SAFETY: `desc` 按值传入；`block` 的签名与接口一致。
    unsafe {
        AUAudioUnit::instantiateWithComponentDescription_options_completionHandler(
            desc.description(),
            AudioComponentInstantiationOptions::LoadOutOfProcess,
            &block,
        );
    }

    let deadline = Instant::now() + TIMEOUT;
    loop {
        if let Ok(result) = rx.try_recv() {
            return result;
        }
        if Instant::now() >= deadline {
            return None;
        }
        pump_main_run_loop(0.01);
    }
}

// ---------------------------------------------------------------------------
// 参数
// ---------------------------------------------------------------------------

struct AuParamMeta {
    address: u64,
    id: ParamId,
    name: String,
    min: f32,
    max: f32,
    default: f32,
    automatable: bool,
}

/// 读出一个 AU 的参数表。
///
/// `AUParameter` 没有"默认值"这个属性（老 C API 的 `AudioUnitParameterInfo.defaultValue` 在
/// AUv3 里没有对应物），所以这里的 `default` 取**加载那一刻的当前值**——也就是插件自称的初始值。
fn collect_params(tree: Option<&AUParameterTree>) -> Vec<AuParamMeta> {
    let Some(tree) = tree else { return Vec::new() };
    // SAFETY: `tree` 来自刚建好的实例，合法。
    let all: Retained<NSArray<AUParameter>> = unsafe { tree.allParameters() };
    all.to_vec()
        .into_iter()
        .map(|param| {
            // SAFETY: 参数对象来自参数树。
            unsafe {
                let address = param.address();
                AuParamMeta {
                    address,
                    id: ParamId(address.to_string()),
                    name: param.displayName().to_string(),
                    min: param.minValue(),
                    max: param.maxValue(),
                    default: param.value(),
                    automatable: param
                        .flags()
                        .contains(AudioUnitParameterOptions::Flag_IsWritable),
                }
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// 状态
// ---------------------------------------------------------------------------

/// 在**当前线程**读插件状态（`fullState` → 二进制 plist 字节）。
///
/// 必须用绑定而不是裸 `msg_send!` + `Retained::from_raw`：`dataWithPropertyList:...` 是
/// **autorelease 工厂方法**（返回 +0），当成 +1 接管会多释放一次——主线程的 autorelease pool
/// 收的时候直接崩在 `objc_release`（应用里就是「打不开 / 选插件闪退」）。
fn save_state_here(shared: &Arc<Mutex<AuInstance>>) -> Vec<u8> {
    let Ok(inst) = shared.lock() else { return Vec::new() };
    // SAFETY: 实例合法；`fullState` 返回一个当前状态的字典快照。
    unsafe {
        let Some(state) = inst.au_ref().fullState() else {
            return Vec::new();
        };
        // `&NSDictionary` → `&AnyObject`：同一个对象指针，只是类型层面的向上转换。
        let plist: &AnyObject = &*(&*state as *const _ as *const AnyObject);
        match NSPropertyListSerialization::dataWithPropertyList_format_options_error(
            plist,
            NSPropertyListFormat::BinaryFormat_v1_0,
            0,
        ) {
            Ok(data) => data.to_vec(),
            Err(_) => Vec::new(),
        }
    }
}

/// 在**当前线程**写插件状态（二进制 plist 字节 → `fullState`）。
fn load_state_here(shared: &Arc<Mutex<AuInstance>>, state: &[u8]) -> Result<(), PluginError> {
    let inst = shared
        .lock()
        .map_err(|_| PluginError::InvalidState("AU 实例锁中毒".into()))?;
    let data = NSData::with_bytes(state);
    // SAFETY: 从 plist 字节解析出对象（通常是个字典），再原样交给插件。
    unsafe {
        let plist = NSPropertyListSerialization::propertyListWithData_options_format_error(
            &data,
            NSPropertyListMutabilityOptions(0),
            std::ptr::null_mut(),
        )
        .map_err(|err| PluginError::InvalidState(format!("状态不是合法的 plist：{err}")))?;
        let _: () = msg_send![&**inst.au_ref(), setFullState: &*plist];
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 配置与渲染
// ---------------------------------------------------------------------------

/// 输入回调（拉取式）用的上下文：AU 渲染时通过它来"拉"宿主的输入。
///
/// 必须放在**堆上固定地址**（`Box`），因为它的指针在创建拉取块时就写进去了，
/// 之后每个块只更新里面的通道指针。
#[repr(C)]
struct InputBuffer {
    channels: [*const f32; ENGINE_CHANNELS],
    frames: usize,
}

// SAFETY: 里面的指针只是本块输入的只读视图，写入与读取都在音频线程上、且受 `AuHostedPlugin`
// 的 `&mut self` 独占约束。
unsafe impl Send for InputBuffer {}

/// 拉取式输入块：`(flags, timestamp, frameCount, bus, ioData)`。
type PullBlock = RcBlock<
    dyn Fn(
        NonNull<AudioUnitRenderActionFlags>,
        NonNull<AudioTimeStamp>,
        AUAudioFrameCount,
        isize,
        NonNull<AudioBufferList>,
    ) -> AUAudioUnitStatus,
>;

/// 渲染块：`(flags, timestamp, frameCount, outputBus, ioData, pullInput)`。
type RenderBlock = RcBlock<
    dyn Fn(
        NonNull<AudioUnitRenderActionFlags>,
        NonNull<AudioTimeStamp>,
        AUAudioFrameCount,
        isize,
        NonNull<AudioBufferList>,
        AURenderPullInputBlock,
    ) -> AUAudioUnitStatus,
>;

/// MIDI 块：`(eventSampleTime, cable, length, bytes)`。
type MidiBlock = RcBlock<dyn Fn(i64, u8, isize, NonNull<u8>)>;

/// 配置阶段产出的渲染句柄（要在音频线程上用，所以整体 `Send`）。
///
/// **`render` / `midi` 必须是 `Block_copy` 过的（`RcBlock`），不能只存裸指针**：
/// `AUAudioUnit.renderBlock` / `scheduleMIDIEventBlock` 返回的是**借来的** block，而我们的配置
/// 跑在主线程的一次派发里——那块任务一结束，主线程的 autorelease pool 就可能把它回收掉，
/// 音频线程再去调就是野指针（实测直接闪退在 `Block::call`）。
struct RenderHandles {
    render: RenderBlock,
    midi: Option<MidiBlock>,
    pull: PullBlock,
}

// SAFETY: 这些 block 句柄由 AU 持有并保证在它存活期间有效；它们只在音频线程上被调用，
// 创建/销毁发生在控制侧线程。`PullBlock` 捕获的是 `InputBuffer` 的稳定地址（`Box`）。
unsafe impl Send for RenderHandles {}

/// 把宿主当前块的输入交给 AU。
unsafe fn copy_input(ctx: &InputBuffer, frames: usize, io_data: *mut AudioBufferList) {
    if io_data.is_null() {
        return;
    }
    // SAFETY: `io_data` 是本块渲染的合法缓冲区列表。
    let abl = unsafe { &mut *io_data };
    let count = abl.mNumberBuffers as usize;
    // SAFETY: `mBuffers` 后面跟着 `count` 个 `AudioBuffer`（变长数组）。
    let buffers = unsafe { std::slice::from_raw_parts_mut(abl.mBuffers.as_mut_ptr(), count) };
    let n = frames.min(ctx.frames);
    for (i, buf) in buffers.iter_mut().enumerate() {
        let src_ptr = ctx.channels.get(i).copied().unwrap_or(std::ptr::null());
        if src_ptr.is_null() || buf.mData.is_null() || n == 0 {
            buf.mDataByteSize = 0;
            continue;
        }
        // SAFETY: 通道指针指向本块输入（长度 >= n），目标缓冲区由 AU 提供且长度 >= n。
        let src = unsafe { std::slice::from_raw_parts(src_ptr, n) };
        let dst = unsafe { std::slice::from_raw_parts_mut(buf.mData as *mut f32, n) };
        dst.copy_from_slice(src);
        buf.mDataByteSize = (n * std::mem::size_of::<f32>()) as u32;
    }
}

/// 建一个拉取块，把 `ctx` 指向的输入喂给 AU。
fn make_pull_block(ctx: *const InputBuffer) -> PullBlock {
    let addr = ctx as usize;
    RcBlock::new(
        move |_flags: NonNull<AudioUnitRenderActionFlags>,
              _ts: NonNull<AudioTimeStamp>,
              frames: AUAudioFrameCount,
              _bus: isize,
              io_data: NonNull<AudioBufferList>|
              -> AUAudioUnitStatus {
            // SAFETY: 地址来自 `AuHostedPlugin` 里那个地址稳定的 `Box<InputBuffer>`。
            let ctx = unsafe { &*(addr as *const InputBuffer) };
            unsafe { copy_input(ctx, frames as usize, io_data.as_ptr()) };
            NO_ERR
        },
    )
}

/// 立体声浮点非交错流格式。
fn stream_format(sample_rate: f64) -> AudioStreamBasicDescription {
    AudioStreamBasicDescription {
        mSampleRate: sample_rate,
        mFormatID: kAudioFormatLinearPCM,
        mFormatFlags: kAudioFormatFlagsNativeFloatPacked | kAudioFormatFlagIsNonInterleaved,
        mBytesPerPacket: 4,
        mFramesPerPacket: 1,
        mBytesPerFrame: 4,
        mChannelsPerFrame: ENGINE_CHANNELS as u32,
        mBitsPerChannel: 32,
        mReserved: 0,
    }
}

/// 给总线设格式。`AUAudioUnitBus` 的格式是 `AVAudioFormat`（绑定里没生成 `setFormat:`），
/// 所以自己发消息。
fn set_bus_format(bus: &AUAudioUnitBus, format: &AVAudioFormat) -> bool {
    // SAFETY: `bus` 合法；`error:` 传 NULL 是允许的（我们只关心成败）。
    let ok: Bool = unsafe {
        msg_send![bus, setFormat: format, error: std::ptr::null_mut::<*mut NSError>()]
    };
    ok.as_bool()
}

/// 在**当前线程（主线程）**上配置并激活：设总线格式 / 块大小上界，再 `allocateRenderResources`。
fn configure_on_main(
    shared: &Arc<Mutex<AuInstance>>,
    has_input: bool,
    sample_rate: f64,
    block_size: usize,
    input_ctx: usize,
) -> Option<RenderHandles> {
    let mut inst = shared.lock().ok()?;
    let au = Retained::clone(inst.au_ref());
    // SAFETY: 所有调用都针对合法实例；AVAudioFormat 由我们的 ASBD 构造。
    unsafe {
        if inst.allocated {
            au.deallocateRenderResources();
            inst.allocated = false;
        }

        let fmt = stream_format(sample_rate);
        let av_format = AVAudioFormat::initWithStreamDescription(
            AVAudioFormat::alloc(),
            NonNull::from(&fmt),
        )?;

        // 输出总线（音源与效果器都要）。
        let outs = au.outputBusses();
        if outs.count() == 0 {
            return None;
        }
        let out_bus = outs.objectAtIndexedSubscript(0);
        if !set_bus_format(&out_bus, &av_format) {
            return None;
        }
        // 总线默认是「未启用」的；不启用渲染时会报 NoConnection（输入也拿不到）。
        out_bus.setEnabled(true);
        if has_input {
            let ins = au.inputBusses();
            if ins.count() == 0 {
                return None;
            }
            let in_bus = ins.objectAtIndexedSubscript(0);
            if !set_bus_format(&in_bus, &av_format) {
                return None;
            }
            in_bus.setEnabled(true);
        }

        // 注意：`AUAudioFrameCount` 在绑定里是 u32，但运行时 `setMaximumFramesToRender:` 的实际
        // 编码是 64 位（'Q'）——用绑定的签名发消息会被 objc2 的校验挡下，所以这里自己发消息。
        let max_frames = block_size.max(1) as u64;
        let _: () = msg_send![&*au, setMaximumFramesToRender: max_frames];

        // 文档要求这两个块在 `allocateRenderResources` **之前**取。
        //
        // **必须 `Block_copy`（`.copy()`）**：这两个属性返回的是借来的 block，配置任务一结束、
        // 主线程的 autorelease pool 一收，裸指针就可能悬空；音频线程再去调就会闪退。
        let render_raw = au.renderBlock();
        if render_raw.is_null() {
            return None;
        }
        let render: RenderBlock = (&*render_raw).copy();
        let midi = if au.isMusicDeviceOrEffect() {
            let raw = au.scheduleMIDIEventBlock();
            if raw.is_null() {
                None
            } else {
                Some((&*raw).copy())
            }
        } else {
            None
        };

        au.allocateRenderResourcesAndReturnError().ok()?;
        inst.allocated = true;

        let pull = make_pull_block(input_ctx as *const InputBuffer);
        Some(RenderHandles { render, midi, pull })
    }
}

// ---------------------------------------------------------------------------
// 托管适配器
// ---------------------------------------------------------------------------

/// 一个被托管的 AU 实例。
pub struct AuHostedPlugin {
    shared: Arc<Mutex<AuInstance>>,
    id: String,
    name: String,
    kind: PluginKind,
    io: IoLayout,
    params: Vec<AuParamMeta>,
    sample_rate: f64,
    block_size: usize,
    /// 配置好之后才有；渲染全靠它。
    handles: Option<RenderHandles>,
    /// 效果器的输入暂存（同时也被拉取块通过 `input_ctx` 读取）。
    in_buf: Vec<Vec<f32>>,
    out_buf: Vec<Vec<f32>>,
    /// 拉取块的上下文；堆分配，地址稳定。
    input_ctx: Box<InputBuffer>,
    /// 已渲染的采样数（作为时间戳给插件，音源会拿它调度 MIDI）。
    sample_position: u64,
    pending: Vec<(ParamId, f32)>,
    /// 通道弯音范围（半音）。
    pitch_bend_range: f32,
}

impl AuHostedPlugin {
    /// 加载一个 AU 实例。
    ///
    /// 实例化派发到主线程执行；真正的 DSP 配置推迟到 [`Plugin::prepare`]（那时才知道采样率/块大小）。
    pub fn from_loaded(id: String) -> Result<Self, HostError> {
        let desc = AuDesc::from_id(&id)?;
        let owned_id = id.clone();
        // 实例化 + 读参数树都派发到主线程（失败则就地建）。
        let created = crate::dispatch::on_main_thread(move || build_instance_on_main(desc));
        let instance = match created {
            Ok(result) => result?,
            Err(_) => build_instance_on_main(desc)?,
        };
        let NewInstance { au, tree } = instance;

        let kind = desc.kind();
        let name = unsafe { au.audioUnitName() }
            .map(|n| n.to_string())
            .unwrap_or_else(|| owned_id.clone());
        let params = collect_params(tree.as_deref());
        let inputs = if kind == PluginKind::Effect { ENGINE_CHANNELS } else { 0 };

        Ok(AuHostedPlugin {
            shared: Arc::new(Mutex::new(AuInstance {
                tree,
                au: Some(au),
                editor: None,
                allocated: false,
            })),
            id: owned_id,
            name,
            kind,
            io: IoLayout { inputs: inputs as u16, outputs: ENGINE_CHANNELS as u16 },
            params,
            sample_rate: 48_000.0,
            block_size: DEFAULT_BLOCK,
            handles: None,
            in_buf: vec![vec![0.0; DEFAULT_BLOCK]; ENGINE_CHANNELS],
            out_buf: vec![vec![0.0; DEFAULT_BLOCK]; ENGINE_CHANNELS],
            input_ctx: Box::new(InputBuffer {
                channels: [std::ptr::null(); ENGINE_CHANNELS],
                frames: 0,
            }),
            sample_position: 0,
            pending: Vec::new(),
            pitch_bend_range: 2.0,
        })
    }

    /// 给界面用的编辑器句柄。
    pub fn editor_handle(&self) -> AuEditorHandle {
        AuEditorHandle { shared: Arc::clone(&self.shared) }
    }
}

impl Plugin for AuHostedPlugin {
    fn descriptor(&self) -> PluginDescriptor {
        PluginDescriptor {
            id: self.id.clone(),
            name: self.name.clone(),
            kind: self.kind,
            io: self.io,
            // AUAudioUnit 基类总能给一个界面（自带或通用参数界面）。
            capabilities: PluginCapabilities {
                per_note_volume: false,
                has_editor: true,
                per_note_pitch: false,
                channel_pitch_bend: self.kind == PluginKind::Instrument,
            },
        }
    }

    fn params(&self) -> Vec<ParamInfo> {
        self.params
            .iter()
            .map(|p| ParamInfo {
                id: p.id.clone(),
                name: p.name.clone(),
                min: p.min,
                max: p.max,
                default: p.default,
                automatable: p.automatable,
            })
            .collect()
    }

    fn get_param(&self, id: &ParamId) -> Option<f32> {
        let meta = self.params.iter().find(|p| &p.id == id)?;
        let inst = self.shared.lock().ok()?;
        let tree = inst.tree.as_deref()?;
        // SAFETY: 参数树与地址都来自这个实例。
        let param = unsafe { tree.parameterWithAddress(meta.address) }?;
        // SAFETY: 参数对象来自参数树。
        Some(unsafe { param.value() })
    }

    fn set_param(&mut self, id: &ParamId, value: f32) {
        if !value.is_finite() {
            return;
        }
        let Some(meta) = self.params.iter().find(|p| &p.id == id) else { return };
        let Ok(inst) = self.shared.lock() else { return };
        let Some(tree) = inst.tree.as_deref() else { return };
        // SAFETY: 参数树与地址都来自这个实例。
        if let Some(param) = unsafe { tree.parameterWithAddress(meta.address) } {
            // SAFETY: 参数对象来自参数树。
            unsafe { param.setValue(value) };
        }
    }

    fn prepare(&mut self, sample_rate: f64, block_size: usize) {
        let rate_changed = (self.sample_rate - sample_rate).abs() > f64::EPSILON;
        self.sample_rate = sample_rate;
        self.block_size = block_size.max(1);
        if rate_changed {
            for ch in self.in_buf.iter_mut() {
                ch.resize(self.block_size, 0.0);
            }
        }
        for ch in self.out_buf.iter_mut() {
            ch.resize(self.block_size, 0.0);
        }

        // 重配要在主线程做（总线格式 / allocate 都偏主线程）。
        let has_input = self.io.inputs > 0;
        let input_ctx = &*self.input_ctx as *const InputBuffer as usize;
        let sr = self.sample_rate;
        let bs = self.block_size;
        let shared = Arc::clone(&self.shared);
        let fallback = Arc::clone(&shared);
        let handles = match crate::dispatch::on_main_thread(move || {
            configure_on_main(&shared, has_input, sr, bs, input_ctx)
        }) {
            Ok(result) => result,
            // 主线程已退出：退回就地配置。线程不理想，但总比整条加载不出来强。
            Err(_) => configure_on_main(&fallback, has_input, sr, bs, input_ctx),
        };
        if handles.is_none() {
            eprintln!("AU 插件激活失败：{}", self.id);
        }
        self.handles = handles;
        // 参数值只有在**分配了渲染资源之后**才读得到（AUv2 桥在未初始化时读会报 -10867 并给 0），
        // 所以「默认值」在这里补读一次——此刻的值就是插件刚加载完的初始值。
        if self.handles.is_some() {
            if let Ok(inst) = self.shared.lock() {
                if let Some(tree) = inst.tree.as_deref() {
                    for meta in self.params.iter_mut() {
                        // SAFETY: 参数树与地址都来自这个实例。
                        if let Some(param) = unsafe { tree.parameterWithAddress(meta.address) } {
                            // SAFETY: 参数对象来自参数树。
                            meta.default = unsafe { param.value() };
                        }
                    }
                }
            }
        }
    }

    fn set_pitch_bend_range(&mut self, semitones: f32) {
        self.pitch_bend_range = if semitones.is_finite() && semitones > 0.0 {
            semitones.clamp(0.01, 24.0)
        } else {
            2.0
        };
    }

    fn reset(&mut self) {
        // 音频线程：切断全部发声。用 try_lock，绝不阻塞实时线程。
        let Ok(inst) = self.shared.try_lock() else { return };
        // SAFETY: 实例合法。
        unsafe { inst.au_ref().reset() };
    }

    fn process(&mut self, _ctx: &ProcessContext, audio: &mut [&mut [f32]], events: &[TimedMidiEvent]) {
        let frames = audio.first().map_or(0, |c| c.len());
        if frames == 0 || audio.len() < ENGINE_CHANNELS {
            return;
        }
        let Some(handles) = self.handles.as_ref() else { return };
        // 界面/参数线程可能正持有实例：本块先静音，下一块继续（同 VST3）。
        let Ok(_inst) = self.shared.try_lock() else { return };

        let inputs = self.io.inputs as usize;
        if inputs > 0 {
            if self.in_buf[0].len() < frames {
                for ch in self.in_buf.iter_mut() {
                    ch.resize(frames, 0.0);
                }
            }
            for (staging, input) in self
                .in_buf
                .iter_mut()
                .zip(audio.iter())
                .take(inputs.min(ENGINE_CHANNELS))
            {
                staging[..frames].copy_from_slice(&input[..frames]);
            }
        }
        if self.out_buf[0].len() < frames {
            for ch in self.out_buf.iter_mut() {
                ch.resize(frames, 0.0);
            }
        }
        for ch in self.out_buf.iter_mut() {
            ch[..frames].fill(0.0);
        }
        // 把本块的输入通道指针告诉拉取块。
        for i in 0..ENGINE_CHANNELS {
            self.input_ctx.channels[i] = self.in_buf[i].as_ptr();
        }
        self.input_ctx.frames = frames;

        // MIDI：`AUEventSampleTimeImmediate + 块内偏移` 就是"本块的第 N 帧"。
        if let Some(midi) = handles.midi.as_ref() {
            let last = (frames - 1) as i64;
            for e in events {
                let (status, data1, data2) = match e.event {
                    HostEvent::NoteOn { note, velocity } => {
                        (0x90u8, note, velocity.min(127))
                    }
                    HostEvent::NoteOff { note } => (0x80, note, 0),
                    // AUv2 没有逐音表达；通道弯音对所有音源都可用。
                    HostEvent::PitchBend { cents } | HostEvent::NotePitch { cents, .. } => {
                        let bytes = pitch_bend_midi_bytes(cents, self.pitch_bend_range);
                        (bytes[0], bytes[1], bytes[2])
                    }
                    HostEvent::ControlChange { controller, value } => {
                        (0xB0, controller, value.min(127))
                    }
                    // 折叠路径已在 `Player` 里完成；本适配器声明不支持 per-note 音量。
                    HostEvent::NoteVolume { .. } => continue,
                };
                let bytes = [status, data1, data2];
                let offset = i64::from(e.frame).min(last).max(0);
                // `Block::call` 是安全方法；`midi` 是我们自己 `Block_copy` 住的。
                midi.call((
                    AUEventSampleTimeImmediate + offset,
                    0,
                    bytes.len() as isize,
                    NonNull::new(bytes.as_ptr() as *mut u8).unwrap(),
                ));
            }
        }

        // 输出缓冲区列表（非交错立体声）。
        let mut abl = OutputBuffers {
            number_buffers: ENGINE_CHANNELS as u32,
            buffers: [
                AudioBuffer {
                    mNumberChannels: 1,
                    mDataByteSize: (frames * std::mem::size_of::<f32>()) as u32,
                    mData: self.out_buf[0].as_mut_ptr() as *mut c_void,
                },
                AudioBuffer {
                    mNumberChannels: 1,
                    mDataByteSize: (frames * std::mem::size_of::<f32>()) as u32,
                    mData: self.out_buf[1].as_mut_ptr() as *mut c_void,
                },
            ],
        };
        let mut timestamp: AudioTimeStamp = unsafe { std::mem::zeroed() };
        timestamp.mSampleTime = self.sample_position as f64;
        timestamp.mFlags = AudioTimeStampFlags::SampleTimeValid;
        let mut flags = AudioUnitRenderActionFlags(0);
        let pull: AURenderPullInputBlock = if inputs > 0 {
            RcBlock::as_ptr(&handles.pull)
        } else {
            std::ptr::null_mut()
        };

        // `Block::call` 是安全方法；`render` 是我们自己 `Block_copy` 住的，实例被上面的
        // try_lock 钉住不会释放，缓冲区/时间戳都是本帧内的合法对象。
        let status = handles.render.call((
            NonNull::from(&mut flags),
            NonNull::from(&mut timestamp),
            frames as AUAudioFrameCount,
            0,
            NonNull::new(&mut abl as *mut OutputBuffers as *mut AudioBufferList).unwrap(),
            pull,
        ));
        if status != NO_ERR {
            // 实时线程：不能打日志，静音即可。
            for ch in audio.iter_mut() {
                ch[..frames].fill(0.0);
            }
            return;
        }
        for (dst, src) in audio.iter_mut().zip(self.out_buf.iter()) {
            dst[..frames].copy_from_slice(&src[..frames]);
        }
        self.sample_position += frames as u64;

        // AUv3 界面里的改动走 `parameterTree` 的观察者，这里没有现成的推回通道，恒为空。
        self.pending.clear();
    }

    fn save_state(&self) -> Vec<u8> {
        let shared = Arc::clone(&self.shared);
        crate::dispatch::on_main_thread(move || save_state_here(&shared))
            .unwrap_or_else(|_| save_state_here(&self.shared))
    }

    fn load_state(&mut self, state: &[u8]) -> Result<(), PluginError> {
        let shared = Arc::clone(&self.shared);
        let moved = state.to_vec();
        match crate::dispatch::on_main_thread(move || load_state_here(&shared, &moved)) {
            Ok(result) => result,
            Err(_) => load_state_here(&self.shared, state),
        }
    }

    fn take_param_changes(&mut self) -> Vec<(ParamId, f32)> {
        std::mem::take(&mut self.pending)
    }
}

/// 2 个 `AudioBuffer` 的输出缓冲区列表（与 `AudioBufferList` 布局兼容的前缀）。
#[repr(C)]
struct OutputBuffers {
    number_buffers: u32,
    buffers: [AudioBuffer; ENGINE_CHANNELS],
}

// ---------------------------------------------------------------------------
// 编辑器句柄与窗口
// ---------------------------------------------------------------------------

/// AU 编辑器句柄：与音频线程共享同一个实例。
#[derive(Clone)]
pub struct AuEditorHandle {
    shared: Arc<Mutex<AuInstance>>,
}

impl AuEditorHandle {
    /// 该插件是否有界面。AUAudioUnit 基类总能给一个界面（自带或通用参数界面），所以恒为 true。
    pub fn has_editor(&self) -> bool {
        true
    }

    /// 读插件状态（重建播放器前抢救用）。与 [`Plugin::save_state`] 同样派发到主线程。
    pub fn save_state(&self) -> Vec<u8> {
        let shared = Arc::clone(&self.shared);
        crate::dispatch::on_main_thread(move || save_state_here(&shared)).unwrap_or_default()
    }

    /// 打开 AU 的原生界面。
    ///
    /// **必须在主 / UI 线程调用。** 第一次会向插件要视图控制器（异步，泵主线程 run loop 等它），
    /// 之后**复用**同一扇窗——`AUAudioUnit` 通常只肯给一次视图控制器，关掉窗口后再去要会得到
    /// nil（应用里就是「插件界面不存在，无法打开」）。
    pub fn open(&self, title: &str) -> Result<AuEditorWindow, HostError> {
        let mtm = MainThreadMarker::new()
            .ok_or_else(|| HostError::Editor("AU 原生界面必须在主线程打开".into()))?;

        // 已经建过：把原来那扇窗叫回前面。
        {
            let inst = self
                .shared
                .lock()
                .map_err(|_| HostError::Editor("AU 实例锁中毒".into()))?;
            if let Some(editor) = inst.editor.as_ref() {
                editor.window.makeKeyAndOrderFront(None);
                return Ok(AuEditorWindow { window: Retained::clone(&editor.window) });
            }
        }

        // 第一次：先取出实例并**放锁**再去要控制器——`request_view_controller` 会泵主线程
        // run loop，持锁等待可能把主线程上的其它代码卡死。
        let au = {
            let inst = self
                .shared
                .lock()
                .map_err(|_| HostError::Editor("AU 实例锁中毒".into()))?;
            Retained::clone(inst.au_ref())
        };
        let controller = request_view_controller(&au).ok_or(HostError::NoEditor)?;
        let view: Retained<NSView> = controller.view();
        let window = build_plugin_window(&view, title, mtm);

        // 缓存起来（关窗后不销毁），下次直接复用。
        let chosen = {
            let mut inst = self
                .shared
                .lock()
                .map_err(|_| HostError::Editor("AU 实例锁中毒".into()))?;
            match inst.editor.as_ref() {
                Some(existing) => Retained::clone(&existing.window),
                None => {
                    inst.editor = Some(EditorState {
                        _controller: controller,
                        window: Retained::clone(&window),
                    });
                    Retained::clone(&window)
                }
            }
        };
        chosen.makeKeyAndOrderFront(None);
        Ok(AuEditorWindow { window: chosen })
    }
}

/// 一个打开着的 AU 窗口。
///
/// 关闭只是**收起**（`orderOut`）：真正的界面对象缓存在 [`AuInstance`] 上，关掉就销毁会导致
/// 再也打不开（AU 只肯给一次视图控制器）。
pub struct AuEditorWindow {
    window: Retained<NSWindow>,
}

impl AuEditorWindow {
    /// 窗口是否还开着（用户自己关掉后 `NSWindow.isVisible` 会变 false）。
    pub fn is_open(&self) -> bool {
        self.window.isVisible()
    }

    /// 收起窗口（`Drop` 也会做）。
    pub fn close(self) {
        self.window.orderOut(None);
    }
}

impl Drop for AuEditorWindow {
    fn drop(&mut self) {
        // 只是收起；界面对象仍由实例持有，下次还能再打开。
        self.window.orderOut(None);
    }
}

/// 问插件要视图控制器。**必须在主线程调用。**
///
/// 这个方法在绑定里没生成（`AUAudioUnit_ViewController` 分类），只能自己发消息；回调是异步的，
/// 所以这里泵主线程 run loop 等它（最多 [`VIEW_CONTROLLER_TIMEOUT`]）。
fn request_view_controller(au: &AUAudioUnit) -> Option<Retained<NSViewController>> {
    use std::time::{Duration, Instant};

    const VIEW_CONTROLLER_TIMEOUT: Duration = Duration::from_secs(3);

    // SAFETY: 先问插件有没有实现这个可选方法。
    let responds: Bool =
        unsafe { msg_send![au, respondsToSelector: sel!(requestViewControllerWithCompletionHandler:)] };
    if !responds.as_bool() {
        return None;
    }

    let (tx, rx) = mpsc::channel::<Option<Retained<NSViewController>>>();
    let block = RcBlock::new(move |view_controller: *mut NSViewController| {
        // 回调参数是 +0 的，`Retained::retain` 把它钉住再送回去。
        // SAFETY: 指针要么为 null，要么是合法的 `NSViewController`。
        let owned = unsafe { Retained::retain(view_controller) };
        let _ = tx.send(owned);
    });
    // SAFETY: `au` 合法；`block` 是签名匹配的 Objective-C block。
    let _: () = unsafe { msg_send![au, requestViewControllerWithCompletionHandler: &*block] };

    let deadline = Instant::now() + VIEW_CONTROLLER_TIMEOUT;
    loop {
        if let Ok(result) = rx.try_recv() {
            return result;
        }
        if Instant::now() >= deadline {
            return None;
        }
        // 泵一小段主线程 run loop，让回调有机会跑（它是投递到主线程队列的）。
        pump_main_run_loop(0.01);
    }
}

/// 泵一小步主线程 run loop。**必须在主线程调用。**
///
/// `AUAudioUnit` 的异步回调（视图控制器、异步实例化）都投在主队列上；不等它跑，回调就永远
/// 不会来。
fn pump_main_run_loop(seconds: f64) {
    let date = NSDate::dateWithTimeIntervalSinceNow(seconds);
    // SAFETY: 主线程 run loop 上跑一小步，到点就返回。
    unsafe { NSRunLoop::mainRunLoop().runMode_beforeDate(NSDefaultRunLoopMode, &date) };
}

/// 用插件给的视图建一扇宿主窗口（不显示，显示由调用方决定）。
fn build_plugin_window(view: &NSView, title: &str, mtm: MainThreadMarker) -> Retained<NSWindow> {
    // SAFETY: `view.frame` 是标准 NSView 属性。
    let frame: NSRect = unsafe { msg_send![view, frame] };
    let style = NSWindowStyleMask::Titled
        | NSWindowStyleMask::Closable
        | NSWindowStyleMask::Miniaturizable;
    // SAFETY: 直接用插件视图的尺寸开窗；视图由控制器持有。
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            frame,
            style,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    window.setTitle(&NSString::from_str(title));
    // 关键：`initWithContentRect:...` 建出来的窗口默认 `isReleasedWhenClosed = YES`——用户点红灯
    // 关窗时 AppKit 会替我们 `release` 一次，等我们自己的 `Retained` 再释放时就是**过度释放**，
    // 崩在关闭动画的 `_NSWindowTransformAnimation dealloc` / `_Block_release` 里。
    // 现代（非手动 retain/release）用法一律显式关掉它。
    // SAFETY: 标准 NSWindow 属性。
    unsafe { window.setReleasedWhenClosed(false) };
    // 关窗动画（`_NSWindowTransformAnimation`）会在主线程异步收尾，而编辑器 ticker 80ms 后就可能
    // 把窗口收起/复用——撞上动画就崩。插件窗口不需要这层动画，直接关掉。
    window.setAnimationBehavior(NSWindowAnimationBehavior::None);
    window.setContentView(Some(view));
    // 一些插件的界面尺寸在 `viewDidLoad` 之后才定下来，这里再对齐一次。
    window.setContentSize(frame.size);
    window
}

// ---------------------------------------------------------------------------
// 统一入口
// ---------------------------------------------------------------------------

/// 按 id 加载一个 AU（含编辑器句柄）。
pub fn load(id: &str) -> Result<(Box<dyn Plugin>, Option<crate::editor::EditorHandle>), HostError> {
    let plugin = AuHostedPlugin::from_loaded(id.to_string())?;
    let handle = plugin.editor_handle();
    Ok((
        Box::new(plugin),
        Some(crate::editor::EditorHandle::Au(handle)),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fourcc_roundtrips_and_pads() {
        assert_eq!(fourcc("aufx"), u32::from_be_bytes(*b"aufx"));
        // 不足 4 位补空格，转回文本会去掉尾部空格（`dls ` → `dls`）。
        assert_eq!(fourcc_str(fourcc("dls")), "dls");
        assert_eq!(fourcc_str(fourcc("aumu")), "aumu");
    }

    #[test]
    fn au_id_roundtrips() {
        let desc = AuDesc::from_id("au:aufx:lmtr:appl").unwrap();
        assert_eq!(desc.type_, fourcc("aufx"));
        assert_eq!(desc.subtype, fourcc("lmtr"));
        assert_eq!(desc.manufacturer, fourcc("appl"));
        assert_eq!(desc.to_id(), "au:aufx:lmtr:appl");
        assert_eq!(desc.kind(), PluginKind::Effect);

        let inst = AuDesc::from_id("au:aumu:dls:appl").unwrap();
        assert_eq!(inst.kind(), PluginKind::Instrument);
        assert!(inst.accepts_midi());
        assert_eq!(inst.to_id(), "au:aumu:dls:appl");
    }

    #[test]
    fn bad_au_id_is_rejected() {
        assert!(AuDesc::from_id("au:aufx:lmtr").is_err());
        assert!(AuDesc::from_id("vst3:123").is_err());
    }

    #[test]
    fn only_hostable_types_pass() {
        assert!(is_hostable(fourcc("aufx")));
        assert!(is_hostable(fourcc("aumu")));
        assert!(is_hostable(fourcc("aumf")));
        assert!(!is_hostable(fourcc("auou")));
        assert!(!is_hostable(fourcc("aufc")));
    }

    #[test]
    fn vendor_comes_from_the_display_name_prefix() {
        assert_eq!(vendor_of("Apple: AUPeakLimiter", fourcc("appl")), "Apple");
        assert_eq!(vendor_of("Acme: Reverb 9000", fourcc("acme")), "Acme");
        // 没有前缀时回退到 manufacturer。
        assert_eq!(vendor_of("Reverb 9000", fourcc("appl")), "Apple");
        assert_eq!(vendor_of("Reverb 9000", fourcc("acme")), "acme");
    }

    #[test]
    fn reads_components_from_a_plist_dictionary() {
        // 用真实 Apple plist 的结构：一个 bundle 里多条 AU + 一条应被过滤的 output unit。
        let text = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>AudioComponents</key>
    <array>
        <dict>
            <key>type</key><string>aufx</string>
            <key>subtype</key><string>lmtr</string>
            <key>manufacturer</key><string>appl</string>
            <key>name</key><string>Apple: AUPeakLimiter</string>
            <key>description</key><string>peak limiter</string>
        </dict>
        <dict>
            <key>type</key><string>aumu</string>
            <key>subtype</key><string>dls </string>
            <key>manufacturer</key><string>appl</string>
            <key>name</key><string>Apple: DLSMusicDevice</string>
        </dict>
        <dict>
            <key>type</key><string>auou</string>
            <key>subtype</key><string>def </string>
            <key>manufacturer</key><string>appl</string>
            <key>name</key><string>Apple: DefaultOutputUnit</string>
        </dict>
    </array>
</dict>
</plist>"#;
        let value = plist::Value::from_reader(std::io::Cursor::new(text.as_bytes())).unwrap();
        let components = value
            .as_dictionary()
            .and_then(|d| d.get("AudioComponents"))
            .and_then(|v| v.as_array())
            .unwrap();
        assert_eq!(components.len(), 3);

        let descs: Vec<AuDesc> = components
            .iter()
            .filter_map(|i| i.as_dictionary())
            .filter_map(|e| {
                let get = |k: &str| e.get(k).and_then(|v| v.as_string());
                Some(AuDesc {
                    type_: fourcc(get("type")?),
                    subtype: fourcc(get("subtype")?),
                    manufacturer: fourcc(get("manufacturer")?),
                })
            })
            .collect();
        let hostable: Vec<AuDesc> = descs.into_iter().filter(|d| is_hostable(d.type_)).collect();
        assert_eq!(hostable.len(), 2, "output unit 应被过滤掉");
        assert_eq!(hostable[0].to_id(), "au:aufx:lmtr:appl");
        assert_eq!(hostable[1].kind(), PluginKind::Instrument);
    }

    /// 回归：`AUAudioUnit.renderBlock` / `scheduleMIDIEventBlock` 必须被 `Block_copy` 住。
    ///
    /// 应用里配置（`prepare`）发生在主线程的**一次派发**里；那块任务一结束，主线程的
    /// autorelease pool 就会收掉借来的 block。这里用一个「每个任务都开/收 autorelease pool」的
    /// 假主线程复现同样的环境，再从**另一个线程**渲染——不 `Block_copy` 的话会直接崩在
    /// `Block::call`（应用里就是这样闪退的）。
    #[test]
    fn render_blocks_survive_the_main_thread_autorelease_pool() {
        use std::sync::mpsc;
        use std::sync::Once;

        struct PoolingMainThread(mpsc::Sender<Box<dyn FnOnce() + Send + 'static>>);
        impl crate::MainThreadDispatch for PoolingMainThread {
            fn dispatch(&self, task: Box<dyn FnOnce() + Send + 'static>) -> bool {
                self.0.send(task).is_ok()
            }
        }

        static INSTALLED: Once = Once::new();
        INSTALLED.call_once(|| {
            let (tx, rx) = mpsc::channel::<Box<dyn FnOnce() + Send + 'static>>();
            std::thread::Builder::new()
                .name("au-fake-main".into())
                .spawn(move || {
                    crate::mark_main_thread();
                    while let Ok(task) = rx.recv() {
                        // 与真实主线程一致：每个任务一个 autorelease pool，跑完就收。
                        objc2::rc::autoreleasepool(|_| task());
                    }
                })
                .expect("起假主线程");
            crate::set_main_thread_dispatch(Arc::new(PoolingMainThread(tx)));
        });

        let mut limiter = AuHostedPlugin::from_loaded("au:aufx:lmtr:appl".into())
            .expect("加载 AUPeakLimiter");
        // `prepare` 会派发到假主线程执行（配置 + 取 render block）。
        limiter.prepare(48_000.0, 128);
        assert!(limiter.handles.is_some(), "限幅器应当配置成功");

        // 读一次状态：`+dataWithPropertyList:...` 是 **+0（autoreleased）** 工厂方法，误当 +1
        // 接管会多释放一次——假主线程的 autorelease pool 一收就会崩在 `objc_release`。
        let state = limiter.save_state();
        assert!(!state.is_empty(), "限幅器应当能存状态");

        // 从另一个线程渲染（模拟音频回调线程）。
        let mut left = vec![0.5f32; 128];
        let mut right = vec![-0.5f32; 128];
        let peak = std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    let ctx = ProcessContext {
                        sample_rate: 48_000.0,
                        block_size: 128,
                        tempo: 120.0,
                        playing: true,
                    };
                    limiter.process(&ctx, &mut [&mut left, &mut right], &[]);
                    left.iter()
                        .chain(right.iter())
                        .fold(0f32, |m, v| m.max(v.abs()))
                })
                .join()
                .expect("渲染线程不该崩")
        });
        assert!(peak > 0.01, "限幅器必须让信号通过（峰值 {peak}）");
    }
}
