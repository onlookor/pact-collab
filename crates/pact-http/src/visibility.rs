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

use pact_ref_spec::{RefBindings, ReferenceForm, SpecError, Visibility};

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
}
