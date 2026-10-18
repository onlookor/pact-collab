//! pact-store 错误类型。

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

/// 域名校验：防路径穿越 / 保留名（store 域名直接映射为文件名）。
/// 规则：非空，仅 [A-Za-z0-9_-]，长度 ≤ 128。
pub fn validate_domain_name(name: &str) -> Result<(), StoreError> {
    let ok = !name.is_empty()
        && name.len() <= 128
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if ok {
        Ok(())
    } else {
        Err(StoreError::InvalidDomainName {
            name: name.to_owned(),
        })
    }
}
