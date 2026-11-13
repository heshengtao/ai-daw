//! 会话持久化：树形消息。
//!
//! 每个会话是一棵「节点树」：`Node` 带 `parent` 链，`head` 指向当前活跃叶。
//! 「编辑某条用户输入后重答」= 建一个同父兄弟节点并切换 `head`，因此天然支持分支。
//! 每个会话落一个 JSON 文件（`agent/sessions/<id>.json`），原子写。

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// 助手发起的一次工具调用（持久化形状）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRef {
    pub id: String,
    pub name: String,
    /// 原始 arguments JSON 文本。
    #[serde(default)]
    pub arguments: String,
}

/// 树上的一个节点。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    #[serde(default)]
    pub parent: Option<String>,
    /// `"user"` / `"assistant"` / `"tool"`。
    pub role: String,
    #[serde(default)]
    pub content: String,
    /// 思考过程（若模型提供）。
    #[serde(default)]
    pub reasoning: Option<String>,
    /// 助手的工具调用。
    #[serde(default)]
    pub tool_calls: Vec<ToolCallRef>,
    /// `tool` 节点对应的调用 id。
    #[serde(default)]
    pub tool_call_id: Option<String>,
    /// `tool` 节点的工具名。
    #[serde(default)]
    pub name: Option<String>,
    pub created_at: u64,
    /// 该轮失败时的提示（挂在 assistant 节点）。
    #[serde(default)]
    pub error: Option<String>,
}

/// 一个完整会话。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    #[serde(default)]
    pub title: String,
    pub created_at: u64,
    pub updated_at: u64,
    #[serde(default)]
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub head: Option<String>,
}

/// 历史列表里的一项（不含节点，列表不拉全量）。
#[derive(Debug, Clone, Serialize)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub updated_at: u64,
    pub count: usize,
}

/// 会话目录的读写器。
#[derive(Clone)]
pub struct SessionStore {
    dir: Option<PathBuf>,
}

impl SessionStore {
    /// `dir` 为 `agent/sessions/`；拿不到时为 `None`（会话只活在内存里）。
    pub fn new(dir: Option<PathBuf>) -> Self {
        if let Some(d) = &dir {
            let _ = std::fs::create_dir_all(d);
        }
        Self { dir }
    }

    fn file(&self, id: &str) -> Option<PathBuf> {
        // id 只由我们自己生成（数字与短横线），用前再过滤一次，杜绝路径穿越。
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return None;
        }
        self.dir.as_ref().map(|d| d.join(format!("{id}.json")))
    }

    /// 全部会话摘要（按更新时间倒序）。
    pub fn list(&self) -> Vec<SessionSummary> {
        let Some(dir) = &self.dir else {
            return Vec::new();
        };
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut out: Vec<SessionSummary> = entries
            .flatten()
            .filter_map(|e| {
                let path = e.path();
                if path.extension().and_then(|x| x.to_str()) != Some("json") {
                    return None;
                }
                let text = std::fs::read_to_string(&path).ok()?;
                let session: Session = serde_json::from_str(&text).ok()?;
                Some(SessionSummary {
                    id: session.id,
                    title: session.title,
                    updated_at: session.updated_at,
                    count: session.nodes.iter().filter(|n| n.role != "tool").count(),
                })
            })
            .collect();
        out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        out
    }

    /// 载入一个会话。
    pub fn load(&self, id: &str) -> Option<Session> {
        let path = self.file(id)?;
        let text = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// 原子写一个会话。
    pub fn save(&self, session: &Session) -> Result<(), String> {
        let Some(path) = self.file(&session.id) else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(session).map_err(|e| e.to_string())?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
    }

    /// 删除一个会话。
    pub fn delete(&self, id: &str) {
        if let Some(path) = self.file(id) {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// 当前毫秒时间戳。
pub fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 生成一个文件系统安全的 id。
pub fn new_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    format!(
        "{}-{}",
        now_millis(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

/// 由首条用户输入生成会话标题。
pub fn title_from(text: &str) -> String {
    let t: String = text.trim().chars().take(24).collect();
    if t.is_empty() {
        "新对话".into()
    } else {
        t
    }
}
