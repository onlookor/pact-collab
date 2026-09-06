//! TokenId 生成抽象（D10）：IdGen trait + 生产（ULID）/测试（顺序）实现。
//! 引擎构造时注入；pact-core 不生成 id（纯度约束 §2.1-4）。

use pact_core::TokenId;

pub trait IdGen: Send + Sync {
    fn next(&self) -> TokenId;
}

/// 生产实现：ULID（有序、唯一、可读，D10）。
pub struct UlidGen;

impl IdGen for UlidGen {
    fn next(&self) -> TokenId {
        TokenId(ulid::Ulid::new().to_string())
    }
}

/// 测试实现：顺序 id（"seq-1"、"seq-2"、…），确定性重放。
pub struct SequentialIdGen(std::sync::atomic::AtomicU64);

impl SequentialIdGen {
    pub fn new() -> Self {
        Self(std::sync::atomic::AtomicU64::new(0))
    }
    pub fn new_at(start: u64) -> Self {
        Self(std::sync::atomic::AtomicU64::new(start))
    }
}

impl Default for SequentialIdGen {
    fn default() -> Self {
        Self::new()
    }
}

impl IdGen for SequentialIdGen {
    fn next(&self) -> TokenId {
        let n = self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        TokenId(format!("seq-{n}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequential_ids_deterministic() {
        let g = SequentialIdGen::new();
        assert_eq!(g.next().0, "seq-0");
        assert_eq!(g.next().0, "seq-1");
    }

    #[test]
    fn ulid_ids_unique_canonical() {
        let g = UlidGen;
        let a = g.next().0;
        let b = g.next().0;
        assert_ne!(a, b);
        assert_eq!(a.len(), 26, "ULID 规范长度 26（Crockford Base32）");
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric()));
    }
}
