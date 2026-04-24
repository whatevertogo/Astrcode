//! server 私有的 session fork 选择枚举。
//!
//! 只保留 `ports::app_session` 需要的最小类型定义。

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionForkSelector {
    Latest,
    TurnEnd { turn_id: String },
    StorageSeq { storage_seq: u64 },
}
