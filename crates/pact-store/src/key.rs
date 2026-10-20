//! 域双键（D27 路径 β）：定义键 `domain` + 实例键 `instance`。
//!
//! 论文侧（rev3 §1.3）：`domain` 是 **domain declaration**（定义标识），
//! `instance` 是 **Counter instance**（实现自由的运行实例划分）。
//! 工程侧**禁用** `domain instance` 指运行实例。

use crate::error::{validate_domain_name, validate_instance_name, StoreError};

/// 实例键：同一域声明可并发承载多个 Counter 实例。
///
/// 单一类型贯穿存储/引擎/接口，避免全链两参数错序；`Ord` 供 `BTreeMap` 排序。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InstanceKey {
    pub domain: String,
    pub instance: String,
}

impl InstanceKey {
    pub fn new(domain: impl Into<String>, instance: impl Into<String>) -> Self {
        Self {
            domain: domain.into(),
            instance: instance.into(),
        }
    }

    /// 两段各自校验（域名/实例名同谓词：非空、`[A-Za-z0-9_-]`、≤128）。
    pub fn validate(&self) -> Result<(), StoreError> {
        validate_domain_name(&self.domain)?;
        validate_instance_name(&self.instance)
    }
}

impl std::fmt::Display for InstanceKey {
    /// `"<domain>/<instance>"`（与磁盘布局一致；不用 `:`，`:suffix` 已被接口层占用）。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.domain, self.instance)
    }
}
