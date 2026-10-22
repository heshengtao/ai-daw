use serde::{Deserialize, Serialize};

use crate::event::Actor;

/// 操作记录里的一个附加参数。
///
/// `untagged` 让线上形状就是裸值（数字 / 字符串 / 布尔），前端拿到后可直接插进本地化文案。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OpValue {
    Num(f64),
    Text(String),
    Bool(bool),
}

/// 一条附加参数：键 + 值。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpArg {
    pub key: String,
    pub value: OpValue,
}

/// 一条操作记录：撤销 / 重做时间线上的一个单元。
///
/// 与引擎的一个撤销批次一一对应——一次 `execute_all` 产生一条，因此「回撤到某条操作」就是
/// 「撤销到该批次之前」。`actor` 记录是人工还是哪个 AI 代理所为。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpRecord {
    /// 会话内稳定自增 id。
    pub id: u64,
    /// 操作发生的 Unix 毫秒时间。
    pub at: u64,
    /// 发起者：人类、外部 AI（MCP）或内置助手（chat）。
    pub actor: Actor,
    /// 机器可读的操作类型（前端据此本地化），如 `add_track`。
    pub kind: String,
    /// 操作对象（轨道名等）；没有明确对象时为 `None`。
    pub target: Option<String>,
    /// 附加参数（数值 / 文本 / 布尔），供界面拼出可读文案。
    pub args: Vec<OpArg>,
    /// 本操作产生的事件条数。
    pub event_count: usize,
    /// 是否仍处于「已应用」状态；`false` 表示已被撤销、可以重做。
    pub applied: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 参数在线上是裸值（不是 `{"Num": 120.0}`），前端才能直接拼文案。
    #[test]
    fn op_record_wire_shape_uses_bare_arg_values() {
        let op = OpRecord {
            id: 7,
            at: 1_700_000_000_000,
            actor: Actor::Agent("mcp".into()),
            kind: "set_tempo".into(),
            target: None,
            args: vec![
                OpArg { key: "tempo".into(), value: OpValue::Num(128.0) },
                OpArg { key: "note".into(), value: OpValue::Text("x".into()) },
                OpArg { key: "on".into(), value: OpValue::Bool(true) },
            ],
            event_count: 1,
            applied: true,
        };
        let value = serde_json::to_value(&op).unwrap();
        assert_eq!(value["kind"], "set_tempo");
        assert_eq!(value["actor"]["Agent"], "mcp");
        assert_eq!(value["args"][0]["value"], 128.0, "数值参数必须是裸数字");
        assert_eq!(value["args"][1]["value"], "x");
        assert_eq!(value["args"][2]["value"], true);
    }
}
