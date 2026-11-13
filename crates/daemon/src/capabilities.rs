//! **能力预检**：逐项报告本地 AI 功能现在能不能用、缺什么、怎么补、不能用时改走哪条路。
//!
//! 为什么要有它：`sing_song` / `swap_voice` / `compose_song` 这类调用一旦真的跑起来，代价是
//! 几十秒到几分钟（下载、加载权重、推理）。缺权重时让它们在深处报错，等于让 AI 白等一场。
//! 所以这里把「前置条件」摊平成一张表：AI 先调 `get_capabilities`，再决定要不要碰某个功能；
//! 各生成工具在动手前也会用 [`gap`] 做同样的检查，缺东西就**正常返回**一段结构化说明
//! （`ok:false` + 缺什么 + 修复工具 + 替代方案），而不是开一个注定失败的重任务。
//!
//! 判定口径与「管理器 → 模型 / 运行时」一致：模型是否**整组就位**、运行时是否**已装**。
//! 内置 DSP 兜底的功能（转 MIDI / 降噪 / 母带 / MIDI 导出）恒为可用，只在缺模型时标 `degraded`。

use serde_json::{json, Value};
use tauri::AppHandle;

use crate::models::ModelStatus;
use crate::runtimes::RuntimeStatus;

/// 一个功能的前置条件说明。
struct FeatureSpec {
    /// 稳定标识（与工具名 / 模型 catalog 的 `feature` 对齐）。
    key: &'static str,
    /// 展示名。
    title: &'static str,
    /// 需要哪一组模型（任一档就位即可）；`None` = 不依赖模型。
    model_feature: Option<&'static str>,
    /// 需要哪一组运行时（任一档就位即可）；`None` = 不需要额外运行时。
    runtime_group: Option<&'static str>,
    /// 是否内置 DSP 兜底（缺模型也能跑，只是质量降级）。
    dsp_fallback: bool,
    /// 不可用（或降级）时推荐的替代做法。
    fallback: &'static str,
}

const FEATURES: &[FeatureSpec] = &[
    FeatureSpec {
        key: "compose",
        title: "AI 写歌（YuE2 / ACE-Step）",
        model_feature: Some("compose"),
        runtime_group: Some("audiocpp"),
        dsp_fallback: false,
        fallback: "改用「自己写 MIDI + 自主配器 / 捏音色」：set_tempo → add_track → add_notes → \
                   find_instrument / describe_plugin / set_params 选音色与调参数 → set_effect* 加效果。",
    },
    FeatureSpec {
        key: "score",
        title: "录音转谱（SheetSage2 · 翻唱）",
        model_feature: Some("cover"),
        runtime_group: Some("audiocpp"),
        dsp_fallback: false,
        fallback: "改用 transcribe_audio（Basic Pitch）拿到音符，再用 add_notes 手工铺成旋律；\
                   或者跳过翻唱，直接 compose_song 从头写。",
    },
    FeatureSpec {
        key: "lyrics",
        title: "抓歌词（ASR · 翻唱）",
        model_feature: Some("lyrics"),
        runtime_group: Some("audiocpp"),
        dsp_fallback: false,
        fallback: "改用 transcribe_score（SheetSage2）拿到旋律，词自己写；或直接照原歌词手打——\
                   翻唱本来要求词与旋律逐行对得上。",
    },
    FeatureSpec {
        key: "sing",
        title: "歌声合成（SoulX-Singer）",
        model_feature: Some("sing"),
        runtime_group: None,
        dsp_fallback: false,
        fallback: "改用内置采样乐器/合成器写人声旋律（或用已导入的干声做 transcribe_audio 拿旋律）。",
    },
    FeatureSpec {
        key: "voice_swap",
        title: "音色替换（SVC）",
        model_feature: Some("voice_swap"),
        runtime_group: None,
        dsp_fallback: false,
        fallback: "改为「换采样乐器 / 用 sing_song 重新合成（若可用）」；或先用 separate_stems 取人声再手工处理。",
    },
    FeatureSpec {
        key: "separate",
        title: "音源分离（HTDemucs）",
        model_feature: Some("separate"),
        runtime_group: None,
        dsp_fallback: false,
        fallback: "改用 transcribe_audio（Basic Pitch / 内置 DSP）拿旋律，再按听感手工拆轨。",
    },
    FeatureSpec {
        key: "detect_tempo",
        title: "节拍检测（Beat This!）",
        model_feature: Some("detect_tempo"),
        runtime_group: None,
        dsp_fallback: false,
        fallback: "手动 set_tempo / set_time_signature，用 transport_seek 试听对齐重拍。",
    },
    FeatureSpec {
        key: "to_midi",
        title: "音频转 MIDI",
        model_feature: Some("to_midi"),
        runtime_group: None,
        dsp_fallback: true,
        fallback: "Basic Pitch 不在位时会自动走内置 DSP（音高检测），可以用，只是精度低一些。",
    },
    FeatureSpec {
        key: "enhance",
        title: "降噪 / 去混响",
        model_feature: Some("enhance"),
        runtime_group: None,
        dsp_fallback: true,
        fallback: "LocalVQE 不在位时会自动走内置 DSP（只在接近底噪处衰减），可以用。",
    },
    FeatureSpec {
        key: "master",
        title: "母带处理",
        model_feature: None,
        runtime_group: None,
        dsp_fallback: true,
        fallback: "纯 DSP，不需要模型或运行时，恒可用。",
    },
    FeatureSpec {
        key: "export_midi",
        title: "导出 MIDI",
        model_feature: None,
        runtime_group: None,
        dsp_fallback: true,
        fallback: "纯工程数据导出，不需要模型或运行时，恒可用。",
    },
];

/// 全部功能的可用性报告（`get_capabilities` 的返回体）。
pub(crate) fn report(app: &AppHandle) -> Value {
    let models = crate::models::statuses(app);
    let runtimes = crate::runtimes::statuses(app);
    let features: Vec<Value> = FEATURES
        .iter()
        .map(|spec| feature_json(spec, &models, &runtimes))
        .collect();
    let available = features
        .iter()
        .filter(|item| item["available"] == json!(true))
        .count();
    let compose_ready = features
        .iter()
        .find(|item| item["feature"] == json!("compose"))
        .is_some_and(|item| item["available"] == json!(true));
    // 把「该走哪条路线」直接写进返回体：这比让 AI 自己从能力表里推更稳，
    // 也保证 MCP 与 skills 的口径一致（写歌可用 → 初稿+分离；否则 → 自写 MIDI+配器）。
    let recommended = if compose_ready {
        json!({
            "route": "ai_draft",
            "title": "AI 写歌初稿 + 音源分离",
            "steps": [
                "compose_song(style, lyrics, duration) → 新音频轨",
                "separate_stems(track=初稿轨, model=htdemucs-6s) → 鼓 / 贝斯 / 其他 / 人声（+吉他 / 钢琴）",
                "按需 swap_voice（换人声音色）/ transcribe_audio（转可编辑 MIDI）/ enhance_audio（清人声）",
                "set_gain / set_pan / set_sends / set_automation → analyze_project → master_project(as_track=true) / export_wav / export_midi",
            ],
            "note": "初稿是音频：要改音符先 transcribe_audio；分离与换声各自也可能缺模型，先看 features[].available。",
        })
    } else {
        json!({
            "route": "manual_midi",
            "title": "自己写 MIDI + 自主配器 / 捏音色",
            "steps": [
                "风格卡（genres / theory / rhythm）→ set_tempo / set_time_signature / set_project_harmony / set_project_sections",
                "add_track → find_instrument / list_plugins → set_track_instrument → set_track_meta",
                "list_synth_presets → preview_synth → set_params（点名音色时）",
                "add_clip → add_notes（批量）→ quantize_notes / apply_swing / humanize_notes",
                "analyze_harmony / analyze_track_overlap → 混音 → master_project / export_wav / export_midi",
            ],
            "note": "写歌不可用（缺 audio.cpp 引擎或 YuE2 权重）。用户若明确想要 AI 初稿，先说明缺什么、多大，同意后再 install_runtime / download_model。",
        })
    };
    json!({
        "ok": true,
        "available": available,
        "total": FEATURES.len(),
        "features": features,
        "recommended_workflow": recommended,
        "backend": crate::backends::ai_backend_info(app.clone()),
        "hint": "先看 features[].available / degraded 与 recommended_workflow：不可用的功能不要直接调用（会白等）；\
                 先向用户说明缺什么、多大，用户同意后再按 fix 里的工具装，或者直接走 fallback。",
    })
}

/// 某个功能的前置条件缺口；可用时返回 `None`。
///
/// 生成类工具在动手前调用它，得到的就是「正常返回给 AI 的说明」，而不是一个跑了几分钟才炸的错误。
pub(crate) fn gap(app: &AppHandle, feature: &str) -> Option<Value> {
    let report = report(app);
    let item = report["features"]
        .as_array()?
        .iter()
        .find(|entry| entry["feature"] == json!(feature))?;
    if item["available"] == json!(true) {
        return None;
    }
    Some(unavailable_body(feature, item))
}

/// 指定某个模型是否就位；缺了返回同一套「不可用说明」（用于工具显式传了 model 的场景）。
pub(crate) fn gap_model(app: &AppHandle, feature: &str, model_id: &str) -> Option<Value> {
    let models = crate::models::statuses(app);
    let Some(model) = models.iter().find(|entry| entry.id == model_id) else {
        return None;
    };
    if model.installed || model.bundled {
        return None;
    }
    let block = model_requirement(model);
    let fallback = FEATURES
        .iter()
        .find(|spec| spec.key == feature)
        .map(|spec| spec.fallback)
        .unwrap_or("换一个可用模型，或改走内置方案。");
    Some(json!({
        "ok": false,
        "status": "unavailable",
        "feature": feature,
        "missing": [block.clone()],
        "fix": [block["fix"].clone()],
        "fallback": fallback,
        "message": format!(
            "指定的模型「{}」（{}）没有安装。{}",
            model.name,
            human_bytes(model.approx_bytes),
            fallback
        ),
        "hint": "先征得用户同意再按 fix 下载；不要重试当前调用。",
    }))
}

/// 单个功能的状态（内部与 `get_capabilities` 共用）。
fn feature_json(spec: &FeatureSpec, models: &[ModelStatus], runtimes: &[RuntimeStatus]) -> Value {
    let mut requires: Vec<Value> = Vec::new();
    let mut missing: Vec<Value> = Vec::new();
    let mut fixes: Vec<Value> = Vec::new();

    let mut model_ready = true;
    if let Some(feature) = spec.model_feature {
        let group: Vec<&ModelStatus> = models
            .iter()
            .filter(|model| model.feature == feature)
            .collect();
        let any = group.iter().any(|model| model.installed || model.bundled);
        model_ready = any || spec.dsp_fallback;
        for model in &group {
            let block = model_requirement(model);
            if block["fix"].is_object() {
                fixes.push(block["fix"].clone());
            }
            if !model.installed && !model.bundled {
                missing.push(block.clone());
            }
            requires.push(block);
        }
    }

    let mut runtime_ready = true;
    if let Some(group_name) = spec.runtime_group {
        let group: Vec<&RuntimeStatus> = runtimes
            .iter()
            .filter(|runtime| runtime.group == group_name)
            .collect();
        let any = group
            .iter()
            .any(|runtime| runtime.installed || runtime.bundled);
        runtime_ready = any;
        for runtime in &group {
            let block = runtime_requirement(runtime);
            if block["fix"].is_object() {
                fixes.push(block["fix"].clone());
            }
            if !runtime.installed && !runtime.bundled {
                missing.push(block.clone());
            }
            requires.push(block);
        }
    }

    let available = model_ready && runtime_ready;
    // 「有兜底但模型没在位」：可用，但要提醒质量降级。
    let degraded = available
        && spec.model_feature.is_some()
        && missing.iter().any(|item| item["kind"] == json!("model"))
        && spec.dsp_fallback;

    json!({
        "feature": spec.key,
        "title": spec.title,
        "available": available,
        "degraded": degraded,
        "requires": requires,
        "missing": missing,
        "fix": fixes,
        "fallback": spec.fallback,
    })
}

fn model_requirement(model: &ModelStatus) -> Value {
    let need = !model.installed && !model.bundled;
    let fix = if need && model.downloadable {
        json!({
            "tool": "download_model",
            "args": { "id": model.id, "confirm": true },
            "bytes": model.approx_bytes,
            "requires_user_consent": true,
        })
    } else {
        Value::Null
    };
    json!({
        "kind": "model",
        "id": model.id,
        "name": model.name,
        "license": model.license,
        "installed": model.installed,
        "bundled": model.bundled,
        "downloadable": model.downloadable,
        "bytes": if need { model.approx_bytes } else { 0 },
        "fix": fix,
    })
}

fn runtime_requirement(runtime: &RuntimeStatus) -> Value {
    let need = !runtime.installed && !runtime.bundled;
    let fix = if need {
        json!({
            "tool": "install_runtime",
            "args": { "id": runtime.id, "confirm": true },
            "bytes": runtime.bytes,
            "requires_user_consent": true,
        })
    } else {
        Value::Null
    };
    json!({
        "kind": "runtime",
        "id": runtime.id,
        "name": runtime.label,
        "installed": runtime.installed,
        "bundled": runtime.bundled,
        "downloadable": !runtime.bundled,
        "bytes": if need { runtime.bytes } else { 0 },
        "fix": fix,
    })
}

/// 把 `get_capabilities` 里的一条功能记录翻成「不可用」的返回体。
fn unavailable_body(feature: &str, item: &Value) -> Value {
    let title = item["title"].as_str().unwrap_or(feature);
    let missing_items = item["missing"].as_array().cloned().unwrap_or_default();
    let names: Vec<String> = missing_items
        .iter()
        .filter_map(|entry| {
            let name = entry["name"].as_str()?;
            let size = entry["bytes"].as_u64().unwrap_or(0);
            Some(if size > 0 {
                format!("{name}（{}）", human_bytes(size))
            } else {
                name.to_string()
            })
        })
        .collect();
    let model_count = missing_items
        .iter()
        .filter(|entry| entry["kind"] == json!("model"))
        .count();
    let runtime_count = missing_items
        .iter()
        .filter(|entry| entry["kind"] == json!("runtime"))
        .count();
    // 「任一档就够」的组只按每类最小的一档累加：模型 + 运行时（写歌两样都要），
    // 不把互斥的几档体积加在一起骗人。
    let min_of = |kind: &str| {
        missing_items
            .iter()
            .filter(|entry| entry["kind"] == json!(kind))
            .filter_map(|entry| entry["bytes"].as_u64())
            .filter(|bytes| *bytes > 0)
            .min()
            .unwrap_or(0)
    };
    let estimated_bytes = min_of("model") + min_of("runtime");
    let message = if names.is_empty() {
        format!("{title} 当前不可用。")
    } else if names.len() <= 3 {
        // 少数几项（通常就是两档模型）：直接点名，AI 好照抄给用户。
        format!("{title} 当前不可用：缺少 {}。", names.join("、"))
    } else {
        let parts = match (model_count, runtime_count) {
            (models, 0) => format!("{models} 档模型"),
            (0, runtimes) => format!("{runtimes} 档运行时"),
            (models, runtimes) => format!("{models} 档模型 + {runtimes} 档运行时"),
        };
        format!(
            "{title} 当前不可用：缺少 {parts}（每类任选一档，最小组合约 {}）。",
            human_bytes(estimated_bytes)
        )
    };
    json!({
        "ok": false,
        "status": "unavailable",
        "feature": feature,
        "title": title,
        "missing": item["missing"].clone(),
        "fix": item["fix"].clone(),
        "fallback": item["fallback"].clone(),
        "estimated_bytes": estimated_bytes,
        "message": format!("{message}{}", item["fallback"].as_str().unwrap_or("")),
        "hint": "不要重试当前调用。先向用户说明缺什么、多大，用户同意后再按 fix 安装；\
                 或者直接按 fallback 换一条路。",
    })
}

fn human_bytes(bytes: u64) -> String {
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    let value = bytes as f64;
    if value >= GIB {
        format!("{:.1} GB", value / GIB)
    } else if value >= MIB {
        format!("{:.0} MB", value / MIB)
    } else {
        format!("{bytes} B")
    }
}
