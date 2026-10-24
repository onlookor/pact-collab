//! 纯模型错误：只含可跨 WASM 边界表达的失败，不携带 `PathBuf`/`io::Error`。

/// 事件/快照模型的错误。
#[derive(Debug)]
pub enum ModelError {
    /// JSON（事件行/快照）序列化或解析失败。
    Json {
        context: &'static str,
        source: serde_json::Error,
    },
    /// 域名非法（含路径分隔符等，防路径穿越）。
    InvalidDomainName { name: String },
    /// 实例名非法（与域名同谓词，防路径穿越）。
    InvalidInstanceName { name: String },
    /// 事件/快照 schema 版本高于本实现支持上限（向前不兼容，fail-fast）。
    UnsupportedSchemaVersion {
        /// 记录中声明的版本。
        found: u32,
        /// 本实现支持的版本上限。
        supported: u32,
    },
    /// `Consumed` 事件的目标不在当前 Counter：日志与状态不一致
    /// （重复消费、越权重放或宿主违反同一实例 `prepare → commit` 串行契约），
    /// fail-fast 以免同一 token 重复入 Archive。
    ConsumeTargetNotOnCounter { token_id: String },
}

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModelError::Json { context, source } => {
                write!(f, "JSON 处理失败（{context}）：{source}")
            }
            ModelError::InvalidDomainName { name } => {
                write!(f, "非法域名：{name:?}（仅允许字母、数字、'-'、'_'）")
            }
            ModelError::InvalidInstanceName { name } => {
                write!(f, "非法实例名：{name:?}（仅允许字母、数字、'-'、'_'）")
            }
            ModelError::UnsupportedSchemaVersion { found, supported } => write!(
                f,
                "记录 schema 版本 v{found} 高于本实现支持上限 v{supported}：\
                 请升级 pactd，或迁移数据后再启动（不按旧版本静默重放）"
            ),
            ModelError::ConsumeTargetNotOnCounter { token_id } => write!(
                f,
                "Consumed 事件的目标 token {token_id} 不在当前 Counter：\
                 日志与状态不一致（可能已消费或未产生），拒绝重复归档"
            ),
        }
    }
}

impl std::error::Error for ModelError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ModelError::Json { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<serde_json::Error> for ModelError {
    fn from(source: serde_json::Error) -> Self {
        ModelError::Json {
            context: "JSON",
            source,
        }
    }
}

/// 键名公共谓词：非空、仅 [A-Za-z0-9_-]、长度 ≤ 128（天然禁 `..` 与分隔符注入）。
fn valid_key_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// 域名校验：防路径穿越 / 保留名（域名直接映射为目录名）。
pub fn validate_domain_name(name: &str) -> Result<(), ModelError> {
    if valid_key_name(name) {
        Ok(())
    } else {
        Err(ModelError::InvalidDomainName {
            name: name.to_owned(),
        })
    }
}

/// 实例名校验（与域名同谓词；实例名直接映射为文件名）。
pub fn validate_instance_name(name: &str) -> Result<(), ModelError> {
    if valid_key_name(name) {
        Ok(())
    } else {
        Err(ModelError::InvalidInstanceName {
            name: name.to_owned(),
        })
    }
}
