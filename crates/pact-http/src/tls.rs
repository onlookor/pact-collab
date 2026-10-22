//! D34：TCP 监听的 TLS 终止与客户端 TLS 配置。
//!
//! 私钥只读取、不打印；错误信息不包含密钥内容。

use crate::config::TlsConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::WebPkiClientVerifier;
use rustls::RootCertStore;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio_rustls::TlsAcceptor;

/// TLS 配置/加载错误。
#[derive(Debug)]
pub enum TlsError {
    Io(PathBuf, String),
    Pem(String),
    Rustls(String),
    Invalid(String),
}

impl std::fmt::Display for TlsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TlsError::Io(p, e) => write!(f, "读取 TLS 文件 {} 失败：{e}", p.display()),
            TlsError::Pem(e) => write!(f, "TLS PEM 解析失败：{e}"),
            TlsError::Rustls(e) => write!(f, "TLS 配置失败：{e}"),
            TlsError::Invalid(e) => write!(f, "TLS 配置非法：{e}"),
        }
    }
}

impl std::error::Error for TlsError {}

/// 安装 ring 后端（幂等；多后端同时启用时 rustls 必须显式选定）。
fn ensure_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// 服务端 TLS 配置：证书链 + 私钥；配置 `client_ca` 时要求客户端证书（mTLS）。
pub fn server_acceptor(cfg: &TlsConfig) -> Result<TlsAcceptor, TlsError> {
    ensure_provider();
    let cert_path = cfg
        .certificate
        .as_ref()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| TlsError::Invalid("缺少 certificate".to_owned()))?;
    let key_path = cfg
        .private_key
        .as_ref()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| TlsError::Invalid("缺少 private_key".to_owned()))?;
    let certs = load_certs(cert_path)?;
    let key = load_private_key(key_path)?;

    let builder = rustls::ServerConfig::builder();
    let builder =
        if let Some(ca_path) = cfg.client_ca.as_ref().filter(|p| !p.as_os_str().is_empty()) {
            let mut roots = RootCertStore::empty();
            for cert in load_certs(ca_path)? {
                roots
                    .add(cert)
                    .map_err(|e| TlsError::Rustls(format!("客户端 CA 证书非法：{e}")))?;
            }
            let verifier = WebPkiClientVerifier::builder(Arc::new(roots))
                .build()
                .map_err(|e| TlsError::Rustls(format!("构建客户端证书校验器失败：{e}")))?;
            builder.with_client_cert_verifier(verifier)
        } else {
            builder.with_no_client_auth()
        };
    let config = builder
        .with_single_cert(certs, key)
        .map_err(|e| TlsError::Rustls(format!("证书与私钥不匹配：{e}")))?;
    Ok(TlsAcceptor::from(Arc::new(config)))
}

/// 客户端 TLS 配置：定制 CA（仅信任该 CA）或 webpki 根证书。
pub fn client_config(ca: Option<&Path>) -> Result<rustls::ClientConfig, TlsError> {
    ensure_provider();
    let mut roots = RootCertStore::empty();
    match ca {
        Some(path) => {
            for cert in load_certs(path)? {
                roots
                    .add(cert)
                    .map_err(|e| TlsError::Rustls(format!("CA 证书非法：{e}")))?;
            }
            if roots.is_empty() {
                return Err(TlsError::Invalid("CA 文件不含证书".to_owned()));
            }
        }
        None => {
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        }
    }
    Ok(rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth())
}

fn load_certs(path: &Path) -> Result<Vec<CertificateDer<'static>>, TlsError> {
    let file = File::open(path).map_err(|e| TlsError::Io(path.to_owned(), e.to_string()))?;
    let mut reader = BufReader::new(file);
    let certs = rustls_pemfile::certs(&mut reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| TlsError::Pem(e.to_string()))?;
    if certs.is_empty() {
        return Err(TlsError::Pem(format!("{} 不含证书", path.display())));
    }
    Ok(certs)
}

fn load_private_key(path: &Path) -> Result<PrivateKeyDer<'static>, TlsError> {
    let file = File::open(path).map_err(|e| TlsError::Io(path.to_owned(), e.to_string()))?;
    let mut reader = BufReader::new(file);
    rustls_pemfile::private_key(&mut reader)
        .map_err(|e| TlsError::Pem(e.to_string()))?
        .ok_or_else(|| TlsError::Pem(format!("{} 不含私钥", path.display())))
}
