//! D42：Counter/Archive 的 `thing_ref` 可见性策略。
//!
//! 默认 `read-gate`：只有调用者对该类型有**无条件** `reads` 或 `consumes` 子句时
//! 才返回票面；`domain` 显式公开；`hidden` 永不返回。显式 profile 来自 daemon 配置
//! 指向的 RefBindings（`spec.types.*.ref.visibility`，含 scheme/instance 覆盖）。
//!
//! 本模块只读配置期文件，请求期只做纯匹配；不访问网络、不写引擎状态。

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use pact_ref_spec::{
    classify_identifier, pinned_digest, PinningRequirement, RefBindings, ReferenceForm, SpecError,
    UnknownSchemePolicy, Visibility, DEFAULT_MAX_REFERENCE_LENGTH, HARD_MAX_REFERENCE_LENGTH,
};

/// 单个 token 的票面暴露级别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefExposure {
    /// 票面可直接返回（显式 `domain`，或 token 本就没有票面）。
    Visible,
    /// 需要调用者具备无条件的 reads/consumes 子句。
    ReadGate,
    /// 永不返回票面。
    Hidden,
}

/// 已加载的 RefBindings 集合（按域索引）。
#[derive(Debug, Clone, Default)]
pub struct VisibilityProfiles {
    bindings: BTreeMap<String, LoadedBindings>,
}

#[derive(Debug, Clone)]
struct LoadedBindings {
    path: PathBuf,
    bindings: RefBindings,
}

impl VisibilityProfiles {
    /// 无显式 profile：所有票面走默认 `read-gate`。
    pub fn empty() -> Self {
        Self::default()
    }

    /// 读取文件/目录并校验；任一文件失败即拒绝启动（fail-closed）。
    pub fn load(paths: &[PathBuf]) -> Result<Self, VisibilityError> {
        let files = expand(paths)?;
        let mut bindings: BTreeMap<String, LoadedBindings> = BTreeMap::new();
        for path in files {
            let text = std::fs::read_to_string(&path).map_err(|source| VisibilityError::Io {
                path: path.clone(),
                source,
            })?;
            let parsed: RefBindings =
                serde_yaml::from_str(&text).map_err(|error| VisibilityError::Parse {
                    path: path.clone(),
                    message: error.to_string(),
                })?;
            parsed
                .validate()
                .map_err(|error| VisibilityError::Invalid {
                    path: path.clone(),
                    message: error.to_string(),
                })?;
            let domain = parsed.metadata.domain.clone();
            if let Some(previous) = bindings.get(&domain) {
                return Err(VisibilityError::DuplicateDomain {
                    domain,
                    first: previous.path.clone(),
                    second: path,
                });
            }
            bindings.insert(
                domain,
                LoadedBindings {
                    path,
                    bindings: parsed,
                },
            );
        }
        Ok(Self { bindings })
    }

    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    pub fn has_domain(&self, domain: &str) -> bool {
        self.bindings.contains_key(domain)
    }

    /// D45：写入路径的票面校验。
    ///
    /// `input` 的三态与 [`ThingRefField`](crate::dto::ThingRefField) 对齐：
    /// `None` = 字段缺失，`Some(None)` = 显式 `∅`，`Some(Some(raw))` = 有引用。
    ///
    /// 无 RefBindings 的域只做结构校验（默认 2048 字节、URI profile、Identifier 文法）；
    /// 有 RefBindings 的域再强制 `required` / `allowIdentifier` / `schemes` /
    /// `onUnknownScheme` / `value` / `pinning: required` / `maxLength`（D45/D46）。
    pub fn validate_thing_ref(
        &self,
        domain: &str,
        instance: &str,
        ty: &str,
        input: Option<Option<&str>>,
    ) -> Result<Option<String>, RefEntryError> {
        let Some(value) = input else {
            return Err(RefEntryError::Missing);
        };
        let Some(raw) = value else {
            if let Some(loaded) = self.bindings.get(domain) {
                loaded
                    .bindings
                    .resolve(Some(instance), ty, &ReferenceForm::Absent)
                    .map_err(RefEntryError::Invalid)?;
            }
            return Ok(None);
        };
        let reference = ReferenceForm::classify_with_limit(Some(raw), HARD_MAX_REFERENCE_LENGTH)
            .map_err(RefEntryError::Invalid)?;
        let Some(loaded) = self.bindings.get(domain) else {
            if raw.len() > DEFAULT_MAX_REFERENCE_LENGTH {
                return Err(RefEntryError::Invalid(SpecError::TooLong {
                    actual: raw.len(),
                    max: DEFAULT_MAX_REFERENCE_LENGTH,
                }));
            }
            return Ok(reference.as_str().map(str::to_owned));
        };
        let matched = loaded
            .bindings
            .resolve(Some(instance), ty, &reference)
            .map_err(RefEntryError::Invalid)?;

        // D46：未知 scheme 的三种处理。`resolve` 只拦 `reject`（缺省），
        // 其余两种在这里落地。
        let mut effective = reference;
        if let ReferenceForm::Uri(uri) = &effective {
            let scheme = effective.scheme().unwrap_or_default().to_owned();
            if scheme_is_unknown(&matched.rule, &scheme) {
                match matched
                    .rule
                    .on_unknown_scheme
                    .unwrap_or(UnknownSchemePolicy::Reject)
                {
                    UnknownSchemePolicy::Allow => {}
                    UnknownSchemePolicy::TreatAsIdentifier => {
                        if !matched.rule.allow_identifier.unwrap_or(false) {
                            return Err(RefEntryError::Invalid(SpecError::IdentifierNotAllowed {
                                type_name: ty.to_owned(),
                            }));
                        }
                        effective = classify_identifier(uri).map_err(RefEntryError::Invalid)?;
                    }
                    UnknownSchemePolicy::Reject => {
                        return Err(RefEntryError::Invalid(SpecError::SchemeNotAllowed {
                            scheme,
                            type_name: ty.to_owned(),
                        }));
                    }
                }
            }
        }

        // D46：`value` 约束非空票面的形态；`∅` 由 `required` 决定。
        if let Some(expected) = matched.rule.value.as_deref() {
            let actual = match &effective {
                ReferenceForm::Uri(_) => "uri",
                ReferenceForm::Identifier(_) => "identifier",
                ReferenceForm::Absent => "null",
            };
            let ok = match expected {
                "none" => effective.is_absent(),
                "uri" => effective.is_uri(),
                "identifier" => effective.is_identifier(),
                _ => true,
            };
            if !ok {
                return Err(RefEntryError::Invalid(SpecError::ValueFormMismatch {
                    expected: expected.to_owned(),
                    actual: actual.to_owned(),
                }));
            }
        }

        // D46：写入时钉版（`required-at-archive` 留到 consume）。
        if matched.pins_at_entry() {
            if let Some(value) = effective.as_str() {
                if pinned_digest(value).is_none() {
                    return Err(RefEntryError::Invalid(SpecError::PinningRequired {
                        type_name: ty.to_owned(),
                        at_archive: false,
                    }));
                }
            }
        }

        let max_length = matched
            .rule
            .max_length
            .unwrap_or(DEFAULT_MAX_REFERENCE_LENGTH);
        if raw.len() > max_length {
            return Err(RefEntryError::Invalid(SpecError::TooLong {
                actual: raw.len(),
                max: max_length,
            }));
        }
        Ok(effective.as_str().map(str::to_owned))
    }

    /// D46：进入 Archive（consume）前的钉版检查。
    ///
    /// 只读取继承出的 `pinning`，不重复执行写入期策略：历史票面可能已经不符合
    /// 收紧后的入口规则，但归档要求仍按当前绑定生效。`∅` 不受钉版约束。
    pub fn validate_archive_thing_ref(
        &self,
        domain: &str,
        instance: &str,
        ty: &str,
        thing_ref: Option<&str>,
    ) -> Result<(), RefEntryError> {
        let Some(raw) = thing_ref else {
            return Ok(());
        };
        if pinned_digest(raw).is_some() {
            return Ok(());
        }
        let Some(loaded) = self.bindings.get(domain) else {
            return Ok(());
        };
        // 分类失败的历史值按 Identifier 处理（无 scheme 覆盖），只影响继承链选择。
        let reference = ReferenceForm::classify_with_limit(Some(raw), HARD_MAX_REFERENCE_LENGTH)
            .unwrap_or_else(|_| ReferenceForm::Identifier(raw.to_owned()));
        let rule = loaded.bindings.merged_rule(Some(instance), ty, &reference);
        if matches!(
            rule.pinning,
            Some(PinningRequirement::Required | PinningRequirement::RequiredAtArchive)
        ) {
            return Err(RefEntryError::Invalid(SpecError::PinningRequired {
                type_name: ty.to_owned(),
                at_archive: true,
            }));
        }
        Ok(())
    }

    /// 计算单个 token 的票面暴露级别。
    ///
    /// `thing_ref` 为空时不涉及票面，返回 `Visible`；匹配失败或绑定拒绝时回退
    /// 到类型/实例层（不看 scheme 覆盖），再回退到默认 `read-gate`。
    pub fn exposure(
        &self,
        domain: &str,
        instance: &str,
        ty: &str,
        thing_ref: Option<&str>,
    ) -> RefExposure {
        let Some(raw) = thing_ref else {
            return RefExposure::Visible;
        };
        let Some(loaded) = self.bindings.get(domain) else {
            return RefExposure::ReadGate;
        };
        let bindings = &loaded.bindings;
        let visibility = match ReferenceForm::classify(Some(raw)) {
            Ok(reference) => match bindings.resolve(Some(instance), ty, &reference) {
                Ok(matched) => matched
                    .rule
                    .visibility
                    .or_else(|| inherited_visibility(bindings, instance, ty)),
                Err(SpecError::SchemeNotAllowed { .. }) => {
                    inherited_visibility(bindings, instance, ty)
                }
                Err(_) => inherited_visibility(bindings, instance, ty),
            },
            Err(_) => inherited_visibility(bindings, instance, ty),
        };
        match visibility.unwrap_or(Visibility::ReadGate) {
            Visibility::Domain => RefExposure::Visible,
            Visibility::Hidden => RefExposure::Hidden,
            Visibility::ReadGate => RefExposure::ReadGate,
        }
    }
}

/// 有效 `schemes` 非空且不含 `*` 时，不在白名单内的 scheme 视为“未知”。
fn scheme_is_unknown(rule: &pact_ref_spec::RefRule, scheme: &str) -> bool {
    rule.schemes.as_ref().is_some_and(|allowed| {
        !allowed.is_empty()
            && !allowed
                .iter()
                .any(|candidate| candidate == "*" || candidate == scheme)
    })
}

/// defaults → type → instance/type 的 visibility 继承（忽略 scheme 覆盖）。
fn inherited_visibility(bindings: &RefBindings, instance: &str, ty: &str) -> Option<Visibility> {
    let mut visibility = bindings
        .spec
        .defaults
        .as_ref()
        .and_then(|entry| entry.rule.visibility);
    if let Some(value) = bindings
        .spec
        .types
        .get(ty)
        .and_then(|entry| entry.rule.visibility)
    {
        visibility = Some(value);
    }
    if let Some(value) = bindings
        .spec
        .instances
        .get(instance)
        .and_then(|instance| instance.types.get(ty))
        .and_then(|entry| entry.rule.visibility)
    {
        visibility = Some(value);
    }
    visibility
}

/// 写入路径的票面校验错误。
#[derive(Debug)]
pub enum RefEntryError {
    /// 请求体缺少 `thing_ref` 字段（作者漏传）。
    Missing,
    /// 结构或 RefBindings 策略拒绝。
    Invalid(SpecError),
}

impl fmt::Display for RefEntryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => f.write_str("请求体缺少 thing_ref 字段；确认无制品请显式传 null"),
            Self::Invalid(error) => error.fmt(f),
        }
    }
}

/// 加载/校验错误。
#[derive(Debug)]
pub enum VisibilityError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Parse {
        path: PathBuf,
        message: String,
    },
    Invalid {
        path: PathBuf,
        message: String,
    },
    DuplicateDomain {
        domain: String,
        first: PathBuf,
        second: PathBuf,
    },
}

impl fmt::Display for VisibilityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(f, "读取 RefBindings {} 失败：{source}", path.display())
            }
            Self::Parse { path, message } => {
                write!(f, "解析 RefBindings {} 失败：{message}", path.display())
            }
            Self::Invalid { path, message } => {
                write!(f, "RefBindings {} 校验失败：{message}", path.display())
            }
            Self::DuplicateDomain {
                domain,
                first,
                second,
            } => write!(
                f,
                "域 {domain} 在 {} 与 {} 中重复声明 RefBindings",
                first.display(),
                second.display()
            ),
        }
    }
}

impl std::error::Error for VisibilityError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

fn expand(paths: &[PathBuf]) -> Result<Vec<PathBuf>, VisibilityError> {
    let mut files = std::collections::BTreeSet::new();
    for path in paths {
        let metadata = std::fs::metadata(path).map_err(|source| VisibilityError::Io {
            path: path.clone(),
            source,
        })?;
        if metadata.is_dir() {
            let entries = std::fs::read_dir(path).map_err(|source| VisibilityError::Io {
                path: path.clone(),
                source,
            })?;
            for entry in entries {
                let entry = entry.map_err(|source| VisibilityError::Io {
                    path: path.clone(),
                    source,
                })?;
                let candidate = entry.path();
                if candidate.is_file() && is_yaml_or_json(&candidate) {
                    files.insert(candidate);
                }
            }
        } else if metadata.is_file() {
            files.insert(path.clone());
        } else {
            return Err(VisibilityError::Io {
                path: path.clone(),
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "既不是普通文件也不是目录",
                ),
            });
        }
    }
    Ok(files.into_iter().collect())
}

fn is_yaml_or_json(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("yaml" | "yml" | "json")
    )
}

#[cfg(test)]
mod tests {
    use super::{RefExposure, VisibilityProfiles};

    const BINDINGS: &str = r#"
apiVersion: pact.ref/v1
kind: RefBindings
metadata:
  domain: proj
spec:
  defaults:
    ref:
      visibility: read-gate
  types:
    requirement:
      ref:
        visibility: domain
    design:
      ref:
        visibility: hidden
    implementation:
      ref:
        visibility: read-gate
    notes:
      ref: {}
  schemes:
    mcp:
      ref:
        visibility: domain
  instances:
    special:
      types:
        implementation:
          ref:
            visibility: domain
"#;

    fn load(text: &str) -> (tempfile::TempDir, VisibilityProfiles) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("refs.yaml"), text).unwrap();
        let profiles = VisibilityProfiles::load(&[dir.path().join("refs.yaml")]).unwrap();
        (dir, profiles)
    }

    #[test]
    fn empty_profiles_default_to_read_gate() {
        let profiles = VisibilityProfiles::empty();
        assert!(profiles.is_empty());
        assert_eq!(
            profiles.exposure("proj", "default", "requirement", Some("https://x/1")),
            RefExposure::ReadGate
        );
    }

    #[test]
    fn explicit_profiles_are_applied() {
        let (_dir, profiles) = load(BINDINGS);
        assert!(profiles.has_domain("proj"));
        assert_eq!(
            profiles.exposure("proj", "default", "requirement", Some("https://x/1")),
            RefExposure::Visible
        );
        assert_eq!(
            profiles.exposure("proj", "default", "design", Some("https://x/2")),
            RefExposure::Hidden
        );
        assert_eq!(
            profiles.exposure("proj", "default", "implementation", Some("https://x/3")),
            RefExposure::ReadGate
        );
    }

    #[test]
    fn token_without_reference_is_visible() {
        let (_dir, profiles) = load(BINDINGS);
        assert_eq!(
            profiles.exposure("proj", "default", "design", None),
            RefExposure::Visible
        );
    }

    #[test]
    fn scheme_and_instance_overrides_win() {
        let (_dir, profiles) = load(BINDINGS);
        assert_eq!(
            profiles.exposure("proj", "default", "notes", Some("mcp://x/1")),
            RefExposure::Visible
        );
        assert_eq!(
            profiles.exposure("proj", "default", "notes", Some("https://x/1")),
            RefExposure::ReadGate
        );
        assert_eq!(
            profiles.exposure("proj", "special", "implementation", Some("https://x/3")),
            RefExposure::Visible
        );
    }

    #[test]
    fn unknown_domain_falls_back_to_read_gate() {
        let (_dir, profiles) = load(BINDINGS);
        assert_eq!(
            profiles.exposure("other", "default", "requirement", Some("https://x/1")),
            RefExposure::ReadGate
        );
    }

    #[test]
    fn duplicate_domain_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("a.yaml");
        let second = dir.path().join("b.yaml");
        std::fs::write(&first, BINDINGS).unwrap();
        std::fs::write(&second, BINDINGS).unwrap();
        let error = VisibilityProfiles::load(&[first, second]).unwrap_err();
        assert!(error.to_string().contains("重复声明"), "{error}");
    }

    #[test]
    fn invalid_bindings_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.yaml");
        std::fs::write(
            &path,
            r#"
apiVersion: pact.ref/v1
kind: RefBindings
metadata:
  domain: proj
spec:
  types:
    design:
      ref:
        contract: "not a contract id"
"#,
        )
        .unwrap();
        let error = VisibilityProfiles::load(&[path]).unwrap_err();
        assert!(error.to_string().contains("校验失败"), "{error}");
    }

    const STRICT_BINDINGS: &str = r#"
apiVersion: pact.ref/v1
kind: RefBindings
metadata:
  domain: proj
spec:
  types:
    requirement:
      ref:
        required: true
        allowIdentifier: false
        schemes: ["https"]
        maxLength: 32
"#;

    #[test]
    fn entry_validation_without_bindings_is_structural() {
        use super::RefEntryError;
        use pact_ref_spec::{SpecError, DEFAULT_MAX_REFERENCE_LENGTH};

        let profiles = VisibilityProfiles::empty();
        assert!(matches!(
            profiles.validate_thing_ref("proj", "default", "requirement", None),
            Err(RefEntryError::Missing)
        ));
        assert_eq!(
            profiles
                .validate_thing_ref("proj", "default", "requirement", Some(None))
                .unwrap(),
            None
        );
        for raw in ["https://user:pass@example.com/a", "/tmp/a", "foo bar"] {
            assert!(
                matches!(
                    profiles.validate_thing_ref("proj", "default", "requirement", Some(Some(raw))),
                    Err(RefEntryError::Invalid(_))
                ),
                "{raw} should be rejected"
            );
        }
        let long = "x".repeat(DEFAULT_MAX_REFERENCE_LENGTH + 1);
        assert!(matches!(
            profiles.validate_thing_ref("proj", "default", "requirement", Some(Some(&long))),
            Err(RefEntryError::Invalid(SpecError::TooLong { .. }))
        ));
        let normalized = profiles
            .validate_thing_ref(
                "proj",
                "default",
                "requirement",
                Some(Some("HTTPS://Example.com/a")),
            )
            .unwrap();
        assert_eq!(normalized.as_deref(), Some("https://Example.com/a"));
    }

    #[test]
    fn entry_validation_applies_bindings_policy() {
        use super::RefEntryError;
        use pact_ref_spec::SpecError;

        let (_dir, profiles) = load(STRICT_BINDINGS);
        assert!(matches!(
            profiles.validate_thing_ref("proj", "default", "requirement", Some(None)),
            Err(RefEntryError::Invalid(SpecError::ReferenceRequired))
        ));
        assert!(matches!(
            profiles.validate_thing_ref("proj", "default", "requirement", Some(Some("REQ-1"))),
            Err(RefEntryError::Invalid(
                SpecError::IdentifierNotAllowed { .. }
            ))
        ));
        assert!(matches!(
            profiles.validate_thing_ref(
                "proj",
                "default",
                "requirement",
                Some(Some("git://example.com/a"))
            ),
            Err(RefEntryError::Invalid(SpecError::SchemeNotAllowed { .. }))
        ));
        assert!(matches!(
            profiles.validate_thing_ref(
                "proj",
                "default",
                "requirement",
                Some(Some("https://example.com/aaaaaaaaaaaaaaaaaaaa"))
            ),
            Err(RefEntryError::Invalid(SpecError::TooLong { .. }))
        ));
    }

    const POLICY_BINDINGS: &str = r#"
apiVersion: pact.ref/v1
kind: RefBindings
metadata:
  domain: proj
spec:
  types:
    requirement:
      ref:
        schemes: ["https"]
        onUnknownScheme: allow
    design:
      ref:
        schemes: ["https"]
        onUnknownScheme: treat-as-identifier
        allowIdentifier: true
    service:
      ref:
        schemes: ["https"]
        onUnknownScheme: treat-as-identifier
    implementation:
      ref:
        allowIdentifier: true
        value: uri
    test_report:
      ref:
        value: none
    deployment:
      ref:
        pinning: required
    review:
      ref:
        allowIdentifier: true
        pinning: required-at-archive
"#;

    fn pinned_uri() -> String {
        format!("https://artifacts.example.com/d#sha256={}", "9f".repeat(32))
    }

    #[test]
    fn unknown_scheme_policy_applies_at_entry() {
        use super::RefEntryError;
        use pact_ref_spec::SpecError;

        let (_dir, profiles) = load(POLICY_BINDINGS);
        assert_eq!(
            profiles
                .validate_thing_ref(
                    "proj",
                    "default",
                    "requirement",
                    Some(Some("mcp://build.example.com/run"))
                )
                .unwrap()
                .as_deref(),
            Some("mcp://build.example.com/run"),
            "allow 放行未知 scheme"
        );
        assert_eq!(
            profiles
                .validate_thing_ref(
                    "proj",
                    "default",
                    "design",
                    Some(Some("mcp://build.example.com/run"))
                )
                .unwrap()
                .as_deref(),
            Some("mcp://build.example.com/run"),
            "treat-as-identifier 且 allowIdentifier=true → 接受"
        );
        let rejected = profiles.validate_thing_ref(
            "proj",
            "default",
            "service",
            Some(Some("mcp://build.example.com/run")),
        );
        assert!(
            matches!(
                rejected,
                Err(RefEntryError::Invalid(
                    SpecError::IdentifierNotAllowed { .. }
                ))
            ),
            "treat-as-identifier 未放行 Identifier → 拒绝"
        );
    }

    #[test]
    fn value_policy_applies_at_entry() {
        use super::RefEntryError;
        use pact_ref_spec::SpecError;

        let (_dir, profiles) = load(POLICY_BINDINGS);
        assert_eq!(
            profiles
                .validate_thing_ref(
                    "proj",
                    "default",
                    "implementation",
                    Some(Some("https://example.com/a"))
                )
                .unwrap()
                .as_deref(),
            Some("https://example.com/a")
        );
        assert!(matches!(
            profiles.validate_thing_ref(
                "proj",
                "default",
                "implementation",
                Some(Some("REQ-001"))
            ),
            Err(RefEntryError::Invalid(SpecError::ValueFormMismatch {
                expected,
                ..
            })) if expected == "uri"
        ));
        assert!(matches!(
            profiles.validate_thing_ref(
                "proj",
                "default",
                "test_report",
                Some(Some("https://example.com/a"))
            ),
            Err(RefEntryError::Invalid(SpecError::ValueFormMismatch {
                expected,
                ..
            })) if expected == "none"
        ));
        assert_eq!(
            profiles
                .validate_thing_ref("proj", "default", "test_report", Some(None))
                .unwrap(),
            None
        );
    }

    #[test]
    fn pinning_required_rejects_unpinned_entry() {
        use super::RefEntryError;
        use pact_ref_spec::SpecError;

        let (_dir, profiles) = load(POLICY_BINDINGS);
        assert!(matches!(
            profiles.validate_thing_ref(
                "proj",
                "default",
                "deployment",
                Some(Some("https://example.com/d"))
            ),
            Err(RefEntryError::Invalid(SpecError::PinningRequired {
                at_archive: false,
                ..
            }))
        ));
        assert_eq!(
            profiles
                .validate_thing_ref("proj", "default", "deployment", Some(Some(&pinned_uri())))
                .unwrap()
                .as_deref(),
            Some(pinned_uri().as_str())
        );
        assert_eq!(
            profiles
                .validate_thing_ref("proj", "default", "deployment", Some(None))
                .unwrap(),
            None,
            "∅ 不受钉版约束"
        );
    }

    #[test]
    fn archive_pinning_requires_digest() {
        use super::RefEntryError;
        use pact_ref_spec::SpecError;

        let (_dir, profiles) = load(POLICY_BINDINGS);
        // required-at-archive 不拦写入。
        assert_eq!(
            profiles
                .validate_thing_ref(
                    "proj",
                    "default",
                    "review",
                    Some(Some("https://example.com/r"))
                )
                .unwrap()
                .as_deref(),
            Some("https://example.com/r")
        );
        assert!(matches!(
            profiles.validate_archive_thing_ref(
                "proj",
                "default",
                "review",
                Some("https://example.com/r")
            ),
            Err(RefEntryError::Invalid(SpecError::PinningRequired {
                at_archive: true,
                ..
            }))
        ));
        profiles
            .validate_archive_thing_ref("proj", "default", "review", Some(&pinned_uri()))
            .expect("pinned");
        profiles
            .validate_archive_thing_ref("proj", "default", "review", None)
            .expect("∅ 可归档");
        profiles
            .validate_archive_thing_ref(
                "proj",
                "default",
                "requirement",
                Some("https://example.com/a"),
            )
            .expect("未要求钉版的类型不拦");
    }
}
