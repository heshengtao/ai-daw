//! 「写歌智能体」的连接配置：OpenAI 兼容端点的地址、Key 与模型名。
//!
//! 落在应用配置目录的 `ai.json`，与 `mcp.json` / `store.json` 同一套惯例（明文 JSON，
//! 项目没有 keychain 集成）。文件缺失或损坏时回退到默认值，不让启动失败。

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// 「文本生成」里的一家供应商接入：端点 + Key + 默认模型 + 可选的模型列表。
///
/// 和云端音乐那些服务一样可以配置多家；设置页逐条管理，写歌智能体用**第一家启用的**。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TextProvider {
    pub id: String,
    /// 供应商预设 id（`openai` / `deepseek` / `ollama` / `custom` …），只用于回显。
    pub preset: String,
    /// 显示名（用户可改）；为空时界面用预设名。
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    /// 默认模型。
    pub model: String,
    /// 「获取模型列表」拉到的候选；空 = 只提供默认模型。
    pub models: Vec<String>,
    pub enabled: bool,
}

/// OpenAI 兼容端点的配置。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentConfig {
    /// 供应商 id（`openai` / `deepseek` / `ollama` / `custom` …）。
    ///
    /// 只用来回显与「老配置缺这个字段时按地址反推」；请求本身永远是 OpenAI 兼容
    /// 的 `/chat/completions`，各家的兼容端点由前端预置表填进 `base_url`。
    ///
    /// `base_url` / `api_key` / `model` 是**当前生效供应商**的镜像（智能体面板直接读它），
    /// 真正的数据在 `providers` 里。
    pub provider: String,
    /// 端点根地址，形如 `https://api.openai.com/v1`（也可填兼容网关）。
    pub base_url: String,
    /// API Key，走 `Authorization: Bearer`。
    pub api_key: String,
    /// 模型名。
    pub model: String,
    /// 文本模型的供应商列表；`base_url` / `api_key` / `model` 是其中第一家的镜像。
    pub providers: Vec<TextProvider>,
}

impl AgentConfig {
    /// 当前生效的供应商：第一个启用的；都没启用就用第一条。
    pub fn active_provider(&self) -> Option<&TextProvider> {
        self.providers
            .iter()
            .find(|provider| provider.enabled)
            .or_else(|| self.providers.first())
    }

    /// 是否已配置到「可以尝试调用」的程度。
    ///
    /// Key 不再强制：本地端点（Ollama / vLLM / LM Studio …）本来就不要 Key，
    /// 以前这里要求 Key 会让它们永远显示「未配置」。两种形态都认：有供应商列表时看
    /// 当前生效那家，没有时看老的单条镜像字段。
    pub fn is_configured(&self) -> bool {
        if let Some(provider) = self.active_provider() {
            return !provider.base_url.trim().is_empty() && !provider.model.trim().is_empty();
        }
        !self.base_url.trim().is_empty() && !self.model.trim().is_empty()
    }

    /// 让 `base_url` / `api_key` / `model` / `provider` 跟上当前生效的供应商。
    ///
    /// 智能体面板和 `agent_config` 只认这几个镜像字段，供应商列表是新的单一数据源。
    pub fn sync_mirror(&mut self) {
        if self.providers.is_empty() {
            // 老配置（只有单条 base_url / key / model）：升级成一条供应商，
            // 这样「文本生成」页打开时能看到它、也能直接再添别的。
            if self.base_url.trim().is_empty() {
                return;
            }
            let preset = self.provider_or_inferred();
            let name = provider_display_name(&preset, &self.base_url);
            let model = self.model.clone();
            self.providers.push(TextProvider {
                id: "default".into(),
                preset,
                name,
                base_url: self.base_url.clone(),
                api_key: self.api_key.clone(),
                model: model.clone(),
                models: if model.trim().is_empty() {
                    Vec::new()
                } else {
                    vec![model]
                },
                enabled: true,
            });
            // 落到下面统一的镜像同步：`provider` 也跟着反推出来的预设走。
        }
        let Some(provider) = self.active_provider().cloned() else {
            return;
        };
        self.provider = provider.preset;
        self.base_url = provider.base_url;
        self.api_key = provider.api_key;
        self.model = provider.model;
    }

    /// 缺 `provider` 的老配置按端点地址反推一个，供界面回显。
    pub fn provider_or_inferred(&self) -> String {
        let provider = self.provider.trim();
        if !provider.is_empty() {
            return provider.to_string();
        }
        infer_provider(&self.base_url)
    }
}

/// 按端点地址猜供应商 id（与前端 `inferProviderId` 的常见域名一一对应）。
pub fn infer_provider(base_url: &str) -> String {
    let base = base_url.trim().to_lowercase();
    const HINTS: &[(&str, &str)] = &[
        ("api.openai.com", "openai"),
        ("api.anthropic.com", "anthropic"),
        ("generativelanguage.googleapis.com", "gemini"),
        ("api.deepseek.com", "deepseek"),
        ("dashscope.aliyuncs.com", "dashscope"),
        ("open.bigmodel.cn", "zhipu"),
        ("ark.cn-beijing.volces.com", "ark"),
        ("api.moonshot.cn", "moonshot"),
        ("api.minimaxi.com", "minimax"),
        ("api.stepfun.com", "stepfun"),
        ("api.mistral.ai", "mistral"),
        ("api.lingyiwanwu.com", "lingyi"),
        ("api.baichuan-ai.com", "baichuan"),
        ("qianfan.baidubce.com", "qianfan"),
        ("api.hunyuan.cloud.tencent.com", "hunyuan"),
        ("api.groq.com", "groq"),
        ("api.x.ai", "xai"),
        ("integrate.api.nvidia.com", "nvidia"),
        ("models.github.ai", "github"),
        ("api-inference.modelscope.cn", "modelscope"),
        ("ai.gitee.com", "gitee"),
        ("api.jina.ai", "jina"),
        ("api.perplexity.ai", "perplexity"),
        ("api.siliconflow.cn", "siliconflow"),
        ("openrouter.ai", "openrouter"),
        ("api.together.xyz", "together"),
        ("api.fireworks.ai", "fireworks"),
        ("aihubmix.com", "aihubmix"),
        ("api.302ai.cn", "302ai"),
        ("api.ocoolai.com", "ocoolai"),
        ("dmxapi.cn", "dmxapi"),
        ("api.hyperbolic.xyz", "hyperbolic"),
        ("api.ppinfra.com", "ppinfra"),
        ("127.0.0.1:11434", "ollama"),
        ("localhost:11434", "ollama"),
        ("127.0.0.1:1234", "lmstudio"),
        ("localhost:1234", "lmstudio"),
        ("127.0.0.1:8000", "vllm"),
        ("localhost:8000", "vllm"),
        ("127.0.0.1:9997", "xinference"),
        ("localhost:9997", "xinference"),
    ];
    if base.is_empty() {
        return "custom".into();
    }
    for (needle, id) in HINTS {
        if base.contains(needle) {
            return (*id).to_string();
        }
    }
    "custom".into()
}

/// 老配置升级成供应商时，给一个能认出来的显示名（域名 / 预设 id）。
fn provider_display_name(preset: &str, base_url: &str) -> String {
    let host = base_url
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or("")
        .to_string();
    match preset {
        "custom" if !host.is_empty() => host,
        "custom" => "自定义端点".into(),
        other => other.to_string(),
    }
}

/// 配置的读写器（由 Tauri 管理）。
pub struct AgentConfigRuntime {
    inner: Mutex<AgentConfig>,
    /// 配置文件位置；拿不到配置目录时为 `None`（配置只活在内存里）。
    path: Option<PathBuf>,
}

impl AgentConfigRuntime {
    /// 从 `ai.json` 载入；缺失/损坏回退默认值。
    pub fn load(path: Option<PathBuf>) -> Self {
        let mut config = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| serde_json::from_str::<AgentConfig>(&text).ok())
            .unwrap_or_default();
        // 老的单端点配置在这里升级成一条供应商，并让镜像字段跟上它。
        config.sync_mirror();
        Self {
            inner: Mutex::new(config),
            path,
        }
    }

    /// 当前配置。
    pub fn get(&self) -> AgentConfig {
        self.inner.lock().unwrap().clone()
    }

    /// 更新配置并写盘，返回最新配置。
    pub fn set(&self, next: AgentConfig) -> AgentConfig {
        {
            let mut inner = self.inner.lock().unwrap();
            *inner = next.clone();
        }
        self.save(&next);
        next
    }

    fn save(&self, config: &AgentConfig) {
        let Some(path) = &self.path else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match serde_json::to_string_pretty(config) {
            Ok(json) => {
                if let Err(err) = std::fs::write(path, json) {
                    eprintln!("[agent] 写入配置失败：{err}");
                }
            }
            Err(err) => eprintln!("[agent] 序列化配置失败：{err}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 老 `ai.json`（只有单条端点 / Key / 模型）仍要能读进来，并升级成一条供应商。
    #[test]
    fn legacy_single_endpoint_migrates_to_one_provider() {
        let json =
            r#"{"base_url":"https://api.deepseek.com/v1","api_key":"sk-x","model":"deepseek-chat"}"#;
        let mut config: AgentConfig = serde_json::from_str(json).expect("老配置要能反序列化");
        assert!(config.providers.is_empty(), "没有 providers 字段时按空列表处理");
        config.sync_mirror();
        assert_eq!(config.providers.len(), 1, "单端点配置要升级成一家供应商");
        let provider = &config.providers[0];
        assert_eq!(provider.id, "default");
        assert_eq!(provider.preset, "deepseek", "预设 id 按端点地址反推");
        assert_eq!(provider.model, "deepseek-chat");
        assert_eq!(provider.models, vec!["deepseek-chat".to_string()]);
        assert!(provider.enabled);
        // 镜像字段跟随这家供应商。
        assert_eq!(config.provider, "deepseek");
        assert_eq!(config.base_url, "https://api.deepseek.com/v1");
        assert!(config.is_configured());
    }

    /// 镜像字段跟随第一家启用的供应商；都没启用时退回第一条。
    #[test]
    fn mirror_follows_first_enabled_provider() {
        let mut config = AgentConfig {
            providers: vec![
                TextProvider {
                    id: "a".into(),
                    preset: "openai".into(),
                    base_url: "https://a.example/v1".into(),
                    model: "m-a".into(),
                    enabled: false,
                    ..Default::default()
                },
                TextProvider {
                    id: "b".into(),
                    preset: "deepseek".into(),
                    base_url: "https://b.example/v1".into(),
                    api_key: "k".into(),
                    model: "m-b".into(),
                    enabled: true,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        config.sync_mirror();
        assert_eq!(config.provider, "deepseek");
        assert_eq!(config.base_url, "https://b.example/v1");
        assert_eq!(config.api_key, "k");
        assert_eq!(config.model, "m-b");

        for provider in &mut config.providers {
            provider.enabled = false;
        }
        config.sync_mirror();
        assert_eq!(config.base_url, "https://a.example/v1", "全停用时退回第一条");
        assert_eq!(config.model, "m-a");
    }

    /// 与前端 `TextProvider` 的字段名约定：进出都是 snake_case（`base_url` / `api_key`），
    /// 缺字段（老文件 / 手工编辑过的 `ai.json`）也要能读。
    #[test]
    fn provider_json_uses_snake_case_fields() {
        let json = r#"[{"id":"x","preset":"openai","name":"OpenAI",
            "base_url":"https://api.openai.com/v1","api_key":"k",
            "model":"gpt-4.1-mini","models":["gpt-4.1-mini"],"enabled":true}]"#;
        let providers: Vec<TextProvider> = serde_json::from_str(json).expect("前端形状要能反序列化");
        assert_eq!(providers[0].base_url, "https://api.openai.com/v1");
        assert_eq!(providers[0].api_key, "k");

        let out = serde_json::to_value(&providers[0]).expect("要能序列化回去");
        assert!(out.get("base_url").is_some(), "出去的字段名同样是 snake_case");
        assert!(out.get("baseUrl").is_none());

        let partial: TextProvider =
            serde_json::from_str(r#"{"id":"y","base_url":"https://b"}"#).expect("缺字段要能读");
        assert_eq!(partial.id, "y");
        assert!(partial.models.is_empty());
        assert!(!partial.enabled);
    }

    /// 镜像字段被清空（供应商列表删光）时，不能再被 `sync_mirror` 复活成一家供应商。
    #[test]
    fn cleared_mirror_does_not_resurrect_a_provider() {
        let mut config = AgentConfig::default();
        config.sync_mirror();
        assert!(config.providers.is_empty());
        assert!(!config.is_configured());
    }
}
