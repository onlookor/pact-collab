//! D48：RefBindings 与 ContractCard 的显式重载（控制面）。
//!
//! 只做“读取配置源 → 校验 → 影响预检 → 审计 → 原子替换”，不做文件监听、
//! SIGHUP、自动回滚或按域灰度。卡片与绑定作为同一事务生效。

use crate::dto::BlockedTokenGroup;
use crate::visibility::{RefEntryError, RefSourceFile, VisibilityProfiles};
use pact_engine::{iso8601_utc, Engine, EngineError, InstanceKey};
use pact_ref_spec::SpecError;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// 重载所需配置（由 daemon 在启动时注入，之后不再变化）。
#[derive(Debug, Clone)]
pub struct RefReloadConfig {
    pub binding_paths: Vec<PathBuf>,
    pub contract_paths: Vec<PathBuf>,
    /// `<data_dir>/ref-reload.jsonl`。
    pub audit_path: PathBuf,
}

impl RefReloadConfig {
    pub fn new(
        binding_paths: Vec<PathBuf>,
        contract_paths: Vec<PathBuf>,
        data_dir: impl AsRef<Path>,
    ) -> Self {
        Self {
            binding_paths,
            contract_paths,
            audit_path: data_dir.as_ref().join("ref-reload.jsonl"),
        }
    }
}

/// 控制面审计记录（每次重载尝试一行 JSON，append+fsync）。
#[derive(Debug, Serialize)]
pub struct RefReloadAudit<'a> {
    pub ts: String,
    pub by: Option<&'a str>,
    #[serde(rename = "dryRun")]
    pub dry_run: bool,
    /// `applied`（校验通过，dryRun 时表示“将会生效”）或 `rejected`。
    pub outcome: &'a str,
    pub files: &'a [RefSourceFile],
    pub bindings: usize,
    pub contracts: usize,
    #[serde(rename = "blockedTokens")]
    pub blocked_tokens: &'a [BlockedTokenGroup],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<&'a str>,
}

/// 追加一行审计并 fsync；失败由调用方转成 `500` 并拒绝替换索引。
pub fn append_audit(path: &Path, record: &RefReloadAudit<'_>) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let mut line = serde_json::to_string(record)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    line.push('\n');
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(line.as_bytes())?;
    file.sync_all()
}

/// 当前 UTC 秒精度时间戳（与事件日志时间格式一致）。
pub fn now_iso8601() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    iso8601_utc(secs)
}

/// 影响预检：统计“换绑后**新增**因缺摘要而无法归档”的存量 Counter token。
///
/// 只比较归档钉版判定（D46/D47 的 `PinningRequired { at_archive: true }`），
/// 旧规则已阻断的 token 不计入本次变更影响；只返回计数，不返回票面值。
pub async fn newly_blocked_tokens(
    engine: &Engine,
    old: &VisibilityProfiles,
    new: &VisibilityProfiles,
) -> Result<Vec<BlockedTokenGroup>, EngineError> {
    let mut blocked: BTreeMap<(String, String, String), usize> = BTreeMap::new();
    for listing in engine.list_domains().await? {
        for instance in &listing.instances {
            let key = InstanceKey::new(listing.name.clone(), instance.clone());
            let view = match engine.counter(key.clone(), None).await {
                Ok(view) => view,
                Err(EngineError::InstanceNotFound { .. }) => continue,
                Err(error) => return Err(error),
            };
            for token in view.tokens {
                let Some(raw) = token.thing_ref.0.as_deref() else {
                    continue;
                };
                let ty = token.ty.0.as_str();
                if old
                    .validate_archive_thing_ref(&key.domain, &key.instance, ty, Some(raw))
                    .is_ok()
                    && requires_archive_pin(new, &key, ty, raw)
                {
                    *blocked
                        .entry((key.domain.clone(), key.instance.clone(), ty.to_owned()))
                        .or_insert(0) += 1;
                }
            }
        }
    }
    Ok(blocked
        .into_iter()
        .map(|((domain, instance, ty), tokens)| BlockedTokenGroup {
            domain,
            instance,
            ty,
            tokens,
        })
        .collect())
}

fn requires_archive_pin(
    profiles: &VisibilityProfiles,
    key: &InstanceKey,
    ty: &str,
    raw: &str,
) -> bool {
    matches!(
        profiles.validate_archive_thing_ref(&key.domain, &key.instance, ty, Some(raw)),
        Err(RefEntryError::Invalid(SpecError::PinningRequired {
            at_archive: true,
            ..
        }))
    )
}

#[cfg(test)]
mod tests {
    use super::{append_audit, now_iso8601, RefReloadAudit};

    #[test]
    fn audit_lines_are_jsonl_and_fsync_writable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ref-reload.jsonl");
        let record = RefReloadAudit {
            ts: now_iso8601(),
            by: Some("architect"),
            dry_run: true,
            outcome: "applied",
            files: &[],
            bindings: 0,
            contracts: 0,
            blocked_tokens: &[],
            error: None,
        };
        append_audit(&path, &record).unwrap();
        append_audit(&path, &record).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["by"], "architect");
        assert_eq!(first["dryRun"], true);
        assert_eq!(first["outcome"], "applied");
        assert!(first.get("error").is_none());
        assert!(first["files"].as_array().unwrap().is_empty());
    }

    #[test]
    fn audit_write_fails_when_target_is_a_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ref-reload.jsonl");
        std::fs::create_dir(&path).unwrap();
        let record = RefReloadAudit {
            ts: now_iso8601(),
            by: None,
            dry_run: false,
            outcome: "rejected",
            files: &[],
            bindings: 0,
            contracts: 0,
            blocked_tokens: &[],
            error: Some("boom"),
        };
        assert!(append_audit(&path, &record).is_err());
    }
}
