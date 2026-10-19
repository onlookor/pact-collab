//! 运行层错误。

use pact_http::client::ClientError;

/// 生产运行层错误。
#[derive(Debug)]
pub enum RuntimeError {
    /// daemon 以 §6.3 结构化错误体拒绝（工具调用 / REST 4xx-5xx）。
    Api { code: String, message: String },
    /// 传输层失败（socket、HTTP、MCP 子进程）。
    Transport(String),
    /// 协议层失败（非法响应、握手失败、工具输出非法 JSON）。
    Protocol(String),
    /// 期限耗尽：参与者未在 deadline 内自驱至终止，或域未在期限内出现。
    Timeout(String),
    /// 本地 IO 失败（配置读取、子进程启动）。
    Io(std::io::Error),
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RuntimeError::Api { code, message } => write!(f, "daemon 拒绝（{code}）：{message}"),
            RuntimeError::Transport(m) => write!(f, "传输失败：{m}"),
            RuntimeError::Protocol(m) => write!(f, "协议失败：{m}"),
            RuntimeError::Timeout(m) => write!(f, "期限耗尽：{m}"),
            RuntimeError::Io(e) => write!(f, "本地 IO 失败：{e}"),
        }
    }
}

impl std::error::Error for RuntimeError {}

impl From<std::io::Error> for RuntimeError {
    fn from(e: std::io::Error) -> Self {
        RuntimeError::Io(e)
    }
}

impl From<ClientError> for RuntimeError {
    fn from(e: ClientError) -> Self {
        match e {
            ClientError::Api(d) => RuntimeError::Api {
                code: d.code,
                message: d.message,
            },
            ClientError::Io(e) => RuntimeError::Io(e),
            ClientError::Http(e) => RuntimeError::Transport(e.to_string()),
            ClientError::Parse(e) => RuntimeError::Protocol(e.to_string()),
            ClientError::UnexpectedStatus(s) => {
                RuntimeError::Protocol(format!("非预期状态码：{s}"))
            }
        }
    }
}
