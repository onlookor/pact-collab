//! `pact.toml` 配置（设计 §9.4）。
//!
//! 缺省语义：文件不存在或缺字段时取默认值——
//! `data_dir = "./data"`、`listen_unix = "./pact.sock"`、无 TCP、无令牌表（信任声明模式）、无 PID 文件。

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// 事件日志/快照/索引目录。
    #[serde(default)]
    pub data_dir: Option<PathBuf>,
    /// unix socket 监听路径（默认；与 `listen_tcp` 可同时启用）。
    #[serde(default)]
    pub listen_unix: Option<PathBuf>,
    /// 可选 TCP 监听（如 "127.0.0.1:8787"）。
    #[serde(default)]
    pub listen_tcp: Option<String>,
    /// 可选 PID 文件路径（缺省不写）。启动时写入、退出时删除；
    /// 启动时若发现陈旧文件且其 PID 仍存活，则拒绝启动（防双实例）。
    #[serde(default)]
    pub pid_file: Option<PathBuf>,
    /// 启动恢复后是否自动压缩全部实例日志（D25 子项 1；缺省 false）。
    #[serde(default)]
    pub compact_on_start: Option<bool>,
    /// 压缩保留的最近事件条数（缺省 0 = 丢弃全部已入快照的前缀）。
    #[serde(default)]
    pub compact_keep_last: Option<u64>,
    #[serde(default)]
    pub mcp: Option<McpConfig>,
    #[serde(default)]
    pub auth: Option<AuthConfig>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct McpConfig {
    #[serde(default)]
    pub enabled: bool,
}

/// 参与者令牌表（NFR5：可选；缺省 = 信任声明模式）。
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct AuthConfig {
    #[serde(default)]
    pub tokens: BTreeMap<String, String>,
}

impl Config {
    /// 读取配置文件；文件不存在返回全默认配置。
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path)
            .map_err(|e| ConfigError::Io(path.to_owned(), e.to_string()))?;
        toml::from_str(&text).map_err(|e| ConfigError::Parse(path.to_owned(), e.to_string()))
    }

    pub fn data_dir(&self) -> PathBuf {
        self.data_dir
            .clone()
            .unwrap_or_else(|| PathBuf::from("./data"))
    }

    /// unix socket 路径：显式配置 > 默认；配置为空串视为禁用。
    pub fn listen_unix(&self) -> Option<PathBuf> {
        match &self.listen_unix {
            Some(p) if p.as_os_str().is_empty() => None,
            Some(p) => Some(p.clone()),
            None => {
                if self.listen_tcp.is_some() {
                    None
                } else {
                    Some(PathBuf::from("./pact.sock"))
                }
            }
        }
    }

    /// 启动后自动压缩全部实例日志（缺省关闭）。
    pub fn compact_on_start(&self) -> bool {
        self.compact_on_start.unwrap_or(false)
    }

    /// 压缩保留的尾部事件条数（缺省 0）。
    pub fn compact_keep_last(&self) -> u64 {
        self.compact_keep_last.unwrap_or(0)
    }

    /// 参与者 → 令牌（空表 = 信任声明模式，不校验）。
    pub fn auth_tokens(&self) -> BTreeMap<String, String> {
        self.auth
            .as_ref()
            .map(|a| a.tokens.clone())
            .unwrap_or_default()
    }
}

#[derive(Debug)]
pub enum ConfigError {
    Io(PathBuf, String),
    Parse(PathBuf, String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io(p, e) => write!(f, "读取配置 {} 失败：{e}", p.display()),
            ConfigError::Parse(p, e) => write!(f, "解析配置 {} 失败：{e}", p.display()),
        }
    }
}

impl std::error::Error for ConfigError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_without_file() {
        let c = Config::load(Path::new("/nonexistent/pact.toml")).unwrap();
        assert_eq!(c.data_dir(), PathBuf::from("./data"));
        assert_eq!(c.listen_unix(), Some(PathBuf::from("./pact.sock")));
        assert!(c.listen_tcp.is_none());
        assert!(c.pid_file.is_none());
        assert!(!c.compact_on_start());
        assert_eq!(c.compact_keep_last(), 0);
        assert!(c.auth_tokens().is_empty());
    }

    #[test]
    fn full_config_roundtrip() {
        let text = r#"
data_dir = "/tmp/pact-data"
listen_unix = "/tmp/pact.sock"
listen_tcp = "127.0.0.1:8787"
pid_file = "/tmp/pactd.pid"
compact_on_start = true
compact_keep_last = 8

[mcp]
enabled = true

[auth]
[auth.tokens]
architect = "sekret"
"#;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pact.toml");
        std::fs::write(&path, text).unwrap();
        let c = Config::load(&path).unwrap();
        assert_eq!(c.data_dir(), PathBuf::from("/tmp/pact-data"));
        assert_eq!(c.listen_unix(), Some(PathBuf::from("/tmp/pact.sock")));
        assert_eq!(c.listen_tcp.as_deref(), Some("127.0.0.1:8787"));
        assert_eq!(c.pid_file.as_deref(), Some(Path::new("/tmp/pactd.pid")));
        assert!(c.compact_on_start());
        assert_eq!(c.compact_keep_last(), 8);
        assert!(c.mcp.as_ref().unwrap().enabled);
        assert_eq!(c.auth_tokens()["architect"], "sekret");
    }

    #[test]
    fn tcp_only_disables_default_unix() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pact.toml");
        std::fs::write(&path, "listen_tcp = \"127.0.0.1:1\"\n").unwrap();
        let c = Config::load(&path).unwrap();
        assert_eq!(c.listen_unix(), None);
    }
}
