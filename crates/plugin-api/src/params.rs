use serde::{Deserialize, Serialize};

/// 参数标识。
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ParamId(pub String);

impl From<&str> for ParamId {
    fn from(s: &str) -> Self {
        ParamId(s.to_string())
    }
}

impl From<String> for ParamId {
    fn from(s: String) -> Self {
        ParamId(s)
    }
}

/// 参数元信息。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParamInfo {
    pub id: ParamId,
    pub name: String,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub automatable: bool,
}
