//! `pact.toml` 配置（设计 §9.4）。
//!
//! 缺省语义：文件不存在或缺字段时取默认值——
//! `data_dir = "./data"`、`listen_unix = "./pact.sock"`、无 TCP、无令牌表（信任声明模式）、无 PID 文件。

use base64::Engine as _;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
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
    /// D42：物引可见性 profile 来源（RefBindings 文件或目录）。
    #[serde(default)]
    pub ref_bindings: Option<RefBindingsConfig>,
    #[serde(default)]
    pub auth: Option<AuthConfig>,
    /// D34：TCP 监听的可选 TLS 终止配置；缺省关闭。
    #[serde(default)]
    pub tls: Option<TlsConfig>,
}

/// D42：RefBindings 路径列表。
///
/// ```toml
/// [ref_bindings]
/// paths = ["/etc/pact/refs/", "se_lifecycle.refs.yaml"]
/// ```
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct RefBindingsConfig {
    /// 文件或目录；目录会展开 `.yaml` / `.yml` / `.json`。
    #[serde(default)]
    pub paths: Vec<PathBuf>,
}

/// D34：TLS 证书与可选 mTLS。
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct TlsConfig {
    #[serde(default)]
    pub certificate: Option<PathBuf>,
    #[serde(default)]
    pub private_key: Option<PathBuf>,
    /// 存在即要求客户端证书（mTLS）。
    #[serde(default)]
    pub client_ca: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct McpConfig {
    #[serde(default)]
    pub enabled: bool,
    /// D36：对外可达基址（OIDC 启用时必填），用于 RFC 9728 元数据。
    #[serde(default)]
    pub public_url: Option<String>,
    /// D36：OIDC 资源服务器配置；缺省不启用。
    #[serde(default)]
    pub oidc: Option<McpOidcConfig>,
}

/// D36：MCP 的 OIDC 资源服务器配置。
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct McpOidcConfig {
    #[serde(default)]
    pub issuer: Option<String>,
    #[serde(default)]
    pub audience: Option<String>,
    #[serde(default)]
    pub principal_claim: Option<String>,
    #[serde(default)]
    pub principal_map: BTreeMap<String, String>,
    #[serde(default)]
    pub jwks_uri: Option<String>,
    #[serde(default)]
    pub discovery: Option<bool>,
    #[serde(default)]
    pub allowed_algs: Option<Vec<String>>,
    #[serde(default)]
    pub ca: Option<PathBuf>,
    #[serde(default)]
    pub allow_static_tokens: Option<bool>,
    /// 仅测试/本地回环：允许 `http://` 的 issuer 与 jwks_uri。
    #[serde(default)]
    pub allow_insecure_http: Option<bool>,
}

/// 已校验的 OIDC 运行配置（`Config::validate` 通过后由 `oidc_settings` 产出）。
#[derive(Debug, Clone, PartialEq)]
pub struct OidcSettings {
    pub issuer: String,
    pub audience: String,
    pub principal_claim: String,
    pub principal_map: BTreeMap<String, String>,
    pub jwks_uri: Option<String>,
    pub discovery: bool,
    pub allowed_algs: Vec<String>,
    pub ca: Option<PathBuf>,
    pub allow_static_tokens: bool,
    pub allow_insecure_http: bool,
}

/// 参与者令牌表与域级授权（NFR5 / FR17：可选；缺省 = 信任声明模式）。
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct AuthConfig {
    #[serde(default)]
    pub tokens: BTreeMap<String, String>,
    /// D34：参与者 → base64(32 字节 Ed25519 公钥)。
    #[serde(default)]
    pub keys: BTreeMap<String, String>,
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

/// ACL 中是否有该域的**通配**授权（`domain/*`）。
/// 影响整个域的运维动作（如活动实例切换）要求通配授权，单实例授权不足。
pub fn acl_allows_wildcard(acl: &AclMap, principal: &str, domain: &str) -> bool {
    acl.get(principal)
        .is_some_and(|patterns| patterns.iter().any(|p| p == &format!("{domain}/*")))
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

    /// D34：参与者 → Ed25519 公钥（32 字节）。非法 base64/长度 → 配置错误。
    pub fn auth_keys(&self) -> Result<BTreeMap<String, [u8; 32]>, ConfigError> {
        let raw = self
            .auth
            .as_ref()
            .map(|a| a.keys.clone())
            .unwrap_or_default();
        let mut out = BTreeMap::new();
        for (principal, encoded) in raw {
            let bytes = decode_public_key(&encoded).ok_or_else(|| {
                ConfigError::Invalid(format!(
                    "[auth.keys] 参与者 {principal} 的公钥非法：须为 base64(32 字节 Ed25519 公钥)"
                ))
            })?;
            out.insert(principal, bytes);
        }
        Ok(out)
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

    /// D36：OIDC 运行配置（无 `[mcp.oidc]` 时返回 None）。
    pub fn oidc_settings(&self) -> Option<OidcSettings> {
        let raw = self.mcp.as_ref()?.oidc.as_ref()?;
        Some(OidcSettings {
            issuer: raw.issuer.clone().unwrap_or_default(),
            audience: raw.audience.clone().unwrap_or_default(),
            principal_claim: raw
                .principal_claim
                .clone()
                .unwrap_or_else(|| "sub".to_owned()),
            principal_map: raw.principal_map.clone(),
            jwks_uri: raw.jwks_uri.clone(),
            discovery: raw.discovery.unwrap_or(true),
            allowed_algs: raw.allowed_algs.clone().unwrap_or_else(default_algs),
            ca: raw.ca.clone(),
            allow_static_tokens: raw.allow_static_tokens.unwrap_or(true),
            allow_insecure_http: raw.allow_insecure_http.unwrap_or(false),
        })
    }

    /// D42：RefBindings 路径（空 = 无显式 profile，全部走默认 read-gate）。
    pub fn ref_binding_paths(&self) -> &[PathBuf] {
        self.ref_bindings
            .as_ref()
            .map(|config| config.paths.as_slice())
            .unwrap_or(&[])
    }

    /// D36：MCP 对外基址（RFC 9728 元数据与 `WWW-Authenticate`）。
    pub fn mcp_public_url(&self) -> Option<&str> {
        self.mcp.as_ref()?.public_url.as_deref()
    }

    /// 配置合法性校验（daemon 启动前调用）。
    pub fn validate(&self) -> Result<(), ConfigError> {
        self.validate_tls()?;
        self.validate_ref_bindings()?;
        let tokens = self.auth_tokens();
        let keys = self.auth_keys()?;
        self.validate_acl(&tokens, &keys)?;
        self.validate_mcp(&tokens, &keys)
    }

    fn validate_ref_bindings(&self) -> Result<(), ConfigError> {
        if self
            .ref_bindings
            .as_ref()
            .is_some_and(|config| config.paths.is_empty())
        {
            return Err(ConfigError::Invalid(
                "[ref_bindings] 已配置但 paths 为空".to_owned(),
            ));
        }
        Ok(())
    }

    fn validate_tls(&self) -> Result<(), ConfigError> {
        let Some(tls) = &self.tls else {
            return Ok(());
        };
        let cert = tls
            .certificate
            .as_ref()
            .filter(|p| !p.as_os_str().is_empty());
        let key = tls
            .private_key
            .as_ref()
            .filter(|p| !p.as_os_str().is_empty());
        let (Some(_), Some(_)) = (cert, key) else {
            return Err(ConfigError::Invalid(
                "[tls] 必须同时配置 certificate 与 private_key".to_owned(),
            ));
        };
        if self.listen_tcp.is_none() {
            return Err(ConfigError::Invalid(
                "[tls] 需要同时配置 listen_tcp（TLS 仅作用于 TCP 监听）".to_owned(),
            ));
        }
        Ok(())
    }

    fn validate_acl(
        &self,
        tokens: &BTreeMap<String, String>,
        keys: &BTreeMap<String, [u8; 32]>,
    ) -> Result<(), ConfigError> {
        let acl = self.acl();
        if acl.is_empty() {
            return Ok(());
        }
        if tokens.is_empty() && keys.is_empty() {
            return Err(ConfigError::Invalid(
                "配置了 [auth.acl] 但未配置 [auth.tokens] 或 [auth.keys]：无法确定调用者身份"
                    .to_owned(),
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

    fn validate_mcp(
        &self,
        tokens: &BTreeMap<String, String>,
        keys: &BTreeMap<String, [u8; 32]>,
    ) -> Result<(), ConfigError> {
        let mcp_enabled = self.mcp.as_ref().is_some_and(|m| m.enabled);
        if let Some(url) = self.mcp_public_url() {
            validate_public_url(url)?;
        }
        let Some(settings) = self.oidc_settings() else {
            if mcp_enabled && !keys.is_empty() && tokens.is_empty() {
                return Err(ConfigError::Invalid(
                    "[mcp].enabled 且配置了 [auth.keys] 但无 [auth.tokens]/[mcp.oidc]：\
                     /mcp 仅凭公钥无法验证调用者"
                        .to_owned(),
                ));
            }
            return Ok(());
        };
        if !mcp_enabled {
            return Err(ConfigError::Invalid(
                "配置了 [mcp.oidc] 但 [mcp].enabled 为 false".to_owned(),
            ));
        }
        validate_oidc(&settings, self.mcp_public_url())
    }
}

/// D34/D36：参与者名（claim 映射结果）字符白名单。
pub fn valid_principal(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn default_algs() -> Vec<String> {
    vec!["RS256".to_owned(), "ES256".to_owned(), "EdDSA".to_owned()]
}

fn decode_public_key(encoded: &str) -> Option<[u8; 32]> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(encoded.trim()))
        .ok()?;
    bytes.try_into().ok()
}

fn validate_public_url(url: &str) -> Result<(), ConfigError> {
    let uri: http::Uri = url
        .parse()
        .map_err(|_| ConfigError::Invalid("[mcp].public_url 不是合法 URL".to_owned()))?;
    match uri.scheme_str() {
        Some("http") | Some("https") => {}
        _ => {
            return Err(ConfigError::Invalid(
                "[mcp].public_url 必须以 http:// 或 https:// 开头".to_owned(),
            ));
        }
    }
    if uri.authority().is_none() {
        return Err(ConfigError::Invalid(
            "[mcp].public_url 缺少主机名".to_owned(),
        ));
    }
    if uri.query().is_some()
        || uri
            .path_and_query()
            .is_some_and(|p| p.as_str().contains('#'))
    {
        return Err(ConfigError::Invalid(
            "[mcp].public_url 不得包含查询串或片段".to_owned(),
        ));
    }
    if !matches!(uri.path(), "" | "/") {
        return Err(ConfigError::Invalid(
            "[mcp].public_url 路径必须为空或 /".to_owned(),
        ));
    }
    Ok(())
}

fn validate_oidc(settings: &OidcSettings, public_url: Option<&str>) -> Result<(), ConfigError> {
    if public_url.is_none() {
        return Err(ConfigError::Invalid(
            "启用 [mcp.oidc] 时必须配置 [mcp].public_url".to_owned(),
        ));
    }
    if settings.issuer.trim().is_empty() {
        return Err(ConfigError::Invalid(
            "[mcp.oidc].issuer 不能为空".to_owned(),
        ));
    }
    validate_idp_url(
        "[mcp.oidc].issuer",
        &settings.issuer,
        settings.allow_insecure_http,
    )?;
    if settings.audience.trim().is_empty() {
        return Err(ConfigError::Invalid(
            "[mcp.oidc].audience 不能为空".to_owned(),
        ));
    }
    if settings.principal_claim.trim().is_empty() {
        return Err(ConfigError::Invalid(
            "[mcp.oidc].principal_claim 不能为空".to_owned(),
        ));
    }
    if !settings.discovery {
        let jwks = settings.jwks_uri.as_deref().unwrap_or("");
        if jwks.trim().is_empty() {
            return Err(ConfigError::Invalid(
                "[mcp.oidc].discovery = false 时必须配置 jwks_uri".to_owned(),
            ));
        }
    }
    if let Some(jwks) = settings
        .jwks_uri
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        validate_idp_url("[mcp.oidc].jwks_uri", jwks, settings.allow_insecure_http)?;
    }
    let allowed = ["RS256", "ES256", "EdDSA"];
    let unknown: BTreeSet<_> = settings
        .allowed_algs
        .iter()
        .map(String::as_str)
        .filter(|a| !allowed.contains(a))
        .collect();
    if !unknown.is_empty() {
        return Err(ConfigError::Invalid(format!(
            "[mcp.oidc].allowed_algs 含不支持的算法：{}",
            unknown.into_iter().collect::<Vec<_>>().join(", ")
        )));
    }
    if settings.allowed_algs.is_empty() {
        return Err(ConfigError::Invalid(
            "[mcp.oidc].allowed_algs 不能为空".to_owned(),
        ));
    }
    for (claim, principal) in &settings.principal_map {
        if claim.is_empty() {
            return Err(ConfigError::Invalid(
                "[mcp.oidc.principal_map] 的 claim 键不能为空".to_owned(),
            ));
        }
        if !valid_principal(principal) {
            return Err(ConfigError::Invalid(format!(
                "[mcp.oidc.principal_map] 映射结果 {principal:?} 非法：\
                 仅允许 [A-Za-z0-9_-]，长度 1..=128"
            )));
        }
    }
    Ok(())
}

fn validate_idp_url(field: &str, url: &str, allow_http: bool) -> Result<(), ConfigError> {
    let uri: http::Uri = url
        .parse()
        .map_err(|_| ConfigError::Invalid(format!("{field} 不是合法 URL")))?;
    let ok = match uri.scheme_str() {
        Some("https") => true,
        Some("http") => allow_http,
        _ => false,
    };
    if !ok || uri.authority().is_none() {
        return Err(ConfigError::Invalid(format!(
            "{field} 必须是 https:// 绝对 URL{}",
            if allow_http {
                "（或已显式允许 http://）"
            } else {
                ""
            }
        )));
    }
    if uri.query().is_some() {
        return Err(ConfigError::Invalid(format!("{field} 不得包含查询串")));
    }
    Ok(())
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

[ref_bindings]
paths = ["/etc/pact/refs/", "se_lifecycle.refs.yaml"]

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
        assert_eq!(
            c.ref_binding_paths(),
            [
                PathBuf::from("/etc/pact/refs/"),
                PathBuf::from("se_lifecycle.refs.yaml")
            ]
        );
        assert!(c.read_audit());
        assert_eq!(c.auth_tokens()["architect"], "sekret");
        assert_eq!(c.acl()["architect"], vec!["proj/*", "other/default"]);
        c.validate().expect("完整配置合法");
    }

    #[test]
    fn empty_ref_bindings_paths_are_rejected() {
        let text = "[ref_bindings]\npaths = []\n";
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pact.toml");
        std::fs::write(&path, text).unwrap();
        let c = Config::load(&path).unwrap();
        assert!(matches!(c.validate(), Err(ConfigError::Invalid(_))));
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

    fn parse(text: &str) -> Config {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pact.toml");
        std::fs::write(&path, text).unwrap();
        Config::load(&path).unwrap()
    }

    #[test]
    fn tls_requires_cert_and_key() {
        let c =
            parse("listen_tcp = \"127.0.0.1:8787\"\n\n[tls]\ncertificate = \"/tmp/cert.pem\"\n");
        assert!(matches!(c.validate(), Err(ConfigError::Invalid(_))));
        let c = parse(
            "listen_tcp = \"127.0.0.1:8787\"\n\n[tls]\ncertificate = \"/tmp/cert.pem\"\nprivate_key = \"/tmp/key.pem\"\n",
        );
        c.validate().expect("成对配置合法");
    }

    #[test]
    fn tls_requires_listen_tcp() {
        let c = parse("[tls]\ncertificate = \"/tmp/cert.pem\"\nprivate_key = \"/tmp/key.pem\"\n");
        assert!(matches!(c.validate(), Err(ConfigError::Invalid(_))));
    }

    #[test]
    fn keys_decode_base64_32() {
        let key = base64::engine::general_purpose::STANDARD.encode([7u8; 32]);
        let c = parse(&format!("[auth.keys]\narchitect = \"{key}\"\n"));
        let keys = c.auth_keys().unwrap();
        assert_eq!(keys["architect"], [7u8; 32]);
        c.validate().expect("只有公钥且未开 MCP 时合法");
    }

    #[test]
    fn key_errors_do_not_echo_value() {
        let secret = "not-a-valid-key-material";
        let c = parse(&format!("[auth.keys]\narchitect = \"{secret}\"\n"));
        let err = c.validate().unwrap_err().to_string();
        assert!(!err.contains(secret), "错误信息不得回显密钥：{err}");
        assert!(err.contains("architect"));
    }

    #[test]
    fn mcp_with_keys_requires_tokens_or_oidc() {
        let key = base64::engine::general_purpose::STANDARD.encode([1u8; 32]);
        let c = parse(&format!(
            "[auth.keys]\narchitect = \"{key}\"\n\n[mcp]\nenabled = true\n"
        ));
        assert!(matches!(c.validate(), Err(ConfigError::Invalid(_))));
        let c = parse(&format!(
            "[auth.keys]\narchitect = \"{key}\"\n\n[mcp]\nenabled = true\npublic_url = \"https://pact.example.com\"\n\n[mcp.oidc]\nissuer = \"https://idp.example.com\"\naudience = \"pact\"\n"
        ));
        c.validate().expect("OIDC 可替代静态令牌保护 /mcp");
    }

    #[test]
    fn oidc_requires_public_url_and_issuer() {
        let c = parse(
            "[mcp]\nenabled = true\n\n[mcp.oidc]\nissuer = \"https://idp.example.com\"\naudience = \"pact\"\n",
        );
        assert!(matches!(c.validate(), Err(ConfigError::Invalid(_))));
        let c = parse(
            "[mcp]\nenabled = true\npublic_url = \"https://pact.example.com\"\n\n[mcp.oidc]\naudience = \"pact\"\n",
        );
        assert!(matches!(c.validate(), Err(ConfigError::Invalid(_))));
    }

    #[test]
    fn oidc_discovery_false_requires_jwks_uri() {
        let c = parse(
            "[mcp]\nenabled = true\npublic_url = \"https://pact.example.com\"\n\n[mcp.oidc]\nissuer = \"https://idp.example.com\"\naudience = \"pact\"\ndiscovery = false\n",
        );
        assert!(matches!(c.validate(), Err(ConfigError::Invalid(_))));
        let c = parse(
            "[mcp]\nenabled = true\npublic_url = \"https://pact.example.com\"\n\n[mcp.oidc]\nissuer = \"https://idp.example.com\"\naudience = \"pact\"\ndiscovery = false\njwks_uri = \"https://idp.example.com/jwks\"\n",
        );
        c.validate().expect("显式 jwks_uri 合法");
    }

    #[test]
    fn oidc_rejects_unknown_alg() {
        let c = parse(
            "[mcp]\nenabled = true\npublic_url = \"https://pact.example.com\"\n\n[mcp.oidc]\nissuer = \"https://idp.example.com\"\naudience = \"pact\"\nallowed_algs = [\"HS256\"]\n",
        );
        assert!(matches!(c.validate(), Err(ConfigError::Invalid(_))));
    }

    #[test]
    fn oidc_requires_enabled_mcp() {
        let c = parse(
            "[mcp]\nenabled = false\npublic_url = \"https://pact.example.com\"\n\n[mcp.oidc]\nissuer = \"https://idp.example.com\"\naudience = \"pact\"\n",
        );
        assert!(matches!(c.validate(), Err(ConfigError::Invalid(_))));
    }

    #[test]
    fn oidc_issuer_must_be_https_unless_allowed() {
        let c = parse(
            "[mcp]\nenabled = true\npublic_url = \"https://pact.example.com\"\n\n[mcp.oidc]\nissuer = \"http://idp.example.com\"\naudience = \"pact\"\n",
        );
        assert!(matches!(c.validate(), Err(ConfigError::Invalid(_))));
        let c = parse(
            "[mcp]\nenabled = true\npublic_url = \"http://127.0.0.1:9999\"\n\n[mcp.oidc]\nissuer = \"http://127.0.0.1:9\"\naudience = \"pact\"\nallow_insecure_http = true\n",
        );
        c.validate().expect("显式允许 http 时合法");
    }

    #[test]
    fn public_url_is_validated() {
        for url in ["ftp://x", "https://", "https://x/path", "https://x/?a=1"] {
            let c = parse(&format!("[mcp]\nenabled = true\npublic_url = \"{url}\"\n"));
            assert!(
                matches!(c.validate(), Err(ConfigError::Invalid(_))),
                "public_url {url:?} 应被拒绝"
            );
        }
    }
}
