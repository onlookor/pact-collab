//! D36：MCP 的 OIDC 资源服务器（仅验证 IdP 签发的 JWT）。
//!
//! 发现文档与 JWKS 均缓存并有界；错误信息不包含令牌或 claim 原值。

use crate::config::{valid_principal, OidcSettings};
use bytes::Bytes;
use http::Request;
use http_body_util::{BodyExt, Full};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use jsonwebtoken::jwk::{AlgorithmParameters, EllipticCurve, Jwk, JwkSet};
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, TokenData, Validation};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

const JWKS_TTL: Duration = Duration::from_secs(300);
const DISCOVERY_TTL: Duration = Duration::from_secs(300);
const FORCE_REFRESH_MIN: Duration = Duration::from_secs(30);
const FETCH_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_KEYS: usize = 64;

#[derive(Debug)]
pub enum OidcError {
    /// 令牌不合法（401）。
    Invalid(String),
    /// IdP 不可达或响应非法（503）。
    Unavailable(String),
}

impl std::fmt::Display for OidcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OidcError::Invalid(e) => write!(f, "OIDC 令牌非法：{e}"),
            OidcError::Unavailable(e) => write!(f, "OIDC 身份提供方不可用：{e}"),
        }
    }
}

impl std::error::Error for OidcError {}

#[derive(Default)]
struct Cache {
    discovery_uri: Option<String>,
    discovery_at: Option<Instant>,
    jwks: Option<JwkSet>,
    jwks_at: Option<Instant>,
    last_force: Option<Instant>,
}

/// OIDC 验证器：发现 + JWKS 缓存 + JWT 校验 + claim 映射。
pub struct OidcValidator {
    settings: OidcSettings,
    issuer: String,
    client: hyper_util::client::legacy::Client<
        hyper_rustls::HttpsConnector<HttpConnector>,
        Full<Bytes>,
    >,
    cache: Mutex<Cache>,
}

impl OidcValidator {
    pub fn new(settings: OidcSettings) -> Result<Arc<Self>, OidcError> {
        let tls = crate::tls::client_config(settings.ca.as_deref())
            .map_err(|e| OidcError::Unavailable(e.to_string()))?;
        let conn = hyper_rustls::HttpsConnectorBuilder::new()
            .with_tls_config(tls)
            .https_or_http()
            .enable_http1()
            .build();
        let client = hyper_util::client::legacy::Client::builder(TokioExecutor::new()).build(conn);
        let issuer = settings.issuer.trim_end_matches('/').to_owned();
        Ok(Arc::new(Self {
            settings,
            issuer,
            client,
            cache: Mutex::new(Cache::default()),
        }))
    }

    /// 校验 JWT 并返回映射后的 PACT 参与者。
    pub async fn validate(&self, token: &str) -> Result<String, OidcError> {
        let header = decode_header(token).map_err(|_| OidcError::Invalid("JWT 头部非法".into()))?;
        let alg = header.alg;
        let alg_name =
            alg_name(alg).ok_or_else(|| OidcError::Invalid("不支持的签名算法".to_owned()))?;
        if !self.settings.allowed_algs.iter().any(|a| a == alg_name) {
            return Err(OidcError::Invalid("签名算法不在白名单内".to_owned()));
        }
        if has_crit(token) {
            return Err(OidcError::Invalid("不支持 crit 头部参数".to_owned()));
        }

        let key = match self.select_key(header.kid.as_deref(), alg).await? {
            Some(k) => k,
            None => {
                // 未知 kid：允许一次限频重取。
                let refreshed = self.jwks(true).await?;
                match pick_key(&refreshed, header.kid.as_deref(), alg) {
                    Some(k) => k,
                    None => return Err(OidcError::Invalid("未知签名密钥".to_owned())),
                }
            }
        };

        let decoding = DecodingKey::from_jwk(&key)
            .map_err(|_| OidcError::Invalid("JWKS 公钥不可用".to_owned()))?;
        let mut validation = Validation::new(alg);
        validation.leeway = 60;
        validation.validate_nbf = true;
        validation.set_audience(&[self.settings.audience.as_str()]);
        validation.set_issuer(&[self.issuer.as_str()]);
        validation.required_spec_claims.insert("aud".to_owned());
        validation.required_spec_claims.insert("iss".to_owned());

        let data: TokenData<serde_json::Value> = decode(token, &decoding, &validation)
            .map_err(|_| OidcError::Invalid("JWT 校验失败".to_owned()))?;
        let claims = data.claims;

        // `sub` 必须存在且非空（即使映射用的是其他 claim）。
        let sub = claim_string(&claims, "sub")
            .ok_or_else(|| OidcError::Invalid("缺少 sub".to_owned()))?;
        if sub.is_empty() {
            return Err(OidcError::Invalid("sub 为空".to_owned()));
        }

        let raw = claim_string(&claims, &self.settings.principal_claim)
            .ok_or_else(|| OidcError::Invalid("缺少身份 claim".to_owned()))?;
        let principal = self
            .settings
            .principal_map
            .get(&raw)
            .cloned()
            .unwrap_or(raw);
        if !valid_principal(&principal) {
            return Err(OidcError::Invalid("身份映射结果非法".to_owned()));
        }
        Ok(principal)
    }

    /// 令牌表非空时是否仍接受静态 Bearer（D36，缺省 true）。
    pub fn allow_static_tokens(&self) -> bool {
        self.settings.allow_static_tokens
    }

    /// RFC 9728 受保护资源元数据。
    pub fn metadata(&self, public_url: &str) -> serde_json::Value {
        serde_json::json!({
            "resource": public_url,
            "authorization_servers": [self.issuer],
            "bearer_methods_supported": ["header"],
        })
    }

    async fn select_key(
        &self,
        kid: Option<&str>,
        alg: Algorithm,
    ) -> Result<Option<Jwk>, OidcError> {
        let set = self.jwks(false).await?;
        Ok(pick_key(&set, kid, alg))
    }

    async fn jwks(&self, force: bool) -> Result<JwkSet, OidcError> {
        let mut cache = self.cache.lock().await;
        let now = Instant::now();
        if !force {
            if let (Some(set), Some(at)) = (&cache.jwks, cache.jwks_at) {
                if now.duration_since(at) < JWKS_TTL {
                    return Ok(set.clone());
                }
            }
        } else if let Some(at) = cache.last_force {
            if now.duration_since(at) < FORCE_REFRESH_MIN {
                if let Some(set) = &cache.jwks {
                    return Ok(set.clone());
                }
            }
        }

        let uri = match self.discovery_uri(&mut cache, now).await? {
            Some(u) => u,
            None => return Err(OidcError::Unavailable("缺少 jwks_uri".to_owned())),
        };
        let value = self.get_json(&uri).await?;
        let mut set: JwkSet = serde_json::from_value(value)
            .map_err(|_| OidcError::Unavailable("JWKS 格式非法".to_owned()))?;
        if set.keys.is_empty() {
            return Err(OidcError::Unavailable("JWKS 不含密钥".to_owned()));
        }
        set.keys.truncate(MAX_KEYS);
        cache.jwks = Some(set.clone());
        cache.jwks_at = Some(now);
        if force {
            cache.last_force = Some(now);
        }
        Ok(set)
    }

    async fn discovery_uri(
        &self,
        cache: &mut Cache,
        now: Instant,
    ) -> Result<Option<String>, OidcError> {
        if let Some(uri) = &self.settings.jwks_uri {
            if !uri.trim().is_empty() {
                if !self.jwks_uri_allowed(uri) {
                    return Err(OidcError::Unavailable(
                        "jwks_uri 必须是 https://（或已显式允许 http://）".to_owned(),
                    ));
                }
                return Ok(Some(uri.clone()));
            }
        }
        if !self.settings.discovery {
            return Err(OidcError::Unavailable(
                "discovery 已关闭且未配置 jwks_uri".to_owned(),
            ));
        }
        if let (Some(uri), Some(at)) = (&cache.discovery_uri, cache.discovery_at) {
            if now.duration_since(at) < DISCOVERY_TTL {
                return Ok(Some(uri.clone()));
            }
        }
        let url = format!("{}/.well-known/openid-configuration", self.issuer);
        let value = self.get_json(&url).await?;
        let uri = value
            .get("jwks_uri")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| OidcError::Unavailable("发现文档缺少 jwks_uri".to_owned()))?
            .to_owned();
        // 发现文档由 IdP 提供：校验协议，避免降级到明文或非 HTTP(S) 目标。
        if !self.jwks_uri_allowed(&uri) {
            return Err(OidcError::Unavailable(
                "发现文档的 jwks_uri 必须是 https://（或已显式允许 http://）".to_owned(),
            ));
        }
        cache.discovery_uri = Some(uri.clone());
        cache.discovery_at = Some(now);
        Ok(Some(uri))
    }

    fn jwks_uri_allowed(&self, uri: &str) -> bool {
        match uri.split_once("://") {
            Some((scheme, rest)) if !rest.trim().is_empty() => {
                scheme.eq_ignore_ascii_case("https")
                    || (self.settings.allow_insecure_http && scheme.eq_ignore_ascii_case("http"))
            }
            _ => false,
        }
    }

    async fn get_json(&self, uri: &str) -> Result<serde_json::Value, OidcError> {
        let req = Request::builder()
            .uri(uri)
            .header("accept", "application/json")
            .body(Full::new(Bytes::new()))
            .map_err(|_| OidcError::Unavailable("IdP URL 非法".to_owned()))?;
        let resp = tokio::time::timeout(FETCH_TIMEOUT, self.client.request(req))
            .await
            .map_err(|_| OidcError::Unavailable("IdP 请求超时".to_owned()))?
            .map_err(|_| OidcError::Unavailable("IdP 请求失败".to_owned()))?;
        if !resp.status().is_success() {
            return Err(OidcError::Unavailable(format!(
                "IdP 返回 {}",
                resp.status()
            )));
        }
        let body = tokio::time::timeout(FETCH_TIMEOUT, resp.into_body().collect())
            .await
            .map_err(|_| OidcError::Unavailable("IdP 响应超时".to_owned()))?
            .map_err(|_| OidcError::Unavailable("IdP 响应读取失败".to_owned()))?
            .to_bytes();
        if body.len() > 1024 * 1024 {
            return Err(OidcError::Unavailable("IdP 响应过大".to_owned()));
        }
        serde_json::from_slice(&body)
            .map_err(|_| OidcError::Unavailable("IdP 响应不是合法 JSON".to_owned()))
    }
}

fn pick_key(set: &JwkSet, kid: Option<&str>, alg: Algorithm) -> Option<Jwk> {
    let candidates: Vec<&Jwk> = match kid {
        Some(kid) => set.find(kid).into_iter().collect(),
        None if set.keys.len() == 1 => set.keys.iter().collect(),
        None => Vec::new(),
    };
    candidates
        .into_iter()
        .find(|k| matches_alg(k, alg))
        .cloned()
}

fn matches_alg(jwk: &Jwk, alg: Algorithm) -> bool {
    match (alg, &jwk.algorithm) {
        (Algorithm::RS256, AlgorithmParameters::RSA(_)) => true,
        (Algorithm::ES256, AlgorithmParameters::EllipticCurve(p)) => p.curve == EllipticCurve::P256,
        (Algorithm::EdDSA, AlgorithmParameters::OctetKeyPair(p)) => {
            p.curve == EllipticCurve::Ed25519
        }
        _ => false,
    }
}

fn alg_name(alg: Algorithm) -> Option<&'static str> {
    match alg {
        Algorithm::RS256 => Some("RS256"),
        Algorithm::ES256 => Some("ES256"),
        Algorithm::EdDSA => Some("EdDSA"),
        _ => None,
    }
}

/// 检查 JWT 头部是否包含非空 `crit`（拒绝扩展关键头）。
fn has_crit(token: &str) -> bool {
    use base64::Engine as _;
    let Some(part) = token.split('.').next() else {
        return true;
    };
    let Ok(bytes) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(part) else {
        return true;
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return true;
    };
    value
        .get("crit")
        .is_some_and(|v| !matches!(v, serde_json::Value::Null))
}

fn claim_string(claims: &serde_json::Value, name: &str) -> Option<String> {
    match claims.get(name)? {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Array(items) if items.len() == 1 => {
            items[0].as_str().map(ToOwned::to_owned)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jwk(kid: &str) -> Jwk {
        let value = serde_json::json!({
            "kty": "OKP",
            "crv": "Ed25519",
            "kid": kid,
            "x": "11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo",
            "alg": "EdDSA",
        });
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn picks_key_by_kid_and_alg() {
        let set = JwkSet {
            keys: vec![jwk("a"), jwk("b")],
        };
        assert_eq!(
            pick_key(&set, Some("b"), Algorithm::EdDSA)
                .unwrap()
                .common
                .key_id,
            Some("b".to_owned())
        );
        assert!(pick_key(&set, Some("missing"), Algorithm::EdDSA).is_none());
        assert!(pick_key(&set, None, Algorithm::EdDSA).is_none());
        assert!(pick_key(&set, Some("a"), Algorithm::RS256).is_none());
    }

    #[test]
    fn crit_header_is_rejected() {
        use base64::Engine as _;
        let header = serde_json::json!({"alg":"EdDSA","crit":["exp"],"typ":"JWT"});
        let encoded =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(header.to_string().as_bytes());
        let token = format!("{encoded}.payload.sig");
        assert!(has_crit(&token));
        let header = serde_json::json!({"alg":"EdDSA","typ":"JWT"});
        let encoded =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(header.to_string().as_bytes());
        assert!(!has_crit(&format!("{encoded}.payload.sig")));
    }

    #[test]
    fn claims_map_single_value_arrays() {
        let claims = serde_json::json!({"sub":"u1","groups":["architect"]});
        assert_eq!(
            claim_string(&claims, "groups").as_deref(),
            Some("architect")
        );
        assert!(claim_string(&claims, "missing").is_none());
    }
}
