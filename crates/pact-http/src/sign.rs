//! D34：Ed25519 请求签名（`PACT-SIGNATURE-V1`）。
//!
//! 规范化串覆盖方法、路径+查询、请求体摘要、时间戳、nonce、参与者；
//! 验签使用 dalek 的恒定时间实现。

use base64::Engine as _;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// 时间戳允许的时钟偏移（秒）。
pub const TIMESTAMP_WINDOW_SECS: u64 = 300;
/// 验签请求体上限（8 MiB）。
pub const MAX_SIGNATURE_BODY: usize = 8 * 1024 * 1024;
/// nonce 缓存 TTL（秒）。
pub const NONCE_TTL_SECS: u64 = 300;
/// nonce 缓存上限。
pub const NONCE_CAPACITY: usize = 8192;

pub const HEADER_PARTICIPANT: &str = "x-pact-participant";
pub const HEADER_TIMESTAMP: &str = "x-pact-timestamp";
pub const HEADER_NONCE: &str = "x-pact-nonce";
pub const HEADER_SIGNATURE: &str = "x-pact-signature";
/// 服务端写入的可信身份头。
pub const HEADER_VERIFIED_PRINCIPAL: &str = "x-pact-verified-principal";
/// 进程内 MCP → 本地 REST 的能力头。
pub const HEADER_INTERNAL_AUTH: &str = "x-pact-internal-auth";

#[derive(Debug)]
pub enum SignError {
    InvalidKey,
    InvalidParticipant,
    MissingHeader(&'static str),
    InvalidHeader(&'static str),
    TimestampOutOfWindow,
    VerifyFailed,
}

impl std::fmt::Display for SignError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SignError::InvalidKey => write!(f, "Ed25519 种子非法：须为 base64(32 字节)"),
            SignError::InvalidParticipant => {
                write!(f, "参与者名非法：仅允许 [A-Za-z0-9_-]，长度 1..=128")
            }
            SignError::MissingHeader(h) => write!(f, "缺少签名头 {h}"),
            SignError::InvalidHeader(h) => write!(f, "签名头 {h} 非法"),
            SignError::TimestampOutOfWindow => write!(f, "时间戳超出允许窗口"),
            SignError::VerifyFailed => write!(f, "签名校验失败"),
        }
    }
}

impl std::error::Error for SignError {}

/// 请求携带的签名材料。
#[derive(Debug, Clone)]
pub struct SignHeaders {
    pub participant: String,
    pub timestamp: u64,
    pub nonce: String,
    pub signature: String,
}

/// 客户端签名身份（种子 = 32 字节 Ed25519 私钥）。
#[derive(Clone)]
pub struct SigningIdentity {
    participant: String,
    key: SigningKey,
}

impl SigningIdentity {
    pub fn from_seed_base64(participant: &str, seed: &str) -> Result<Self, SignError> {
        if !crate::config::valid_principal(participant) {
            return Err(SignError::InvalidParticipant);
        }
        let bytes = decode_base64(seed.trim()).ok_or(SignError::InvalidKey)?;
        let seed: [u8; 32] = bytes.try_into().map_err(|_| SignError::InvalidKey)?;
        Ok(Self {
            participant: participant.to_owned(),
            key: SigningKey::from_bytes(&seed),
        })
    }

    pub fn participant(&self) -> &str {
        &self.participant
    }

    /// base64 公钥（用于写入 daemon 的 `[auth.keys]`）。
    pub fn public_key_base64(&self) -> String {
        base64::engine::general_purpose::STANDARD.encode(self.key.verifying_key().to_bytes())
    }

    /// 生成请求签名头。
    pub fn headers(
        &self,
        method: &str,
        path_and_query: &str,
        body: &[u8],
        timestamp: u64,
        nonce: &str,
    ) -> SignHeaders {
        let msg = canonical(
            method,
            path_and_query,
            body,
            timestamp,
            nonce,
            &self.participant,
        );
        let sig = self.key.sign(msg.as_bytes());
        SignHeaders {
            participant: self.participant.clone(),
            timestamp,
            nonce: nonce.to_owned(),
            signature: base64::engine::general_purpose::STANDARD.encode(sig.to_bytes()),
        }
    }
}

/// 规范化签名串（无尾随换行）。
pub fn canonical(
    method: &str,
    path_and_query: &str,
    body: &[u8],
    timestamp: u64,
    nonce: &str,
    participant: &str,
) -> String {
    format!(
        "PACT-SIGNATURE-V1\n{method}\n{path_and_query}\n{}\n{timestamp}\n{nonce}\n{participant}",
        sha256_hex(body)
    )
}

pub fn sha256_hex(body: &[u8]) -> String {
    let digest = Sha256::digest(body);
    let mut out = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// 验签：时间窗 + Ed25519。nonce 去重由 [`NonceCache`] 负责。
pub fn verify(
    public_key: &[u8; 32],
    headers: &SignHeaders,
    method: &str,
    path_and_query: &str,
    body: &[u8],
    now: u64,
) -> Result<(), SignError> {
    if !crate::config::valid_principal(&headers.participant) {
        return Err(SignError::InvalidParticipant);
    }
    if now.abs_diff(headers.timestamp) > TIMESTAMP_WINDOW_SECS {
        return Err(SignError::TimestampOutOfWindow);
    }
    let sig_bytes =
        decode_base64(&headers.signature).ok_or(SignError::InvalidHeader(HEADER_SIGNATURE))?;
    let sig_bytes: [u8; 64] = sig_bytes
        .try_into()
        .map_err(|_| SignError::InvalidHeader(HEADER_SIGNATURE))?;
    let verifying = VerifyingKey::from_bytes(public_key).map_err(|_| SignError::InvalidKey)?;
    let msg = canonical(
        method,
        path_and_query,
        body,
        headers.timestamp,
        &headers.nonce,
        &headers.participant,
    );
    verifying
        .verify_strict(msg.as_bytes(), &Signature::from_bytes(&sig_bytes))
        .map_err(|_| SignError::VerifyFailed)
}

fn decode_base64(s: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(s))
        .ok()
}

/// 有界 nonce 缓存：TTL 300s、上限 8192；满即拒绝（429）。
pub struct NonceCache {
    ttl: Duration,
    capacity: usize,
    entries: HashMap<String, Instant>,
}

impl Default for NonceCache {
    fn default() -> Self {
        Self {
            ttl: Duration::from_secs(NONCE_TTL_SECS),
            capacity: NONCE_CAPACITY,
            entries: HashMap::new(),
        }
    }
}

impl NonceCache {
    pub fn with_limits(ttl: Duration, capacity: usize) -> Self {
        Self {
            ttl,
            capacity,
            entries: HashMap::new(),
        }
    }

    /// 首次出现返回 true 并记录；重复或缓存满返回 false。
    pub fn check_and_insert(&mut self, nonce: &str, now: Instant) -> bool {
        self.gc(now);
        if self.entries.contains_key(nonce) {
            return false;
        }
        if self.entries.len() >= self.capacity {
            return false;
        }
        self.entries.insert(nonce.to_owned(), now);
        true
    }

    fn gc(&mut self, now: Instant) {
        self.entries
            .retain(|_, at| now.duration_since(*at) < self.ttl);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

    #[test]
    fn empty_body_digest_is_known() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn canonical_layout_is_stable() {
        let msg = canonical("POST", "/domains/x:analyze", b"{}", 100, "n1", "architect");
        assert_eq!(
            msg,
            format!(
                "PACT-SIGNATURE-V1\nPOST\n/domains/x:analyze\n{}\n100\nn1\narchitect",
                sha256_hex(b"{}")
            )
        );
    }

    #[test]
    fn sign_and_verify_roundtrip() {
        let id = SigningIdentity::from_seed_base64("architect", SEED).unwrap();
        let pubkey: [u8; 32] = base64::engine::general_purpose::STANDARD
            .decode(id.public_key_base64())
            .unwrap()
            .try_into()
            .unwrap();
        let h = id.headers("POST", "/domains?x=1", b"{\"a\":1}", 1000, "nonce-1");
        verify(&pubkey, &h, "POST", "/domains?x=1", b"{\"a\":1}", 1001).unwrap();
    }

    #[test]
    fn tampering_is_rejected() {
        let id = SigningIdentity::from_seed_base64("architect", SEED).unwrap();
        let pubkey: [u8; 32] = base64::engine::general_purpose::STANDARD
            .decode(id.public_key_base64())
            .unwrap()
            .try_into()
            .unwrap();
        let h = id.headers("POST", "/domains", b"{}", 1000, "nonce-1");
        assert!(verify(&pubkey, &h, "POST", "/domains", b"{\"x\":1}", 1000).is_err());
        assert!(verify(&pubkey, &h, "POST", "/domains?x=1", b"{}", 1000).is_err());
        assert!(verify(&pubkey, &h, "PUT", "/domains", b"{}", 1000).is_err());
        let mut other = h.clone();
        other.participant = "reviewer".to_owned();
        assert!(verify(&pubkey, &other, "POST", "/domains", b"{}", 1000).is_err());
    }

    #[test]
    fn timestamp_window_is_enforced() {
        let id = SigningIdentity::from_seed_base64("architect", SEED).unwrap();
        let pubkey: [u8; 32] = base64::engine::general_purpose::STANDARD
            .decode(id.public_key_base64())
            .unwrap()
            .try_into()
            .unwrap();
        let h = id.headers("GET", "/health", b"", 1000, "n");
        assert!(verify(&pubkey, &h, "GET", "/health", b"", 1301).is_err());
        verify(&pubkey, &h, "GET", "/health", b"", 1299).unwrap();
    }

    #[test]
    fn nonce_replay_and_capacity() {
        let now = Instant::now();
        let mut cache = NonceCache::with_limits(Duration::from_secs(300), 2);
        assert!(cache.check_and_insert("a", now));
        assert!(!cache.check_and_insert("a", now));
        assert!(cache.check_and_insert("b", now));
        assert!(!cache.check_and_insert("c", now));
        let later = now + Duration::from_secs(301);
        assert!(cache.check_and_insert("c", later));
    }
}
