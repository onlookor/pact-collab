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
    /// 是否持久化 Rule-R 读审计（D25 子项 3；缺省 false）。
    /// 开启后审计写失败即拒绝本次读取（fail-closed）。
    #[serde(default)]
    pub read_audit: Option<bool>,
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

/// 参与者令牌表与域级授权（NFR5 / FR17：可选；缺省 = 信任声明模式）。
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct AuthConfig {
    #[serde(default)]
    pub tokens: BTreeMap<String, String>,
    /// 参与者 → 授权条目清单，条目形如 `domain/instance` 或 `domain/*`（FR19 实例键）。
    #[serde(default)]
    pub acl: BTreeMap<String, Vec<String>>,
}

/// 域 ACL：参与者 → 授权条目（`domain/instance` / `domain/*`）。
pub type AclMap = BTreeMap<String, Vec<String>>;

/// ACL 条目是否覆盖 `(domain, instance)`。
pub fn acl_allows(acl: &AclMap, principal: &str, domain: &str, instance: &str) -> bool {
    acl.get(principal).is_some_and(|patterns| {
        patterns.iter().any(|p| match p.split_once('/') {
            Some((d, i)) => d == domain && (i == "*" || i == instance),
            None => false,
        })
    })
}

/// ACL 中是否存在该参与者对 `domain` 的任一授权（定义级操作使用）。
pub fn acl_allows_domain(acl: &AclMap, principal: &str, domain: &str) -> bool {
    acl.get(principal).is_some_and(|patterns| {
        patterns
            .iter()
            .any(|p| p.split_once('/').is_some_and(|(d, _)| d == domain))
    })
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

    /// 是否持久化 Rule-R 读审计（缺省关闭）。
    pub fn read_audit(&self) -> bool {
        self.read_audit.unwrap_or(false)
    }

    /// 域级授权表（空表 = 不启用 ACL，保持向后兼容）。
    pub fn acl(&self) -> AclMap {
        self.auth
            .as_ref()
            .map(|a| a.acl.clone())
            .unwrap_or_default()
    }

    /// 配置合法性校验（daemon 启动前调用）。
    ///
    /// - ACL 非空但令牌表为空：无法确定调用者身份 → 拒绝启动；
    /// - ACL 条目必须是 `domain/instance` 或 `domain/*`，且域名/实例名合法。
    pub fn validate(&self) -> Result<(), ConfigError> {
        let acl = self.acl();
        if acl.is_empty() {
            return Ok(());
        }
        if self.auth_tokens().is_empty() {
            return Err(ConfigError::Invalid(
                "配置了 [auth.acl] 但未配置 [auth.tokens]：无法确定调用者身份".to_owned(),
            ));
        }
        for (principal, patterns) in &acl {
            if patterns.is_empty() {
                return Err(ConfigError::Invalid(format!(
                    "ACL 参与者 {principal} 的授权清单为空"
                )));
            }
            for pattern in patterns {
                let (domain, instance) = pattern.split_once('/').ok_or_else(|| {
                    ConfigError::Invalid(format!(
                        "ACL 条目 {pattern:?} 非法：须为 domain/instance 或 domain/*"
                    ))
                })?;
                pact_store::validate_domain_name(domain)
                    .map_err(|_| ConfigError::Invalid(format!("ACL 条目 {pattern:?}：域名非法")))?;
                if instance != "*" {
                    pact_store::validate_instance_name(instance).map_err(|_| {
                        ConfigError::Invalid(format!("ACL 条目 {pattern:?}：实例名非法"))
                    })?;
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub enum ConfigError {
    Io(PathBuf, String),
    Parse(PathBuf, String),
    /// 语义非法（如 ACL 无令牌、条目格式错误）。
    Invalid(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io(p, e) => write!(f, "读取配置 {} 失败：{e}", p.display()),
            ConfigError::Parse(p, e) => write!(f, "解析配置 {} 失败：{e}", p.display()),
            ConfigError::Invalid(msg) => write!(f, "配置非法：{msg}"),
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
        assert!(!c.read_audit());
        assert!(c.auth_tokens().is_empty());
        assert!(c.acl().is_empty());
        c.validate().expect("缺省配置合法");
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
read_audit = true

[mcp]
enabled = true

[auth]
[auth.tokens]
architect = "sekret"
reviewer = "sekret2"

[auth.acl]
architect = ["proj/*", "other/default"]
reviewer = ["proj/review"]
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
        assert!(c.read_audit());
        assert_eq!(c.auth_tokens()["architect"], "sekret");
        assert_eq!(c.acl()["architect"], vec!["proj/*", "other/default"]);
        c.validate().expect("完整配置合法");
    }

    #[test]
    fn acl_without_tokens_is_rejected() {
        let text = r#"
[auth.acl]
architect = ["proj/*"]
"#;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pact.toml");
        std::fs::write(&path, text).unwrap();
        let c = Config::load(&path).unwrap();
        assert!(matches!(c.validate(), Err(ConfigError::Invalid(_))));
    }

    #[test]
    fn acl_patterns_are_validated() {
        for pattern in ["proj", "proj/*/x", "../x", "proj/bad name", "/default"] {
            let text = format!(
                "[auth.tokens]\narchitect = \"t\"\n\n[auth.acl]\narchitect = [\"{pattern}\"]\n"
            );
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("pact.toml");
            std::fs::write(&path, text).unwrap();
            let c = Config::load(&path).unwrap();
            assert!(
                matches!(c.validate(), Err(ConfigError::Invalid(_))),
                "条目 {pattern:?} 应被拒绝"
            );
        }
    }

    #[test]
    fn acl_matching_supports_instance_and_wildcard() {
        let mut acl = AclMap::new();
        acl.insert("arch".into(), vec!["proj/*".into(), "other/x".into()]);
        assert!(acl_allows(&acl, "arch", "proj", "default"));
        assert!(acl_allows(&acl, "arch", "proj", "x"));
        assert!(acl_allows(&acl, "arch", "other", "x"));
        assert!(!acl_allows(&acl, "arch", "other", "default"));
        assert!(!acl_allows(&acl, "reviewer", "proj", "default"));
        assert!(acl_allows_domain(&acl, "arch", "proj"));
        assert!(acl_allows_domain(&acl, "arch", "other"));
        assert!(!acl_allows_domain(&acl, "arch", "third"));
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
