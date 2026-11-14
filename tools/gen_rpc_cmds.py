import pathlib, re

ROOT = pathlib.Path(__file__).resolve().parent.parent
SRC = ROOT / 'crates/daemon/src'
NAMES = [l.strip() for l in (ROOT / 'tools/rpc_cmds.txt').read_text().splitlines() if l.strip()]

rows = []
seen = set()
# `commands.rs` 是需要优先认的「界面命令层」：同名函数在别的模块里也有（内部实现），
# 这里要的是能直接被 Tauri 调用的那个。
FILES = [SRC / 'commands.rs'] + [p for p in sorted(SRC.rglob('*.rs')) if p.name != 'commands.rs']
for p in FILES:
    text = p.read_text()
    for m in re.finditer(r'pub (async )?fn (\w+)\s*\(([^;]*?)\)\s*(->[^{]+)?\{', text, re.S):
        name = m.group(2)
        if name not in NAMES or name in seen:
            continue
        seen.add(name)
        rows.append([
            name,
            str(p.relative_to(ROOT)),
            'ASYNC' if m.group(1) else 'sync',
            ' '.join(m.group(3).split()),
            ' '.join((m.group(4) or '').split()),
        ])
MANUAL = {'web_access_info', 'web_access_set_lan'}
missing = [n for n in NAMES if n not in seen and n not in MANUAL]
if missing:
    raise SystemExit('找不到实现：' + ', '.join(missing))

UNSUPPORTED = {
 'open_plugin_window': '原生插件窗口',
 'open_effect_plugin_window': '原生插件窗口',
 'open_synth_window': '原生合成器窗口',
 'open_effect_window': '原生效果器窗口',
 'close_plugin_window': '原生插件窗口',
 'close_effect_plugin_window': '原生插件窗口',
 'plugin_manager_open': '打开外部浏览器',
 'web_access_info': '桌面端网络设置',
 'web_access_set_lan': '桌面端网络设置',
 'cli_status': '命令行安装',
 'install_cli': '命令行安装',
 'uninstall_cli': '命令行安装',
}

SKIP_STATE = {'AgentRuntime','AgentConfigRuntime','McpRuntime'}

IMPORTS = """use std::collections::HashMap;
use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde_json::Value;

use ai_daw_engine::model::HarmonyBar;
use ai_daw_plugin_api::MidiEvent;

use crate::ai::MasterArgs;
use crate::manager::{
    BackendPreference, InstallRequest, ModelSourcePreference, ProxyMode, StoreRuntime,
};
use crate::singing::SingArgs;
use crate::svc::SwapArgs;
use crate::state::AppState;
#[allow(unused_imports)] // 桌面端需要 `Manager` 才有 `app.state()`；无头端是固有方法
use crate::tauri_api::{AppHandle, Manager};
use crate::voices::VoiceSaveArgs;
use crate::yue_engine::ComposeArgs;
use crate::yue_lyrics::LyricsArgs;
use crate::yue_transcribe::TranscribeArgs;

use super::rpc::RpcError;
"""

HEADER = '''//! 浏览器端命令桥：直接调用按 Tauri 签名写的实现。
//!
//! 网页端与桌面端必须执行**同一份**实现（`commands::` / `manager::` / `ai::` …），否则
//! "桌面能用、浏览器不能用"会永远修不完。这些函数是按 Tauri 的约定签名的——第一个参数
//! 是 `AppHandle`，状态参数是 `State<'_, T>`——所以在无头进程里怎么拿到它们是个问题：
//!
//! * `AppHandle`：桌面端由外壳经 [`super::ServerState::app`] 传进来；无头进程用空壳
//!   （`tauri_api::rpc_app_handle()`），空壳的 `state()` / `emit()` 接的是全局托管表与
//!   事件总线（见 `tauri_api`）。
//! * `State<'_, T>`：`app.state::<T>()` 在两种构建里同形，桌面端从 `app.manage()` 取，
//!   无头进程从托管表取。
//!
//! 参数名按 Tauri 的约定：前端传 camelCase，Rust 侧是 snake_case，[`take`] 两者都认。
//!
//! **本文件由 `tools/gen_rpc_cmds.py` 生成**，手工改动请先改生成器。

'''

def snake_to_camel(s):
    head, *rest = s.split('_')
    return head + ''.join(p.title() for p in rest)

def split_params(s):
    out, depth, cur = [], 0, ''
    for ch in s:
        if ch == '<':
            depth += 1
        elif ch == '>':
            depth -= 1
        if ch == ',' and depth == 0:
            out.append(cur)
            cur = ''
        else:
            cur += ch
    if cur.strip():
        out.append(cur)
    return [x.strip() for x in out if x.strip()]

def gen(name, params, ret, is_async):
    args = []
    pre = []
    call = []
    for p in split_params(params):
        if ':' not in p:
            continue
        pname, ptype = [x.strip() for x in p.split(':', 1)]
        if ptype.endswith('AppHandle'):
            call.append('app.clone()')
        elif 'State<' in ptype:
            inner = ptype.split("State<'_,", 1)[1].strip()[:-1].strip()
            call.append(f'app.state::<{inner}>()')
        else:
            pre.append(f'        let {pname.lstrip("_")}: {ptype} = take(&args, "{pname.lstrip("_")}")?;')
            call.append(pname.lstrip('_'))
    out = f'        let out = crate::{MODULE[name]}::{name}({", ".join(call)});'
    ret_is_result = ret.replace(' ', '').startswith('->Result<')
    if is_async:
        out = f'        let out = crate::{MODULE[name]}::{name}({", ".join(call)}).await;'
    body = '\n'.join(pre)
    fin = 'value(out)' if ret_is_result else 'plain(out)'
    body = body or ''
    uses_app = any(a.startswith('app') for a in call)
    app_name = 'app' if uses_app else '_app'
    args_name = 'args' if pre else '_args'
    return f'    "{name}" => |{app_name}, {args_name}| {{\n{body}{out}\n        {fin}\n    }},\n'

MODULE = {}
for name, path, kind, params, ret in rows:
    parts = path.split('/')
    base = parts[-1].replace('.rs', '')
    mod = parts[-2] if base == 'mod' else base
    MODULE[name] = mod

sync_arms, async_arms, unsup = [], [], []
SKIP_GENERATED = {'render_preview'}

for name, path, kind, params, ret in rows:
    if name in SKIP_GENERATED:
        continue
    if name in UNSUPPORTED:
        unsup.append((name, UNSUPPORTED[name]))
        continue
    st = re.search(r"State<'_,\s*([^>]+)>", params)
    if st and st.group(1) in SKIP_STATE:
        unsup.append((name, '后台运行时（浏览器/容器里没有启动）'))
        continue
    if name == 'app_ready':
        sync_arms.append('    "app_ready" => |_app, _args| {\n        // 首帧就绪：浏览器自己控制渲染时机，这里只回一句"收到了"。\n        plain(())\n    },\n')
        continue
    arm = gen(name, params, ret, kind == 'ASYNC')
    if kind == 'ASYNC':
        arm = re.sub(r'=> \|_?app, _?args\| \{', '=> {', arm, count=1)
        arm = arm.replace('let out = ', 'let out = ')
    (async_arms if kind == 'ASYNC' else sync_arms).append(arm)

MANUAL_UNSUPPORTED = {
    "web_access_info": "桌面端网络设置",
    "web_access_set_lan": "桌面端网络设置",
}
unsup.extend(MANUAL_UNSUPPORTED.items())
unsup.sort()
unsup_arms = ''.join(
    f'    "{n}" => |_app, _args| unsupported("{n}", "{why}"),\n' for n, why in unsup
)

ASYNC_NAMES = [a.split('"')[1] for a in async_arms]
async_names = [a.split('"')[1] for a in async_arms]
all_names = sorted(
    set([a.split('"')[1] for a in sync_arms] + async_names + [n for n, _ in unsup])
    # 由 `server/mod.rs` 手写（二进制回传，不进 JSON 表）。
    | {"render_preview"}
)

COMMON = '''
/// 前端按 Tauri 约定传 camelCase，Rust 参数是 snake_case：两个名字都试。
fn camel(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut upper = false;
    for ch in name.chars() {
        if ch == '_' {
            upper = true;
        } else if upper {
            out.extend(ch.to_uppercase());
            upper = false;
        } else {
            out.push(ch);
        }
    }
    out
}

/// 取一个参数。缺省按 `null` 处理，于是 `Option<T>` 可以省略、必填项会得到明确的报错。
pub(crate) fn take<T: DeserializeOwned>(args: &Value, name: &str) -> Result<T, RpcError> {
    let value = args
        .get(name)
        .or_else(|| args.get(&camel(name)))
        .cloned()
        .unwrap_or(Value::Null);
    serde_json::from_value(value).map_err(|err| RpcError::BadArgs(format!("{name}: {err}")))
}

/// 成功结果 → JSON。
fn plain<T: serde::Serialize>(value: T) -> Result<Value, RpcError> {
    serde_json::to_value(value).map_err(|err| RpcError::Failed(err.to_string()))
}

/// `Result<T, String>` → JSON（错误走 500，与桌面端抛出的字符串一致）。
fn value<T: serde::Serialize>(value: Result<T, String>) -> Result<Value, RpcError> {
    plain(value.map_err(RpcError::Failed)?)
}

/// 只有桌面端能做到的事：浏览器里给出明确原因，而不是一句 `unknown command`。
fn unsupported(name: &str, why: &str) -> Result<Value, RpcError> {
    Err(RpcError::Unsupported(format!(
        "{name}：{why}只在桌面端可用；浏览器模式下请用页面内的对应操作"
    )))
}

'''

doc = HEADER + IMPORTS + COMMON + '''
/// 同步命令表；`None` = 这条命令不在这里。
pub fn call(app: &AppHandle, cmd: &str, args: Value) -> Option<Result<Value, RpcError>> {
    let handler: fn(&AppHandle, Value) -> Result<Value, RpcError> = match cmd {
''' + ''.join(sync_arms) + unsup_arms + '''        _ => return None,
    };
    Some(handler(app, args))
}

/// 异步命令名（要 await 的那批）。
const ASYNC_CMDS: &[&str] = &[
''' + ''.join(f'    "{n}",\n' for n in async_names) + '''];

/// 异步命令表（打开工程、渲染、重扫插件这类要 await 的）。
async fn run_async(app: &AppHandle, cmd: &str, args: Value) -> Result<Value, RpcError> {
    match cmd {
''' + ''.join(async_arms) + '''        _ => Err(RpcError::Unknown(cmd.to_string())),
    }
}

/// 全部命令名（`/commands` 自述用）。
pub const NAMES: &[&str] = &[
''' + ''.join(f'    "{n}",\n' for n in all_names) + '''];

/// 异步命令入口；`None` = 这条命令不在这里。
pub async fn call_async(
    app: &AppHandle,
    cmd: &str,
    args: Value,
) -> Option<Result<Value, RpcError>> {
    if !ASYNC_CMDS.contains(&cmd) {
        return None;
    }
    Some(run_async(app, cmd, args).await)
}
'''
(ROOT / 'crates/daemon/src/server/cmds.rs').write_text(doc)
print('sync', len(sync_arms), 'async', len(async_arms), 'unsupported', len(unsup))
