//! pact-store 错误类型。

use pact_store_model::ModelError;
use std::path::PathBuf;

#[derive(Debug)]
pub enum StoreError {
    /// 文件系统 IO 失败（含上下文描述）。
    Io {
        context: &'static str,
        source: std::io::Error,
    },
    /// JSON（事件行/快照）序列化或解析失败。
    Json {
        context: &'static str,
        source: serde_json::Error,
    },
    /// turso 索引错误（字符串化，D13 隔离层内部细节不外泄）。
    Index(String),
    /// 日志中存在无法解析的完整行（非崩溃残留，疑似损坏）。
    CorruptLine {
        path: PathBuf,
        line: usize,
        reason: String,
    },
    /// 事件 seq 违反逐行 +1 单调不变量。
    SeqNotMonotonic {
        path: PathBuf,
        expected: u64,
        got: u64,
    },
    /// 域名非法（含路径分隔符等，防路径穿越）。
    InvalidDomainName { name: String },
    /// 实例名非法（与域名同谓词，防路径穿越）。
    InvalidInstanceName { name: String },
    /// 检测到旧单层布局（`<data_dir>/<domain>.events.jsonl` 等）残留。
    /// 双键布局下不再识别，须人工迁移（fail-fast，不静默丢数据）。
    LegacyLayoutDetected { path: PathBuf },
    /// 事件/快照 schema 版本高于本实现支持上限（向前不兼容，fail-fast）。
    /// 发生于读到更高版本日志/快照时（不静默按 v1 重放未知语义）。
    UnsupportedSchemaVersion {
        /// 记录中声明的版本。
        found: u32,
        /// 本实现支持的版本上限。
        supported: u32,
    },
    /// 日志前缀已被压缩（`base_seq > 0` 或首行 seq > 1）却找不到覆盖其前缀的快照。
    /// 意味着历史状态不可恢复（数据已丢失），fail-fast 并须人工介入。
    SnapshotRequiredForCompactedLog {
        path: PathBuf,
        /// 需要被快照覆盖的最小事件 seq（含）。
        base_seq: u64,
    },
    /// `Consumed` 事件的目标不在当前 Counter（日志与状态不一致，拒绝重复归档）。
    ConsumeTargetNotOnCounter { token_id: String },
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Io { context, source } => write!(f, "存储 IO 失败（{context}）：{source}"),
            StoreError::Json { context, source } => {
                write!(f, "JSON 处理失败（{context}）：{source}")
            }
            StoreError::Index(e) => write!(f, "索引失败：{e}"),
            StoreError::CorruptLine { path, line, reason } => {
                write!(f, "日志 {} 第 {line} 行损坏：{reason}", path.display())
            }
            StoreError::SeqNotMonotonic {
                path,
                expected,
                got,
            } => {
                write!(
                    f,
                    "日志 {} 事件序号断裂：期望 {expected}，遇到 {got}",
                    path.display()
                )
            }
            StoreError::InvalidDomainName { name } => {
                write!(f, "非法域名：{name:?}（仅允许字母、数字、'-'、'_'）")
            }
            StoreError::InvalidInstanceName { name } => {
                write!(f, "非法实例名：{name:?}（仅允许字母、数字、'-'、'_'）")
            }
            StoreError::LegacyLayoutDetected { path } => write!(
                f,
                "检测到旧单层布局残留 {}：双键布局（<domain>/<instance>.events.jsonl）\
                 不再识别旧文件，请人工迁移后再启动",
                path.display()
            ),
            StoreError::UnsupportedSchemaVersion { found, supported } => write!(
                f,
                "记录 schema 版本 v{found} 高于本实现支持上限 v{supported}：\
                 请升级 pactd，或迁移数据后再启动（不按旧版本静默重放）"
            ),
            StoreError::SnapshotRequiredForCompactedLog { path, base_seq } => write!(
                f,
                "日志 {} 的前缀已被压缩（需覆盖 seq ≤ {base_seq}），但缺少可用快照：\
                 历史状态不可恢复，请人工介入（禁止静默丢数据）",
                path.display()
            ),
            StoreError::ConsumeTargetNotOnCounter { token_id } => write!(
                f,
                "Consumed 事件的目标 token {token_id} 不在当前 Counter：\
                 日志与状态不一致，拒绝重复归档"
            ),
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StoreError::Io { source, .. } => Some(source),
            StoreError::Json { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<std::io::Error> for StoreError {
    fn from(source: std::io::Error) -> Self {
        StoreError::Io {
            context: "文件操作",
            source,
        }
    }
}

impl From<serde_json::Error> for StoreError {
    fn from(source: serde_json::Error) -> Self {
        StoreError::Json {
            context: "JSON",
            source,
        }
    }
}

/// 纯模型错误 → 存储错误：变体一一对应，调用方仍可匹配既有 `StoreError`。
impl From<ModelError> for StoreError {
    fn from(e: ModelError) -> Self {
        match e {
            ModelError::Json { context, source } => StoreError::Json { context, source },
            ModelError::InvalidDomainName { name } => StoreError::InvalidDomainName { name },
            ModelError::InvalidInstanceName { name } => StoreError::InvalidInstanceName { name },
            ModelError::UnsupportedSchemaVersion { found, supported } => {
                StoreError::UnsupportedSchemaVersion { found, supported }
            }
            ModelError::ConsumeTargetNotOnCounter { token_id } => {
                StoreError::ConsumeTargetNotOnCounter { token_id }
            }
        }
    }
}

/// 域名校验：防路径穿越 / 保留名（store 域名直接映射为目录名）。
/// 规则：非空，仅 [A-Za-z0-9_-]，长度 ≤ 128。
pub fn validate_domain_name(name: &str) -> Result<(), StoreError> {
    pact_store_model::validate_domain_name(name).map_err(Into::into)
}

/// 实例名校验（与域名同谓词；store 实例名直接映射为文件名）。
pub fn validate_instance_name(name: &str) -> Result<(), StoreError> {
    pact_store_model::validate_instance_name(name).map_err(Into::into)
}
