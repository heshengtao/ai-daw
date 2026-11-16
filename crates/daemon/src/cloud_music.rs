//! **云端音乐生成**：Google Lyria、ElevenLabs Music、Mureka、Treblo（原 Sonauto）。
//!
//! 设计目标与 `agent` 的配置层一致：一家供应商一个条目，配置落
//! `<配置目录>/cloud-music.json`；密钥明文保存（与 `ai.json` / `mcp.json` 同一套惯例，
//! 项目没有 keychain 集成）。
//!
//! 这里只接**官方公开文档里明确支持人声 / 歌词**的 API：
//! - Google Lyria 3.5：`generateContent`，返回 inlineData 音频 + 歌词文本；
//! - ElevenLabs Music v2.5：`POST /v1/music`，支持 prompt 与 composition plan（逐段歌词）；
//! - Mureka：`POST /v1/song/generate`（歌词）或 `/v1/song/easy-generate`（提示词），异步轮询；
//! - Treblo：`POST /v1/generations/v3`，异步轮询；
//! - 阿里云百炼 Fun-Music：`POST /api/v1/services/audio/music/generation`；
//! - 火山引擎豆包音乐：`GenSongForTime` / `GenBGMForTime`，AK/SK 签名 + QuerySong 轮询；
//! - 腾讯云 MPS：`CreateAigcAudioTask` / `DescribeAigcAudioTask`，TC3 签名。
//!
//! **没有接** Suno / Udio（没有公开自助 API），也没有接 Stability / Mubert / Soundraw /
//! Beatoven（官方 API 是纯音乐 / 配乐，不支持人声与歌词）。
//!
//! 音频落 `<配置目录>/ai-music/`，随后解码、按当前速度换算成拍、作为一条新音频轨加进工程。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ai_daw_engine::command::Command as EngineCommand;
use ai_daw_engine::model::AudioSource;
use base64::Engine as _;
use chrono::Utc;
use sha2::{Digest, Sha256};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::state::AppState;
#[allow(unused_imports)] // 桌面端用 `Emitter` 发事件；无头端 `emit` 是固有方法。
use crate::tauri_api::{AppHandle, Emitter, State};

/// 进度事件名（桌面走 Tauri event，浏览器走 SSE）。
pub const EVENT_CLOUD_MUSIC: &str = "cloud-music-progress";

const HTTP_TIMEOUT: Duration = Duration::from_secs(600);
const POLL_INTERVAL: Duration = Duration::from_secs(3);
const MAX_POLLS: usize = 240;

// ---------------------------------------------------------------------------
// 配置
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CloudMusicKind {
    #[default]
    Lyria,
    Elevenlabs,
    Mureka,
    Treblo,
    Alibaba,
    Volcengine,
    Tencent,
}

struct Preset {
    name: &'static str,
    base_url: &'static str,
    model: &'static str,
    max_seconds: u32,
    docs_url: &'static str,
    note: &'static str,
}

impl CloudMusicKind {
    fn preset(self) -> Preset {
        match self {
            CloudMusicKind::Lyria => Preset {
                name: "Google Lyria 3.5",
                base_url: "https://generativelanguage.googleapis.com",
                model: "lyria-3.5",
                max_seconds: 180,
                docs_url: "https://ai.google.dev/gemini-api/docs/music-generation",
                note: "Gemini API；44.1 kHz 立体声，支持人声与歌词；按首计费。",
            },
            CloudMusicKind::Elevenlabs => Preset {
                name: "ElevenLabs Music",
                base_url: "https://api.elevenlabs.io",
                model: "music_v2_5",
                max_seconds: 600,
                docs_url: "https://elevenlabs.io/docs/api-reference/music/compose",
                note: "官方 Music API；支持 composition plan 逐段歌词；按分钟计费。",
            },
            CloudMusicKind::Mureka => Preset {
                name: "Mureka",
                base_url: "https://api.mureka.cn",
                model: "auto",
                max_seconds: 300,
                docs_url: "https://platform.mureka.ai/docs/",
                note: "昆仑万维；支持歌词 / 提示词生歌与纯音乐；异步任务轮询。",
            },
            CloudMusicKind::Treblo => Preset {
                name: "Treblo (Sonauto)",
                base_url: "https://api.treblo.com",
                model: "",
                max_seconds: 285,
                docs_url: "https://treblo.com/developers/docs",
                note: "原 Sonauto；支持歌词、纯音乐、局部替换；异步任务轮询。",
            },
            CloudMusicKind::Alibaba => Preset {
                name: "阿里云百炼 Fun-Music",
                base_url: "https://YOUR_WORKSPACE.cn-beijing.maas.aliyuncs.com",
                model: "fun-music-v1",
                max_seconds: 300,
                docs_url: "https://help.aliyun.com/zh/model-studio/fun-music-api",
                note: "百炼 Fun-Music；中英文人声、歌词、纯音乐；目前邀测，Base URL 要带业务空间 ID。",
            },
            CloudMusicKind::Volcengine => Preset {
                name: "火山引擎 豆包音乐",
                base_url: "https://open.volcengineapi.com",
                model: "v5.0",
                max_seconds: 240,
                docs_url: "https://www.volcengine.com/docs/84992/1967910",
                note: "豆包音乐；人声歌曲 / 纯音乐；AK/SK 签名，异步 QuerySong 轮询。",
            },
            CloudMusicKind::Tencent => Preset {
                name: "腾讯云 MPS（MiniMax）",
                base_url: "https://mps.tencentcloudapi.com",
                model: "MinimaxMusic",
                max_seconds: 300,
                docs_url: "https://cloud.tencent.com/document/product/862/133520",
                note: "MPS 聚合 MiniMax Music；歌曲 / 纯音乐；SecretId + SecretKey 的 TC3 签名。",
            },
        }
    }
}

/// 供应商预设（前端「新增服务」时用，避免两端各写一份默认值）。
#[derive(Debug, Clone, Serialize)]
pub struct CloudMusicPreset {
    pub kind: CloudMusicKind,
    pub name: &'static str,
    pub group: &'static str,
    pub base_url: &'static str,
    pub model: &'static str,
    pub max_seconds: u32,
    pub docs_url: &'static str,
    pub note: &'static str,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CloudMusicProvider {
    pub id: String,
    pub name: String,
    pub kind: CloudMusicKind,
    pub enabled: bool,
    /// 例如 `https://api.mureka.cn`；不带末尾斜杠。
    pub base_url: String,
    pub api_key: String,
    /// 火山 / 腾讯这类双密钥服务的 Secret（AK/SK 或 SecretId/SecretKey）。
    #[serde(default)]
    pub api_secret: String,
    /// 模型名；空 = 用该 kind 的默认模型。
    pub model: String,
}

impl Default for CloudMusicProvider {
    fn default() -> Self {
        let preset = CloudMusicKind::default().preset();
        Self {
            id: String::new(),
            name: preset.name.to_string(),
            kind: CloudMusicKind::default(),
            enabled: true,
            base_url: preset.base_url.to_string(),
            api_key: String::new(),
            api_secret: String::new(),
            model: preset.model.to_string(),
        }
    }
}

impl CloudMusicProvider {
    fn preset(&self) -> Preset {
        self.kind.preset()
    }

    fn effective_model(&self) -> String {
        let model = self.model.trim();
        if model.is_empty() {
            self.preset().model.to_string()
        } else {
            model.to_string()
        }
    }

    fn effective_base(&self) -> String {
        let base = self.base_url.trim().trim_end_matches('/');
        if base.is_empty() {
            self.preset().base_url.to_string()
        } else {
            base.to_string()
        }
    }

    fn max_seconds(&self) -> u32 {
        self.preset().max_seconds
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CloudMusicConfig {
    pub providers: Vec<CloudMusicProvider>,
}

/// 配置读写器（挂在 [`AppState`] 上，由启动时注入文件位置）。
pub struct CloudMusicRuntime {
    inner: Mutex<CloudMusicConfig>,
    path: Mutex<Option<PathBuf>>,
}

impl Default for CloudMusicRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl CloudMusicRuntime {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(CloudMusicConfig::default()),
            path: Mutex::new(None),
        }
    }

    /// 从配置文件载入；逐条反序列化，单条坏数据不拖垮整张表。
    pub fn load(&self, path: Option<PathBuf>) {
        let config = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .map(|value| CloudMusicConfig {
                providers: value
                    .get("providers")
                    .and_then(Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| {
                                serde_json::from_value::<CloudMusicProvider>(item.clone()).ok()
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            })
            .unwrap_or_default();
        let mut config = config;
        for provider in &mut config.providers {
            normalize(provider);
        }
        *self.inner.lock().unwrap() = config;
        *self.path.lock().unwrap() = path;
    }

    pub fn get(&self) -> CloudMusicConfig {
        self.inner.lock().unwrap().clone()
    }

    pub fn set(&self, providers: Vec<CloudMusicProvider>) -> CloudMusicConfig {
        let mut providers = providers;
        for provider in &mut providers {
            normalize(provider);
        }
        let config = CloudMusicConfig { providers };
        *self.inner.lock().unwrap() = config.clone();
        self.save(&config);
        config
    }

    pub fn provider(&self, id: &str) -> Option<CloudMusicProvider> {
        self.get().providers.into_iter().find(|p| p.id == id)
    }

    fn save(&self, config: &CloudMusicConfig) {
        let path = self.path.lock().unwrap().clone();
        let Some(path) = path else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match serde_json::to_string_pretty(config) {
            Ok(text) => {
                if let Err(err) = std::fs::write(&path, text) {
                    eprintln!("[cloud-music] 写入配置失败：{err}");
                }
            }
            Err(err) => eprintln!("[cloud-music] 序列化配置失败：{err}"),
        }
    }
}

fn normalize(provider: &mut CloudMusicProvider) {
    if provider.id.trim().is_empty() {
        provider.id = new_id();
    }
    let preset = provider.kind.preset();
    // 从默认 kind（Lyria）复制过来的字段，在用户切换 kind 后不该继续跟着走：
    // 只在「为空」或「仍是默认 kind 的预设值」时补成当前 kind 的预设。
    let default_preset = CloudMusicKind::default().preset();
    let inherited = provider.kind != CloudMusicKind::default();
    if provider.name.trim().is_empty() || (inherited && provider.name.trim() == default_preset.name)
    {
        provider.name = preset.name.to_string();
    }
    provider.name = provider.name.trim().to_string();
    provider.base_url = provider.base_url.trim().trim_end_matches('/').to_string();
    if provider.base_url.is_empty() || (inherited && provider.base_url == default_preset.base_url) {
        provider.base_url = preset.base_url.to_string();
    }
    provider.api_key = provider.api_key.trim().to_string();
    provider.api_secret = provider.api_secret.trim().to_string();
    if provider.model.trim().is_empty()
        || (inherited && provider.model.trim() == default_preset.model)
    {
        provider.model = preset.model.to_string();
    }
    provider.model = provider.model.trim().to_string();
}

fn new_id() -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("cm-{millis}-{}", SEQ.fetch_add(1, Ordering::Relaxed))
}

// ---------------------------------------------------------------------------
// 任务状态
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct CloudMusicProgress {
    /// `preparing` / `running` / `done` / `error` / `cancelled`。
    pub state: &'static str,
    pub message: Option<String>,
    /// 0..1；轮询阶段是估算值。
    pub progress: f32,
    pub elapsed: f64,
}

struct TaskFlags {
    running: AtomicBool,
    cancel: AtomicBool,
}

fn flags() -> &'static TaskFlags {
    static FLAGS: OnceLock<TaskFlags> = OnceLock::new();
    FLAGS.get_or_init(|| TaskFlags {
        running: AtomicBool::new(false),
        cancel: AtomicBool::new(false),
    })
}

fn begin_task() -> Result<(), String> {
    if flags()
        .running
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("已有音乐生成任务在跑，等它结束或先取消".into());
    }
    flags().cancel.store(false, Ordering::SeqCst);
    Ok(())
}

fn end_task() {
    flags().running.store(false, Ordering::SeqCst);
}

fn cancelled() -> bool {
    flags().cancel.load(Ordering::SeqCst)
}

fn emit(
    app: &AppHandle,
    state: &'static str,
    message: impl Into<String>,
    progress: f32,
    elapsed: f64,
) {
    let _ = app.emit(
        EVENT_CLOUD_MUSIC,
        CloudMusicProgress {
            state,
            message: Some(message.into()),
            progress: progress.clamp(0.0, 1.0),
            elapsed,
        },
    );
}

// ---------------------------------------------------------------------------
// 命令
// ---------------------------------------------------------------------------

#[cfg_attr(feature = "desktop", tauri::command)]
pub fn cloud_music_config(state: State<'_, std::sync::Arc<AppState>>) -> CloudMusicConfig {
    state.cloud_music.get()
}

#[cfg_attr(feature = "desktop", tauri::command)]
pub fn cloud_music_presets() -> Vec<CloudMusicPreset> {
    [
        CloudMusicKind::Lyria,
        CloudMusicKind::Elevenlabs,
        CloudMusicKind::Mureka,
        CloudMusicKind::Treblo,
        CloudMusicKind::Alibaba,
        CloudMusicKind::Volcengine,
        CloudMusicKind::Tencent,
    ]
    .into_iter()
    .map(|kind| {
        let preset = kind.preset();
        CloudMusicPreset {
            kind,
            name: preset.name,
            group: "official",
            base_url: preset.base_url,
            model: preset.model,
            max_seconds: preset.max_seconds,
            docs_url: preset.docs_url,
            note: preset.note,
        }
    })
    .collect()
}

#[cfg_attr(feature = "desktop", tauri::command)]
pub fn cloud_music_set_config(
    state: State<'_, std::sync::Arc<AppState>>,
    providers: Vec<CloudMusicProvider>,
) -> CloudMusicConfig {
    state.cloud_music.set(providers)
}

#[cfg_attr(feature = "desktop", tauri::command)]
pub fn cloud_music_busy() -> bool {
    flags().running.load(Ordering::SeqCst)
}

#[cfg_attr(feature = "desktop", tauri::command)]
pub fn cloud_music_cancel() {
    flags().cancel.store(true, Ordering::SeqCst);
}

/// 生成请求（一次只跑一个）。
#[derive(Debug, Clone, Deserialize)]
pub struct CloudMusicRequest {
    pub provider_id: String,
    pub prompt: String,
    #[serde(default)]
    pub lyrics: String,
    #[serde(default = "default_duration")]
    pub duration_seconds: u32,
    #[serde(default)]
    pub instrumental: bool,
}

fn default_duration() -> u32 {
    120
}

#[derive(Debug, Clone, Serialize)]
pub struct CloudMusicResult {
    pub path: String,
    pub duration: f64,
    pub provider: String,
    pub model: String,
    pub lyrics: Option<String>,
}

/// 设置页「测试」：只做一次轻量 GET，确认地址 / Key / 网络通。
#[cfg_attr(feature = "desktop", tauri::command)]
pub async fn cloud_music_test(provider: CloudMusicProvider) -> Result<String, String> {
    let mut provider = provider;
    normalize(&mut provider);
    let base = provider.effective_base();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败：{e}"))?;
    let (url, auth) = match provider.kind {
        CloudMusicKind::Lyria => (
            format!(
                "{base}/v1beta/models/{}",
                url_encode(&provider.effective_model())
            ),
            Auth::Google(provider.api_key.clone()),
        ),
        CloudMusicKind::Elevenlabs => (
            format!("{base}/v1/user"),
            Auth::Eleven(provider.api_key.clone()),
        ),
        CloudMusicKind::Mureka => (
            format!("{base}/v1/account/billing"),
            Auth::Bearer(provider.api_key.clone()),
        ),
        CloudMusicKind::Treblo => (
            format!("{base}/v1/credits/balance"),
            Auth::Bearer(provider.api_key.clone()),
        ),
        CloudMusicKind::Alibaba => (
            format!("{base}/api/v1/models"),
            Auth::Bearer(provider.api_key.clone()),
        ),
        CloudMusicKind::Volcengine | CloudMusicKind::Tencent => {
            // 这两家是双密钥签名，下面单独处理。
            return cloud_music_test_signed(&provider).await;
        }
    };
    let request = match auth {
        Auth::Google(key) => client.get(&url).header("x-goog-api-key", key),
        Auth::Eleven(key) => client.get(&url).header("xi-api-key", key),
        Auth::Bearer(key) => client.get(&url).bearer_auth(key),
    };
    let response = request.send().await.map_err(|e| format!("连接失败：{e}"))?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if status.is_success() {
        Ok(format!("连接正常（HTTP {status}）"))
    } else if provider.kind == CloudMusicKind::Alibaba && (status.as_u16() == 404 || status.as_u16() == 405) {
        Ok(format!(
            "地址可达（HTTP {status}）；Fun-Music 没有轻量探针，实际生成时会验证 Key"
        ))
    } else {
        Err(format!(
            "连接失败（HTTP {status}）：{}",
            truncate(&body, 300)
        ))
    }
}

enum Auth {
    Google(String),
    Eleven(String),
    Bearer(String),
}

/// 生成一首歌，落盘并加入工程。
#[cfg_attr(feature = "desktop", tauri::command)]
pub async fn cloud_music_generate(
    app: AppHandle,
    state: State<'_, std::sync::Arc<AppState>>,
    request: CloudMusicRequest,
) -> Result<CloudMusicResult, String> {
    let state = std::sync::Arc::clone(state.inner());
    let provider = state
        .cloud_music
        .provider(&request.provider_id)
        .ok_or_else(|| format!("未找到音乐服务：{}", request.provider_id))?;
    if !provider.enabled {
        return Err(format!("{} 已停用", provider.name));
    }
    if request.prompt.trim().is_empty() && request.lyrics.trim().is_empty() {
        return Err("描述或歌词至少填一项".into());
    }
    let duration = request.duration_seconds.clamp(10, provider.max_seconds());
    let request = CloudMusicRequest {
        provider_id: request.provider_id,
        prompt: request.prompt.trim().to_string(),
        lyrics: request.lyrics.trim().to_string(),
        duration_seconds: duration,
        instrumental: request.instrumental,
    };

    begin_task()?;
    let started = Instant::now();
    emit(&app, "preparing", "准备请求…", 0.02, 0.0);

    let result = generate_inner(&app, &state, &provider, &request, started).await;
    end_task();

    match result {
        Ok(done) => {
            emit(
                &app,
                "done",
                "已加入工程",
                1.0,
                started.elapsed().as_secs_f64(),
            );
            Ok(done)
        }
        Err(error) => {
            if error.contains("已取消") || cancelled() {
                emit(
                    &app,
                    "cancelled",
                    "已取消",
                    0.0,
                    started.elapsed().as_secs_f64(),
                );
                Err("已取消".into())
            } else {
                emit(
                    &app,
                    "error",
                    error.clone(),
                    0.0,
                    started.elapsed().as_secs_f64(),
                );
                Err(error)
            }
        }
    }
}

async fn generate_inner(
    app: &AppHandle,
    state: &std::sync::Arc<AppState>,
    provider: &CloudMusicProvider,
    request: &CloudMusicRequest,
    started: Instant,
) -> Result<CloudMusicResult, String> {
    let client = reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败：{e}"))?;
    emit(
        app,
        "running",
        format!("正在调用 {}…", provider.name),
        0.08,
        started.elapsed().as_secs_f64(),
    );

    let generated = match provider.kind {
        CloudMusicKind::Lyria => generate_lyria(&client, provider, request, app, started).await?,
        CloudMusicKind::Elevenlabs => {
            generate_elevenlabs(&client, provider, request, app, started).await?
        }
        CloudMusicKind::Mureka => generate_mureka(&client, provider, request, app, started).await?,
        CloudMusicKind::Treblo => generate_treblo(&client, provider, request, app, started).await?,
        CloudMusicKind::Alibaba => generate_alibaba(&client, provider, request, app, started).await?,
        CloudMusicKind::Volcengine => {
            generate_volcengine(&client, provider, request, app, started).await?
        }
        CloudMusicKind::Tencent => generate_tencent(&client, provider, request, app, started).await?,
    };
    if cancelled() {
        return Err("已取消".into());
    }

    emit(
        app,
        "running",
        "保存并加入工程…",
        0.92,
        started.elapsed().as_secs_f64(),
    );
    let model = generated
        .model
        .clone()
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| provider.effective_model());
    save_and_import(
        app,
        state,
        &generated.bytes,
        &generated.ext,
        &provider.name,
        &model,
        generated.lyrics,
    )
}

struct Generated {
    bytes: Vec<u8>,
    ext: String,
    model: Option<String>,
    lyrics: Option<String>,
}

// ---------------------------------------------------------------------------
// Lyria 3.5
// ---------------------------------------------------------------------------

async fn generate_lyria(
    client: &reqwest::Client,
    provider: &CloudMusicProvider,
    request: &CloudMusicRequest,
    app: &AppHandle,
    started: Instant,
) -> Result<Generated, String> {
    let model = provider.effective_model();
    let url = format!(
        "{}/v1beta/models/{}:generateContent",
        provider.effective_base(),
        url_encode(&model)
    );
    let mut prompt = request.prompt.clone();
    if request.instrumental {
        prompt.push_str("\n\nInstrumental only. No vocals, no lyrics.");
    } else if !request.lyrics.is_empty() {
        prompt
            .push_str("\n\nSing the following lyrics exactly as written. Keep the section tags:\n");
        prompt.push_str(&request.lyrics);
    }
    if prompt.trim().is_empty() {
        prompt = "Create a complete song with vocals and instrumentation.".into();
    }
    let with_wav = json!({
        "contents": [{ "parts": [{ "text": prompt }] }],
        "generationConfig": {
            "responseModalities": ["AUDIO", "TEXT"],
            "responseFormat": { "audio": { "mimeType": "audio/wav" } }
        }
    });
    let response = client
        .post(&url)
        .header("x-goog-api-key", &provider.api_key)
        .json(&with_wav)
        .send()
        .await
        .map_err(|e| format!("Lyria 请求失败：{e}"))?;
    let status = response.status();
    if !status.is_success() {
        let first_error = response.text().await.unwrap_or_default();
        // 个别账号 / 模型不接受 `responseFormat.audio`：退回默认 MP3，不要因此整条失败。
        let without_wav = json!({
            "contents": [{ "parts": [{ "text": prompt }] }],
            "generationConfig": { "responseModalities": ["AUDIO", "TEXT"] }
        });
        let retry = client
            .post(&url)
            .header("x-goog-api-key", &provider.api_key)
            .json(&without_wav)
            .send()
            .await
            .map_err(|e| format!("Lyria 重试失败：{e}"))?;
        if !retry.status().is_success() {
            return Err(format!(
                "Lyria 失败（HTTP {status}）：{}",
                truncate(&first_error, 300)
            ));
        }
        let value = retry
            .json::<Value>()
            .await
            .map_err(|e| format!("Lyria 返回不是 JSON：{e}"))?;
        return parse_lyria(value, model, app, started);
    }
    let value = response
        .json::<Value>()
        .await
        .map_err(|e| format!("Lyria 返回不是 JSON：{e}"))?;
    parse_lyria(value, model, app, started)
}

fn parse_lyria(
    value: Value,
    model: String,
    app: &AppHandle,
    started: Instant,
) -> Result<Generated, String> {
    let parts = value
        .pointer("/candidates/0/content/parts")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("Lyria 没有返回音频：{}", truncate(&value.to_string(), 300)))?;
    let mut audio: Option<(Vec<u8>, String)> = None;
    let mut text = String::new();
    for part in parts {
        if let Some(inline) = part.get("inlineData").or_else(|| part.get("inline_data")) {
            let data = inline
                .get("data")
                .and_then(Value::as_str)
                .ok_or_else(|| "Lyria inlineData 缺少 data".to_string())?;
            let mime = inline
                .get("mimeType")
                .or_else(|| inline.get("mime_type"))
                .and_then(Value::as_str)
                .unwrap_or("audio/wav");
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|e| format!("Lyria 音频 base64 解码失败：{e}"))?;
            audio = Some((bytes, ext_from_mime(mime).to_string()));
        } else if let Some(t) = part.get("text").and_then(Value::as_str) {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(t);
        }
    }
    let (bytes, ext) = audio.ok_or_else(|| {
        format!(
            "Lyria 没有返回音频（可能被安全策略拦截）：{}",
            truncate(&text, 300)
        )
    })?;
    emit(
        app,
        "running",
        "音频已返回，准备落盘…",
        0.9,
        started.elapsed().as_secs_f64(),
    );
    Ok(Generated {
        bytes,
        ext,
        model: Some(model),
        lyrics: (!text.trim().is_empty()).then_some(text.trim().to_string()),
    })
}

// ---------------------------------------------------------------------------
// ElevenLabs Music
// ---------------------------------------------------------------------------

async fn generate_elevenlabs(
    client: &reqwest::Client,
    provider: &CloudMusicProvider,
    request: &CloudMusicRequest,
    app: &AppHandle,
    started: Instant,
) -> Result<Generated, String> {
    let model = provider.effective_model();
    let url = format!(
        "{}/v1/music?output_format=mp3_48000_192",
        provider.effective_base()
    );
    let duration_ms = request.duration_seconds as i64 * 1000;
    let wants_plan = !request.instrumental && !request.lyrics.is_empty();
    let body = if wants_plan {
        let styles: Vec<String> = request
            .prompt
            .split([',', '，', ';', '；'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .take(8)
            .map(ToString::to_string)
            .collect();
        let lines: Vec<String> = request
            .lyrics
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
            .collect();
        let style_json = if styles.is_empty() {
            json!([request.prompt])
        } else {
            json!(styles)
        };
        json!({
            // v2 / v2.5 的 CompositionPlan 只有 sections 一层；全局风格放 local styles 里，
            // 避免给 v2 发送它不认识的字段。
            "composition_plan": {
                "sections": [{
                    "section_name": "Song",
                    "duration_ms": duration_ms,
                    "lines": lines,
                    "positive_local_styles": style_json,
                    "negative_local_styles": []
                }]
            },
            "model_id": model
        })
    } else {
        json!({
            "prompt": request.prompt,
            "music_length_ms": duration_ms,
            "model_id": model,
            "force_instrumental": request.instrumental
        })
    };
    let response = client
        .post(&url)
        .header("xi-api-key", &provider.api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("ElevenLabs 请求失败：{e}"))?;
    let status = response.status();
    if !status.is_success() {
        // composition plan 在个别账号 / 模型上可能不可用：退回「歌词写进 prompt」的老路。
        let first_error = response.text().await.unwrap_or_default();
        if wants_plan {
            let fallback = json!({
                "prompt": format!("{}\n\nLyrics:\n{}", request.prompt, request.lyrics),
                "music_length_ms": duration_ms,
                "model_id": model,
                "force_instrumental": false
            });
            let retry = client
                .post(&url)
                .header("xi-api-key", &provider.api_key)
                .json(&fallback)
                .send()
                .await
                .map_err(|e| format!("ElevenLabs 重试失败：{e}"))?;
            if retry.status().is_success() {
                let bytes = retry
                    .bytes()
                    .await
                    .map_err(|e| format!("ElevenLabs 读取音频失败：{e}"))?
                    .to_vec();
                return Ok(Generated {
                    bytes,
                    ext: "mp3".into(),
                    model: Some(model),
                    lyrics: Some(request.lyrics.clone()),
                });
            }
        }
        return Err(format!(
            "ElevenLabs 失败（HTTP {status}）：{}",
            truncate(&first_error, 300)
        ));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("ElevenLabs 读取音频失败：{e}"))?
        .to_vec();
    emit(
        app,
        "running",
        "音频已返回，准备落盘…",
        0.9,
        started.elapsed().as_secs_f64(),
    );
    Ok(Generated {
        bytes,
        ext: "mp3".into(),
        model: Some(model),
        lyrics: None,
    })
}

// ---------------------------------------------------------------------------
// Mureka
// ---------------------------------------------------------------------------

async fn generate_mureka(
    client: &reqwest::Client,
    provider: &CloudMusicProvider,
    request: &CloudMusicRequest,
    app: &AppHandle,
    started: Instant,
) -> Result<Generated, String> {
    let base = provider.effective_base();
    let model = provider.effective_model();
    let (create_path, query_path) = if request.instrumental {
        ("/v1/instrumental/generate", "/v1/instrumental/query/")
    } else if request.lyrics.is_empty() {
        ("/v1/song/easy-generate", "/v1/song/query/")
    } else {
        ("/v1/song/generate", "/v1/song/query/")
    };
    let body = if request.instrumental || request.lyrics.is_empty() {
        json!({ "prompt": request.prompt, "model": model })
    } else {
        json!({
            "lyrics": request.lyrics,
            "prompt": request.prompt,
            "model": model
        })
    };
    let response = client
        .post(format!("{base}{create_path}"))
        .bearer_auth(&provider.api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Mureka 请求失败：{e}"))?;
    let value = json_response(response, "Mureka").await?;
    let task_id = value
        .get("id")
        .or_else(|| value.get("task_id"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            format!(
                "Mureka 没有返回任务 id：{}",
                truncate(&value.to_string(), 300)
            )
        })?;
    emit(
        app,
        "running",
        format!("Mureka 任务已提交：{task_id}"),
        0.15,
        started.elapsed().as_secs_f64(),
    );

    let mut url: Option<String> = None;
    for attempt in 0..MAX_POLLS {
        if cancelled() {
            return Err("已取消".into());
        }
        tokio::time::sleep(POLL_INTERVAL).await;
        let response = client
            .get(format!("{base}{query_path}{task_id}"))
            .bearer_auth(&provider.api_key)
            .send()
            .await
            .map_err(|e| format!("Mureka 查询失败：{e}"))?;
        let value = json_response(response, "Mureka 查询").await?;
        let status = value
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        let progress = 0.15 + 0.7 * (attempt as f32 / MAX_POLLS as f32);
        emit(
            app,
            "running",
            format!("Mureka 生成中…（{status}）"),
            progress,
            started.elapsed().as_secs_f64(),
        );
        if matches!(
            status.as_str(),
            "succeeded" | "success" | "completed" | "complete"
        ) {
            url = choice_url(&value);
            if url.is_none() {
                return Err(format!(
                    "Mureka 任务成功但没有音频地址：{}",
                    truncate(&value.to_string(), 400)
                ));
            }
            break;
        }
        if matches!(
            status.as_str(),
            "failed" | "error" | "cancelled" | "canceled" | "timeouted" | "timeout"
        ) {
            return Err(format!(
                "Mureka 生成失败：{}",
                truncate(&error_message(&value), 400)
            ));
        }
    }
    let url = url.ok_or_else(|| "Mureka 生成超时".to_string())?;
    let bytes = download(client, &url).await?;
    Ok(Generated {
        bytes,
        ext: ext_from_url(&url).unwrap_or("mp3").to_string(),
        model: Some(model),
        lyrics: Some(request.lyrics.clone()).filter(|s| !s.is_empty()),
    })
}

fn choice_url(value: &Value) -> Option<String> {
    let choices = value.get("choices").and_then(Value::as_array)?;
    for choice in choices {
        for key in ["url", "audio_url", "wav_url", "flac_url", "mp3_url"] {
            if let Some(url) = choice.get(key).and_then(Value::as_str) {
                if !url.trim().is_empty() {
                    return Some(url.to_string());
                }
            }
        }
    }
    None
}

fn string_value(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(ToString::to_string)
        .or_else(|| value.as_i64().map(|n| n.to_string()))
}

fn error_message(value: &Value) -> String {
    value
        .pointer("/error/message")
        .and_then(Value::as_str)
        .or_else(|| value.get("Message").and_then(Value::as_str))
        .or_else(|| value.get("message").and_then(Value::as_str))
        .or_else(|| value.pointer("/ResponseMetadata/Error/Message").and_then(Value::as_str))
        .or_else(|| value.get("error").and_then(Value::as_str))
        .unwrap_or("未知错误")
        .to_string()
}

// ---------------------------------------------------------------------------
// Treblo（原 Sonauto）
// ---------------------------------------------------------------------------

async fn generate_treblo(
    client: &reqwest::Client,
    provider: &CloudMusicProvider,
    request: &CloudMusicRequest,
    app: &AppHandle,
    started: Instant,
) -> Result<Generated, String> {
    let base = provider.effective_base();
    // Treblo 文档要求 length_range 两端都是 30 的倍数。
    let seconds = (request.duration_seconds / 30 * 30).clamp(30, 285);
    let low = if seconds <= 30 { 0 } else { 30 };
    let mut body = json!({
        "prompt": request.prompt,
        "instrumental": request.instrumental,
        "length_range": [low, seconds]
    });
    if !request.instrumental && !request.lyrics.is_empty() {
        body["lyrics"] = json!(request.lyrics);
    }
    let response = client
        .post(format!("{base}/v1/generations/v3"))
        .bearer_auth(&provider.api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Treblo 请求失败：{e}"))?;
    let value = json_response(response, "Treblo").await?;
    let task_id = value
        .get("task_id")
        .or_else(|| value.get("id"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            format!(
                "Treblo 没有返回任务 id：{}",
                truncate(&value.to_string(), 300)
            )
        })?;
    emit(
        app,
        "running",
        format!("Treblo 任务已提交：{task_id}"),
        0.15,
        started.elapsed().as_secs_f64(),
    );

    let mut song_url: Option<String> = None;
    for attempt in 0..MAX_POLLS {
        if cancelled() {
            return Err("已取消".into());
        }
        tokio::time::sleep(POLL_INTERVAL).await;
        let response = client
            .get(format!("{base}/v1/generations/status/{task_id}"))
            .bearer_auth(&provider.api_key)
            .send()
            .await
            .map_err(|e| format!("Treblo 状态查询失败：{e}"))?;
        // 状态接口有时回 JSON，有时回纯文本（`SUCCESS` / `FAILURE`）；两种都收。
        let status_value = {
            let response = ensure_ok(response, "Treblo 状态").await?;
            let text = response
                .text()
                .await
                .map_err(|e| format!("Treblo 状态读取失败：{e}"))?;
            serde_json::from_str::<Value>(&text)
                .unwrap_or_else(|_| Value::String(text.trim().into()))
        };
        let status = status_value
            .get("status")
            .and_then(Value::as_str)
            .or_else(|| status_value.as_str())
            .unwrap_or("")
            .to_ascii_uppercase();
        let progress = 0.15 + 0.7 * (attempt as f32 / MAX_POLLS as f32);
        emit(
            app,
            "running",
            format!("Treblo 生成中…（{status}）"),
            progress,
            started.elapsed().as_secs_f64(),
        );
        if status == "SUCCESS" {
            let response = client
                .get(format!("{base}/v1/generations/{task_id}"))
                .bearer_auth(&provider.api_key)
                .send()
                .await
                .map_err(|e| format!("Treblo 取结果失败：{e}"))?;
            let value = json_response(response, "Treblo 结果").await?;
            song_url = value
                .get("song_paths")
                .and_then(Value::as_array)
                .and_then(|paths| paths.first())
                .and_then(Value::as_str)
                .map(|s| {
                    if s.starts_with("http://") || s.starts_with("https://") {
                        s.to_string()
                    } else {
                        format!("{base}{}", s)
                    }
                });
            if song_url.is_none() {
                return Err(format!(
                    "Treblo 任务成功但没有音频地址：{}",
                    truncate(&value.to_string(), 400)
                ));
            }
            break;
        }
        if matches!(
            status.as_str(),
            "FAILURE" | "FAILED" | "ERROR" | "CANCELLED"
        ) {
            return Err(format!(
                "Treblo 生成失败：{}",
                truncate(&status_value.to_string(), 400)
            ));
        }
    }
    let url = song_url.ok_or_else(|| "Treblo 生成超时".to_string())?;
    let bytes = download(client, &url).await?;
    Ok(Generated {
        bytes,
        ext: ext_from_url(&url).unwrap_or("mp3").to_string(),
        model: None,
        lyrics: Some(request.lyrics.clone()).filter(|s| !s.is_empty()),
    })
}


// ---------------------------------------------------------------------------
// 阿里云百炼 Fun-Music
// ---------------------------------------------------------------------------

async fn generate_alibaba(
    client: &reqwest::Client,
    provider: &CloudMusicProvider,
    request: &CloudMusicRequest,
    app: &AppHandle,
    started: Instant,
) -> Result<Generated, String> {
    let model = provider.effective_model();
    let url = format!(
        "{}/api/v1/services/audio/music/generation",
        provider.effective_base()
    );
    let mut input = json!({
        "prompt": request.prompt,
        "is_instrumental": request.instrumental,
        "format": "wav"
    });
    if !request.instrumental && !request.lyrics.is_empty() {
        input["lyrics"] = json!(request.lyrics);
    }
    let body = json!({ "model": model, "input": input });
    let value = json_response(
        client
            .post(&url)
            .bearer_auth(&provider.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("阿里云 Fun-Music 请求失败：{e}"))?,
        "阿里云 Fun-Music",
    )
    .await?;
    let audio = value
        .pointer("/output/audio")
        .ok_or_else(|| format!("Fun-Music 没有返回音频：{}", truncate(&value.to_string(), 300)))?;
    let bytes = if let Some(url) = audio.get("url").and_then(Value::as_str).filter(|s| !s.trim().is_empty()) {
        download(client, url).await?
    } else if let Some(data) = audio.get("data").and_then(Value::as_str).filter(|s| !s.trim().is_empty()) {
        base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|e| format!("Fun-Music 音频 base64 解码失败：{e}"))?
    } else {
        return Err(format!(
            "Fun-Music 没有返回可用音频：{}",
            truncate(&value.to_string(), 300)
        ));
    };
    let lyrics = value
        .pointer("/output/extra_info/lyrics")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);
    emit(
        app,
        "running",
        "音频已返回，准备落盘…",
        0.9,
        started.elapsed().as_secs_f64(),
    );
    Ok(Generated {
        bytes,
        ext: "wav".into(),
        model: Some(model),
        lyrics,
    })
}

// ---------------------------------------------------------------------------
// 火山引擎 豆包音乐（AK/SK 签名）
// ---------------------------------------------------------------------------

fn sha256_hex(data: &[u8]) -> String {
    hex_encode(&Sha256::digest(data))
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}


fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut block = [0u8; 64];
    if key.len() > 64 {
        block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    let mut outer = Sha256::new();
    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for i in 0..64 {
        ipad[i] ^= block[i];
        opad[i] ^= block[i];
    }
    inner.update(ipad);
    inner.update(data);
    let inner_hash = inner.finalize();
    outer.update(opad);
    outer.update(inner_hash);
    outer.finalize().into()
}

fn volc_signing_key(secret_key: &str, date: &str) -> [u8; 32] {
    let k_date = hmac_sha256(secret_key.as_bytes(), date.as_bytes());
    let k_region = hmac_sha256(&k_date, b"cn-beijing");
    let k_service = hmac_sha256(&k_region, b"imagination");
    hmac_sha256(&k_service, b"request")
}

fn volc_sign(
    access_key: &str,
    secret_key: &str,
    action: &str,
    body: &[u8],
) -> Result<(String, String), String> {
    if access_key.trim().is_empty() || secret_key.trim().is_empty() {
        return Err("火山引擎需要 AccessKey ID 与 Secret Access Key".into());
    }
    let now = Utc::now();
    let x_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let date = now.format("%Y%m%d").to_string();
    let host = "open.volcengineapi.com";
    let query = format!("Action={action}&Version=2024-08-12");
    let canonical_headers = format!("host:{host}\nx-date:{x_date}\n");
    let signed_headers = "host;x-date";
    let canonical_request = format!(
        "POST\n/\n{query}\n{canonical_headers}\n{signed_headers}\n{}",
        sha256_hex(body)
    );
    let scope = format!("{date}/cn-beijing/imagination/request");
    let string_to_sign = format!(
        "HMAC-SHA256\n{x_date}\n{scope}\n{}",
        sha256_hex(canonical_request.as_bytes())
    );
    let k_date = hmac_sha256(secret_key.as_bytes(), date.as_bytes());
    // 火山的文档写的是「Hex 类型的密钥」：实际是把上一步的 hex 解码回字节再当 key。
    let k_region = hmac_sha256(&k_date, b"cn-beijing");
    eprintln!("kregion {}", hex_encode(&k_region));
    let k_service = hmac_sha256(&k_region, b"imagination");
    let k_signing = hmac_sha256(&k_service, b"request");
    let signature = hex_encode(&hmac_sha256(&k_signing, string_to_sign.as_bytes()));
    let authorization = format!(
        "HMAC-SHA256 Credential={access_key}/{scope}, SignedHeaders={signed_headers}, Signature={signature}"
    );
    Ok((authorization, x_date))
}

async fn volc_post_raw(
    client: &reqwest::Client,
    provider: &CloudMusicProvider,
    action: &str,
    body: Value,
) -> Result<Value, String> {
    let bytes = serde_json::to_vec(&body).map_err(|e| format!("火山引擎请求序列化失败：{e}"))?;
    let (authorization, x_date) = volc_sign(
        &provider.api_key,
        &provider.api_secret,
        action,
        &bytes,
    )?;
    let url = format!(
        "{}/?Action={action}&Version=2024-08-12",
        provider.effective_base()
    );
    let value = json_response(
        client
            .post(url)
            .header("X-Date", x_date)
            .header("Authorization", authorization)
            .header("Content-Type", "application/json")
            .body(bytes)
            .send()
            .await
            .map_err(|e| format!("火山引擎请求失败：{e}"))?,
        "火山引擎",
    )
    .await?;
    Ok(value)
}

async fn volc_post(
    client: &reqwest::Client,
    provider: &CloudMusicProvider,
    action: &str,
    body: Value,
) -> Result<Value, String> {
    let value = volc_post_raw(client, provider, action, body).await?;
    if value.get("Code").and_then(Value::as_i64).unwrap_or(0) != 0 {
        return Err(format!(
            "火山引擎失败：{}",
            truncate(&error_message(&value), 400)
        ));
    }
    Ok(value)
}

async fn generate_volcengine(
    client: &reqwest::Client,
    provider: &CloudMusicProvider,
    request: &CloudMusicRequest,
    app: &AppHandle,
    started: Instant,
) -> Result<Generated, String> {
    let model = provider.effective_model();
    let duration = request.duration_seconds.clamp(30, 240);
    let action = if request.instrumental {
        "GenBGMForTime"
    } else {
        "GenSongForTime"
    };
    let body = if request.instrumental {
        json!({
            "Text": request.prompt,
            "Duration": duration,
            "Version": model,
            "EnableInputRewrite": false,
            "AigcWatermark": false
        })
    } else {
        let mut body = json!({
            "Prompt": request.prompt,
            "ModelVersion": model,
            "Duration": duration,
            "VodFormat": "wav",
            "AigcWatermark": false
        });
        if !request.lyrics.is_empty() {
            body["Lyrics"] = json!(request.lyrics);
        }
        body
    };
    let value = volc_post(client, provider, action, body).await?;
    let task_id = value
        .pointer("/Result/TaskID")
        .and_then(string_value)
        .ok_or_else(|| format!("火山引擎没有返回 TaskID：{}", truncate(&value.to_string(), 300)))?;
    emit(
        app,
        "running",
        format!("火山引擎任务已提交：{task_id}"),
        0.15,
        started.elapsed().as_secs_f64(),
    );
    let mut audio_url: Option<String> = None;
    let mut lyrics: Option<String> = None;
    for attempt in 0..MAX_POLLS {
        if cancelled() {
            return Err("已取消".into());
        }
        tokio::time::sleep(POLL_INTERVAL).await;
        let value = volc_post(
            client,
            provider,
            "QuerySong",
            json!({ "TaskID": task_id }),
        )
        .await?;
        let status = value
            .pointer("/Result/Status")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        emit(
            app,
            "running",
            format!("火山引擎生成中…（status {status}）"),
            0.15 + 0.7 * (attempt as f32 / MAX_POLLS as f32),
            started.elapsed().as_secs_f64(),
        );
        if status == 2 {
            audio_url = value
                .pointer("/Result/SongDetail/AudioUrl")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            lyrics = value
                .pointer("/Result/SongDetail/Lyrics")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToString::to_string);
            if audio_url.is_none() {
                return Err(format!(
                    "火山引擎任务成功但没有音频地址：{}",
                    truncate(&value.to_string(), 400)
                ));
            }
            break;
        }
        if status == 3 {
            return Err(format!(
                "火山引擎生成失败：{}",
                truncate(&value.to_string(), 400)
            ));
        }
    }
    let url = audio_url.ok_or_else(|| "火山引擎生成超时".to_string())?;
    let bytes = download(client, &url).await?;
    Ok(Generated {
        bytes,
        ext: ext_from_url(&url).unwrap_or("wav").to_string(),
        model: Some(model),
        lyrics: lyrics.or_else(|| Some(request.lyrics.clone()).filter(|s| !s.is_empty())),
    })
}

// ---------------------------------------------------------------------------
// 腾讯云 MPS（TC3-HMAC-SHA256）
// ---------------------------------------------------------------------------

fn tencent_sign(
    secret_id: &str,
    secret_key: &str,
    action: &str,
    payload: &[u8],
) -> Result<(String, String), String> {
    if secret_id.trim().is_empty() || secret_key.trim().is_empty() {
        return Err("腾讯云需要 SecretId 与 SecretKey".into());
    }
    let now = Utc::now();
    let timestamp = now.timestamp().to_string();
    let date = now.format("%Y%m%d").to_string();
    let host = "mps.tencentcloudapi.com";
    let canonical_headers = format!(
        "content-type:application/json\nhost:{host}\nx-tc-action:{}\n",
        action.to_lowercase()
    );
    let signed_headers = "content-type;host;x-tc-action";
    let canonical_request = format!(
        "POST\n/\n\n{canonical_headers}\n{signed_headers}\n{}",
        sha256_hex(payload)
    );
    let scope = format!("{date}/mps/tc3_request");
    let string_to_sign = format!(
        "TC3-HMAC-SHA256\n{timestamp}\n{scope}\n{}",
        sha256_hex(canonical_request.as_bytes())
    );
    let k_date = hmac_sha256(format!("TC3{secret_key}").as_bytes(), date.as_bytes());
    let k_service = hmac_sha256(&k_date, b"mps");
    let k_signing = hmac_sha256(&k_service, b"tc3_request");
    let signature = hex_encode(&hmac_sha256(&k_signing, string_to_sign.as_bytes()));
    let authorization = format!(
        "TC3-HMAC-SHA256 Credential={secret_id}/{scope}, SignedHeaders={signed_headers}, Signature={signature}"
    );
    let _ = date;
    Ok((authorization, timestamp))
}

async fn tencent_post_raw(
    client: &reqwest::Client,
    provider: &CloudMusicProvider,
    action: &str,
    body: Value,
) -> Result<Value, String> {
    let bytes = serde_json::to_vec(&body).map_err(|e| format!("腾讯云请求序列化失败：{e}"))?;
    let (authorization, timestamp) =
        tencent_sign(&provider.api_key, &provider.api_secret, action, &bytes)?;
    let value = json_response(
        client
            .post(provider.effective_base())
            .header("Authorization", authorization)
            .header("Content-Type", "application/json")
            .header("Host", "mps.tencentcloudapi.com")
            .header("X-TC-Action", action)
            .header("X-TC-Version", "2019-06-12")
            .header("X-TC-Timestamp", timestamp)
            .header("X-TC-Region", "ap-guangzhou")
            .body(bytes)
            .send()
            .await
            .map_err(|e| format!("腾讯云请求失败：{e}"))?,
        "腾讯云",
    )
    .await?;
    Ok(value)
}

async fn tencent_post(
    client: &reqwest::Client,
    provider: &CloudMusicProvider,
    action: &str,
    body: Value,
) -> Result<Value, String> {
    let value = tencent_post_raw(client, provider, action, body).await?;
    if let Some(error) = value.pointer("/Response/Error") {
        return Err(format!(
            "腾讯云失败：{}",
            truncate(&error.to_string(), 400)
        ));
    }
    Ok(value)
}

async fn generate_tencent(
    client: &reqwest::Client,
    provider: &CloudMusicProvider,
    request: &CloudMusicRequest,
    app: &AppHandle,
    started: Instant,
) -> Result<Generated, String> {
    let model = provider.effective_model();
    let mut body = json!({
        "ModelName": model,
        "ModelVersion": "2.6",
        "SceneType": "music",
        "Prompt": request.prompt,
        "OutputAudioFormat": "wav"
    });
    if !request.instrumental && !request.lyrics.is_empty() {
        body["AdditionalParameters"] =
            json!(serde_json::to_string(&json!({ "lyrics": request.lyrics })).unwrap_or_default());
    }
    let value = tencent_post(client, provider, "CreateAigcAudioTask", body).await?;
    let task_id = value
        .pointer("/Response/TaskId")
        .and_then(string_value)
        .ok_or_else(|| format!("腾讯云没有返回 TaskId：{}", truncate(&value.to_string(), 300)))?;
    emit(
        app,
        "running",
        format!("腾讯云任务已提交：{task_id}"),
        0.15,
        started.elapsed().as_secs_f64(),
    );
    let mut audio_url: Option<String> = None;
    for attempt in 0..MAX_POLLS {
        if cancelled() {
            return Err("已取消".into());
        }
        tokio::time::sleep(POLL_INTERVAL).await;
        let value = tencent_post(
            client,
            provider,
            "DescribeAigcAudioTask",
            json!({ "TaskId": task_id }),
        )
        .await?;
        let status = value
            .pointer("/Response/Status")
            .and_then(Value::as_str)
            .unwrap_or("WAIT")
            .to_ascii_uppercase();
        emit(
            app,
            "running",
            format!("腾讯云生成中…（{status}）"),
            0.15 + 0.7 * (attempt as f32 / MAX_POLLS as f32),
            started.elapsed().as_secs_f64(),
        );
        if status == "DONE" {
            audio_url = value
                .pointer("/Response/AudioInfos/0/Url")
                .or_else(|| value.pointer("/Response/AudioInfos/0/AudioUrl"))
                .or_else(|| value.pointer("/Response/AudioUrl"))
                .and_then(Value::as_str)
                .map(ToString::to_string);
            if audio_url.is_none() {
                return Err(format!(
                    "腾讯云任务成功但没有音频地址：{}",
                    truncate(&value.to_string(), 400)
                ));
            }
            break;
        }
        if status == "FAIL" {
            return Err(format!(
                "腾讯云生成失败：{}",
                truncate(&value.to_string(), 400)
            ));
        }
    }
    let url = audio_url.ok_or_else(|| "腾讯云生成超时".to_string())?;
    let bytes = download(client, &url).await?;
    Ok(Generated {
        bytes,
        ext: ext_from_url(&url).unwrap_or("wav").to_string(),
        model: Some(model),
        lyrics: Some(request.lyrics.clone()).filter(|s| !s.is_empty()),
    })
}

async fn cloud_music_test_signed(provider: &CloudMusicProvider) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败：{e}"))?;
    match provider.kind {
        CloudMusicKind::Volcengine => {
            let body = json!({ "TaskID": "0" });
            let value = volc_post_raw(&client, provider, "QuerySong", body).await?;
            let code = value.get("Code").and_then(Value::as_i64).unwrap_or(0);
            if matches!(code, 100002 | 100003 | 100004 | 100005 | 100009) {
                return Err(format!(
                    "火山引擎鉴权失败：{}",
                    truncate(&error_message(&value), 300)
                ));
            }
            Ok(format!("签名已送达（QuerySong Code {code}，假 TaskID 报错属正常）"))
        }
        CloudMusicKind::Tencent => {
            let body = json!({ "TaskId": "0" });
            let value = tencent_post_raw(&client, provider, "DescribeAigcAudioTask", body).await?;
            if let Some(error) = value.pointer("/Response/Error") {
                let text = error.to_string();
                let lower = text.to_ascii_lowercase();
                if lower.contains("auth") || lower.contains("signature") || lower.contains("credential") {
                    return Err(format!("腾讯云鉴权失败：{}", truncate(&text, 300)));
                }
                return Ok(format!("签名已送达（假 TaskId 返回错误属正常）：{}", truncate(&text, 160)));
            }
            Ok("签名可用：DescribeAigcAudioTask 已送达".into())
        }
        _ => Err("不是双密钥服务".into()),
    }
}

// ---------------------------------------------------------------------------
// 落盘 + 入工程
// ---------------------------------------------------------------------------

fn save_and_import(
    app: &AppHandle,
    state: &std::sync::Arc<AppState>,
    bytes: &[u8],
    ext: &str,
    provider_name: &str,
    model: &str,
    lyrics: Option<String>,
) -> Result<CloudMusicResult, String> {
    let dir = crate::paths::config_dir(app)?.join("ai-music");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建音乐目录失败：{e}"))?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S-%3f");
    let safe = provider_name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>();
    let path = dir.join(format!("{stamp}-{safe}.{ext}"));
    std::fs::write(&path, bytes).map_err(|e| format!("写音频失败：{e}"))?;
    let path_text = path.to_string_lossy().into_owned();

    let decoded = state.assets.load(&path_text)?;
    let sample_rate = decoded.sample_rate.max(1.0);
    let seconds = decoded.frames() as f64 / sample_rate;
    let tempo = {
        let engine = state.engine.lock().unwrap();
        engine.project().transport.tempo
    };
    let beats = (seconds * tempo / 60.0).max(0.001);
    let source = AudioSource {
        path: path_text.clone(),
        offset_frames: 0,
        frames: None,
        rate: 1.0,
        pitch_semitones: 0.0,
    };
    let command = EngineCommand::AddAudioTrack {
        name: format!("{provider_name} 生成"),
        start: 0.0,
        length: beats.max(1.0),
        source,
        at: None,
    };
    crate::commands::run_agent_all(app, vec![command]);
    Ok(CloudMusicResult {
        path: path_text,
        duration: seconds,
        provider: provider_name.to_string(),
        model: model.to_string(),
        lyrics,
    })
}

// ---------------------------------------------------------------------------
// HTTP 小工具
// ---------------------------------------------------------------------------

async fn ensure_ok(response: reqwest::Response, label: &str) -> Result<reqwest::Response, String> {
    if response.status().is_success() {
        return Ok(response);
    }
    Err(response_error(response, label).await)
}

async fn response_error(response: reqwest::Response, label: &str) -> String {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    format!("{label} 失败（HTTP {status}）：{}", truncate(&body, 300))
}

async fn json_response(response: reqwest::Response, label: &str) -> Result<Value, String> {
    let response = ensure_ok(response, label).await?;
    response
        .json::<Value>()
        .await
        .map_err(|e| format!("{label} 返回不是 JSON：{e}"))
}

async fn download(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("下载音频失败：{e}"))?;
    let response = ensure_ok(response, "下载音频").await?;
    Ok(response
        .bytes()
        .await
        .map_err(|e| format!("读取音频失败：{e}"))?
        .to_vec())
}

fn ext_from_mime(mime: &str) -> &'static str {
    let mime = mime.to_ascii_lowercase();
    if mime.contains("wav") {
        "wav"
    } else if mime.contains("mpeg") || mime.contains("mp3") {
        "mp3"
    } else if mime.contains("flac") {
        "flac"
    } else if mime.contains("ogg") {
        "ogg"
    } else {
        "mp3"
    }
}

fn ext_from_url(url: &str) -> Option<&'static str> {
    let path = url.split('?').next().unwrap_or(url).to_ascii_lowercase();
    for (suffix, ext) in [
        (".wav", "wav"),
        (".mp3", "mp3"),
        (".flac", "flac"),
        (".ogg", "ogg"),
        (".m4a", "m4a"),
    ] {
        if path.ends_with(suffix) {
            return Some(ext);
        }
    }
    None
}

fn url_encode(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            _ => format!("%{:02X}", c as u32 & 0xff),
        })
        .collect()
}

fn truncate(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_fills_defaults() {
        let mut provider = CloudMusicProvider {
            kind: CloudMusicKind::Treblo,
            ..Default::default()
        };
        normalize(&mut provider);
        assert!(!provider.id.is_empty());
        assert_eq!(provider.base_url, "https://api.treblo.com");
        assert_eq!(provider.name, "Treblo (Sonauto)");
    }

    #[test]
    fn volc_signing_key_matches_documented_sample() {
        let key = volc_signing_key(
            "WkRZeE1EQmxPVGhsWWpWak5HVmtNbUUxTXpZeU9UVXlOMlE1TmpZeVlqTQ==",
            "20250329",
        );
        assert_eq!(
            hex_encode(&key),
            "08e8964f0522368617ac2b0b02c42163fb2589674dcf4ce42628780391057d93"
        );
    }

    #[test]
    fn ext_from_url_handles_query() {
        assert_eq!(ext_from_url("https://x.test/a.wav?sig=1"), Some("wav"));
        assert_eq!(ext_from_url("https://x.test/a.mp3"), Some("mp3"));
        assert_eq!(ext_from_url("https://x.test/a"), None);
    }
}
