//! D34/D35/D36：身份中间件。
//!
//! - [`enforce_identity`]：剥离伪造可信头、识别进程内能力令牌、校验 Ed25519 签名；
//! - [`require_mcp_auth`]：`/mcp` 的 OIDC JWT / 静态 Bearer 鉴权。

use crate::server::{ApiError, ServerState};
use crate::sign::{
    HEADER_INTERNAL_AUTH, HEADER_NONCE, HEADER_PARTICIPANT, HEADER_SIGNATURE, HEADER_TIMESTAMP,
    HEADER_VERIFIED_PRINCIPAL,
};
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::sync::Arc;

/// 豁免签名的端点（精确匹配）。
const EXEMPT_PATHS: &[&str] = &[
    "/health",
    "/ready",
    "/metrics",
    "/mcp",
    "/.well-known/oauth-protected-resource",
];

fn is_exempt(path: &str) -> bool {
    EXEMPT_PATHS.contains(&path)
}

/// 签名/身份中间件（所有请求都会经过，用于剥离伪造头）。
pub async fn enforce_identity(
    State(state): State<Arc<ServerState>>,
    mut req: Request,
    next: Next,
) -> Response {
    let internal = req
        .headers()
        .get(HEADER_INTERNAL_AUTH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| ct_eq(v, &state.internal_token_hex()));
    let verified = req
        .headers()
        .get(HEADER_VERIFIED_PRINCIPAL)
        .and_then(|v| v.to_str().ok())
        .map(ToOwned::to_owned);
    // 外部请求不得自带可信身份头；内部调用由上面的能力令牌判定。
    req.headers_mut().remove(HEADER_VERIFIED_PRINCIPAL);
    req.headers_mut().remove(HEADER_INTERNAL_AUTH);

    if internal {
        if let Some(p) = verified {
            if !crate::config::valid_principal(&p) {
                return unauthorized("内部身份非法");
            }
            if let Ok(v) = HeaderValue::from_str(&p) {
                req.headers_mut().insert(HEADER_VERIFIED_PRINCIPAL, v);
            }
        }
        return next.run(req).await;
    }

    if !state.signing_enabled() || is_exempt(req.uri().path()) {
        return next.run(req).await;
    }

    let (mut parts, body) = req.into_parts();
    let bytes = match axum::body::to_bytes(body, crate::sign::MAX_SIGNATURE_BODY).await {
        Ok(b) => b,
        Err(_) => {
            return ApiError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "PayloadTooLarge",
                "请求体超过签名校验上限（8 MiB）".to_owned(),
            )
            .into_response();
        }
    };
    let headers = parts.headers.clone();
    let parsed = match parse_sign_headers(&headers) {
        Ok(h) => h,
        Err(msg) => return unauthorized(&msg),
    };
    let now = match unix_now() {
        Ok(n) => n,
        Err(msg) => return unauthorized(&msg),
    };
    if now.abs_diff(parsed.timestamp) > crate::sign::TIMESTAMP_WINDOW_SECS {
        return unauthorized("时间戳超出允许窗口");
    }
    let Some(key) = state.auth_key(&parsed.participant) else {
        return unauthorized("参与者未登记公钥");
    };
    let method = parts.method.as_str().to_owned();
    let path_and_query = parts
        .uri
        .path_and_query()
        .map(|p| p.as_str().to_owned())
        .unwrap_or_else(|| parts.uri.path().to_owned());
    if crate::sign::verify(key, &parsed, &method, &path_and_query, &bytes, now).is_err() {
        return unauthorized("签名校验失败");
    }
    // 先验签再消耗 nonce：未通过签名的请求不得污染防重放缓存（防 DoS 放大）。
    if !state.accept_nonce(&parsed.nonce).await {
        return ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "TooManyRequests",
            "nonce 重复或防重放缓存已满".to_owned(),
        )
        .into_response();
    }
    if let Ok(v) = HeaderValue::from_str(&parsed.participant) {
        parts.headers.insert(HEADER_VERIFIED_PRINCIPAL, v);
    }
    next.run(Request::from_parts(parts, Body::from(bytes)))
        .await
}

/// `/mcp` 鉴权：OIDC JWT → 静态 Bearer → 信任模式（仅未配置任何身份时）。
pub async fn require_mcp_auth(
    State(state): State<Arc<ServerState>>,
    mut req: Request,
    next: Next,
) -> Response {
    let supplied = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(ToOwned::to_owned);
    req.headers_mut().remove(HEADER_VERIFIED_PRINCIPAL);

    let oidc = state.oidc().cloned();
    let principal = match supplied {
        None => {
            if oidc.is_none() && !state.has_static_tokens() {
                req.headers()
                    .get(HEADER_VERIFIED_PRINCIPAL)
                    .and_then(|v| v.to_str().ok())
                    .map(ToOwned::to_owned)
            } else {
                return mcp_unauthorized(&state, "缺少 Bearer 令牌");
            }
        }
        Some(token) => {
            if let Some(validator) = &oidc {
                if is_jwt(&token) {
                    match validator.validate(&token).await {
                        Ok(p) => Some(p),
                        Err(crate::oidc::OidcError::Unavailable(e)) => {
                            return ApiError::new(
                                StatusCode::SERVICE_UNAVAILABLE,
                                "ServiceUnavailable",
                                e,
                            )
                            .into_response();
                        }
                        Err(_) => return mcp_unauthorized(&state, "OIDC 令牌校验失败"),
                    }
                } else if state.has_static_tokens() && validator.allow_static_tokens() {
                    state.static_token_matches(&token)
                } else {
                    return mcp_unauthorized(&state, "令牌格式不受支持");
                }
            } else if state.has_static_tokens() {
                state.static_token_matches(&token)
            } else {
                return mcp_unauthorized(&state, "未配置可用的身份校验");
            }
        }
    };

    if let Some(p) = principal {
        if !crate::config::valid_principal(&p) {
            return mcp_unauthorized(&state, "身份非法");
        }
        if let Ok(v) = HeaderValue::from_str(&p) {
            req.headers_mut().insert(HEADER_VERIFIED_PRINCIPAL, v);
        }
    }
    next.run(req).await
}

fn mcp_unauthorized(state: &ServerState, msg: &str) -> Response {
    let mut resp =
        ApiError::new(StatusCode::UNAUTHORIZED, "Unauthorized", msg.to_owned()).into_response();
    if let Some(url) = state.public_url() {
        let header = format!(
            "Bearer resource_metadata=\"{}/.well-known/oauth-protected-resource\"",
            url.trim_end_matches('/')
        );
        if let Ok(v) = HeaderValue::from_str(&header) {
            resp.headers_mut().insert("www-authenticate", v);
        }
    }
    resp
}

fn unauthorized(msg: &str) -> Response {
    ApiError::new(StatusCode::UNAUTHORIZED, "Unauthorized", msg.to_owned()).into_response()
}

fn is_jwt(token: &str) -> bool {
    token.split('.').count() == 3
}

fn parse_sign_headers(headers: &HeaderMap) -> Result<crate::sign::SignHeaders, String> {
    let get = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(ToOwned::to_owned)
    };
    let participant = get(HEADER_PARTICIPANT).ok_or("缺少签名头 x-pact-participant")?;
    let timestamp = get(HEADER_TIMESTAMP).ok_or("缺少签名头 x-pact-timestamp")?;
    let nonce = get(HEADER_NONCE).ok_or("缺少签名头 x-pact-nonce")?;
    let signature = get(HEADER_SIGNATURE).ok_or("缺少签名头 x-pact-signature")?;
    let timestamp: u64 = timestamp
        .parse()
        .map_err(|_| "签名头 x-pact-timestamp 非法")?;
    if nonce.is_empty() || nonce.len() > 128 {
        return Err("签名头 x-pact-nonce 非法".to_owned());
    }
    Ok(crate::sign::SignHeaders {
        participant,
        timestamp,
        nonce,
        signature,
    })
}

fn unix_now() -> Result<u64, String> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|_| "系统时钟早于 Unix 纪元".to_owned())
}

fn ct_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut diff = a.len() ^ b.len();
    for (x, y) in a.iter().zip(b) {
        diff |= usize::from(x ^ y);
    }
    diff == 0
}
