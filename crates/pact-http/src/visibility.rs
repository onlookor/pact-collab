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
    pinned_digest, validate_entry, ContractCard, DigestPolicy, EntryRef, PinningRequirement,
    RefBindings, RefRule, ReferenceForm, SpecError, Visibility, HARD_MAX_REFERENCE_LENGTH,
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

/// 已加载的 RefBindings 与 ContractCard 集合（按域 / 契约 ID 索引）。
#[derive(Debug, Clone, Default)]
pub struct VisibilityProfiles {
    bindings: BTreeMap<String, LoadedBindings>,
    /// D47：`contract_id()`（`metadata.id@version`）→ 卡片；同版本不可变。
    contracts: BTreeMap<String, ContractCard>,
    /// D48：本次加载实际读取的文件与内容摘要（控制面审计用）。
    sources: Vec<RefSourceFile>,
}

#[derive(Debug, Clone)]
struct LoadedBindings {
    path: PathBuf,
    bindings: RefBindings,
}

/// D48：控制面审计中的一个配置源文件。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RefSourceFile {
    pub path: PathBuf,
    pub sha256: String,
    pub kind: RefSourceKind,
}

/// D48：配置源类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RefSourceKind {
    Bindings,
    Contract,
}

impl VisibilityProfiles {
    /// 无显式 profile：所有票面走默认 `read-gate`。
    pub fn empty() -> Self {
        Self::default()
    }

    /// 读取文件/目录并校验；任一文件失败即拒绝启动（fail-closed）。
    pub fn load(paths: &[PathBuf]) -> Result<Self, VisibilityError> {
        Self::load_with_contracts(paths, &[])
    }

    /// D47：加载 RefBindings 与 ContractCard，并校验“按数据触发”的引用闭环。
    ///
    /// 任一生效规则声明了 `contract` 时，必须配置 `[ref_contracts]` 且该契约可解析；
    /// 没有 `contract` 引用的部署不要求配置。重复 `contract_id()` 一律拒绝。
    pub fn load_with_contracts(
        binding_paths: &[PathBuf],
        contract_paths: &[PathBuf],
    ) -> Result<Self, VisibilityError> {
        let (contracts, contract_sources) = load_contracts(contract_paths)?;
        let mut bindings: BTreeMap<String, LoadedBindings> = BTreeMap::new();
        let mut binding_sources = Vec::new();
        for path in expand(binding_paths)? {
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
            binding_sources.push(RefSourceFile {
                sha256: sha256_hex(text.as_bytes()),
                path: path.clone(),
                kind: RefSourceKind::Bindings,
            });
            bindings.insert(
                domain,
                LoadedBindings {
                    path,
                    bindings: parsed,
                },
            );
        }
        verify_contract_references(&bindings, &contracts, !contract_paths.is_empty())?;
        let mut sources = binding_sources;
        sources.extend(contract_sources);
        Ok(Self {
            bindings,
            contracts,
            sources,
        })
    }

    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    /// 已加载的绑定域名（D48：重载授权需覆盖当前与新的全部绑定域）。
    pub fn domain_names(&self) -> impl Iterator<Item = &str> {
        self.bindings.keys().map(String::as_str)
    }

    /// D48：本次加载读取的源文件（路径 + 内容摘要 + 类别）。
    pub fn sources(&self) -> &[RefSourceFile] {
        &self.sources
    }

    /// 已加载的契约卡数量（D47）。
    pub fn contract_len(&self) -> usize {
        self.contracts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    pub fn has_domain(&self, domain: &str) -> bool {
        self.bindings.contains_key(domain)
    }

    /// D49 第二步：该域绑定记录的 `metadata.declarationDigest`。
    ///
    /// 未加载该域或绑定未记录摘要时为 `None`（运行期不比较）。
    pub fn declaration_digest(&self, domain: &str) -> Option<&str> {
        self.bindings
            .get(domain)?
            .bindings
            .metadata
            .declaration_digest
            .as_deref()
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
        let input = match input {
            None => EntryRef::Missing,
            Some(None) => EntryRef::Absent,
            Some(Some(raw)) => EntryRef::Present(raw),
        };
        let bindings = self.bindings.get(domain).map(|loaded| &loaded.bindings);
        validate_entry(bindings, Some(instance), ty, input)
    }

    /// D46/D47：进入 Archive（consume）前的钉版检查。
    ///
    /// 只读取继承出的 `pinning` 与绑定引用的 ContractCard，不重复执行写入期策略：
    /// 历史票面可能已经不符合收紧后的入口规则，但归档要求仍按当前绑定/卡片生效。
    /// 卡片只能加严（`reference.pinning.requiredForArchive` 或
    /// `artifact.digestPolicy: required-at-archive`），不能放宽绑定。`∅` 不受约束。
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
        let binding_requires = matches!(
            rule.pinning,
            Some(PinningRequirement::Required | PinningRequirement::RequiredAtArchive)
        );
        let card_requires = match rule.contract.as_deref() {
            Some(contract_id) => {
                let Some(card) = self.contracts.get(contract_id) else {
                    // 启动/重载校验应保证引用可解析；运行期仍 fail-closed。
                    return Err(RefEntryError::Invalid(SpecError::InvalidContract(format!(
                        "contract {contract_id} not loaded"
                    ))));
                };
                card_requires_archive_pinning(card)
            }
            None => false,
        };
        if binding_requires || card_requires {
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

/// 卡片是否要求归档钉版（只加严，不检查 `accepted`）。
fn card_requires_archive_pinning(card: &ContractCard) -> bool {
    let pinning = card
        .spec
        .reference
        .pinning
        .as_ref()
        .is_some_and(|pinning| pinning.required_for_archive);
    let digest_policy = card
        .spec
        .artifact
        .as_ref()
        .is_some_and(|artifact| artifact.digest_policy == DigestPolicy::RequiredAtArchive);
    pinning || digest_policy
}

/// D47：读取并校验契约卡；重复 `contract_id()`（无论内容）即拒绝。
fn load_contracts(
    paths: &[PathBuf],
) -> Result<(BTreeMap<String, ContractCard>, Vec<RefSourceFile>), VisibilityError> {
    let mut cards: BTreeMap<String, ContractCard> = BTreeMap::new();
    let mut sources: BTreeMap<String, PathBuf> = BTreeMap::new();
    let mut files = Vec::new();
    for path in expand_paths(paths)
        .map_err(|(path, source)| VisibilityError::ContractIo { path, source })?
    {
        let text =
            std::fs::read_to_string(&path).map_err(|source| VisibilityError::ContractIo {
                path: path.clone(),
                source,
            })?;
        let card: ContractCard =
            serde_yaml::from_str(&text).map_err(|error| VisibilityError::ContractParse {
                path: path.clone(),
                message: error.to_string(),
            })?;
        card.validate()
            .map_err(|error| VisibilityError::ContractInvalid {
                path: path.clone(),
                message: error.to_string(),
            })?;
        let contract_id = card.contract_id();
        if let Some(first) = sources.get(&contract_id) {
            // 同版本不可变：即使内容相同也拒绝，避免“哪份文件生效”依赖加载顺序。
            return Err(VisibilityError::DuplicateContract {
                contract_id,
                first: first.clone(),
                second: path,
            });
        }
        files.push(RefSourceFile {
            sha256: sha256_hex(text.as_bytes()),
            path: path.clone(),
            kind: RefSourceKind::Contract,
        });
        sources.insert(contract_id.clone(), path);
        cards.insert(contract_id, card);
    }
    Ok((cards, files))
}

/// D48：文件内容摘要（小写十六进制 SHA-256）。
fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write as _;
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// D47：按数据触发闭环。只扫描声明了 `contract` 的规则；未声明则不要求配置。
fn verify_contract_references(
    bindings: &BTreeMap<String, LoadedBindings>,
    contracts: &BTreeMap<String, ContractCard>,
    contracts_configured: bool,
) -> Result<(), VisibilityError> {
    let mut missing: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (domain, loaded) in bindings {
        for contract_id in contract_references(&loaded.bindings) {
            if !contracts.contains_key(&contract_id) {
                missing.entry(domain.clone()).or_default().push(contract_id);
            }
        }
    }
    if missing.is_empty() {
        return Ok(());
    }
    if !contracts_configured {
        let references = missing
            .into_values()
            .flatten()
            .collect::<std::collections::BTreeSet<_>>();
        return Err(VisibilityError::ContractsNotConfigured {
            references: references.into_iter().collect(),
        });
    }
    let (domain, contract_ids) = missing.into_iter().next().expect("非空");
    Err(VisibilityError::MissingContract {
        domain,
        contract_ids,
    })
}

/// 合并继承前的原始规则集合中出现过的 `contract` 引用（去重、排序）。
fn contract_references(bindings: &RefBindings) -> Vec<String> {
    let mut references = std::collections::BTreeSet::new();
    let mut push = |rule: &RefRule| {
        if let Some(contract) = &rule.contract {
            references.insert(contract.clone());
        }
    };
    if let Some(defaults) = &bindings.spec.defaults {
        push(&defaults.rule);
    }
    for binding in bindings.spec.types.values() {
        push(&binding.rule);
    }
    for rule in bindings.spec.schemes.values() {
        push(&rule.rule);
    }
    for instance in bindings.spec.instances.values() {
        for binding in instance.types.values() {
            push(&binding.rule);
        }
    }
    references.into_iter().collect()
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

/// 写入路径的票面校验错误（规范层定义，REST 与 WASM 共用）。
pub use pact_ref_spec::EntryError as RefEntryError;

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
    /// D47：契约卡读取失败。
    ContractIo {
        path: PathBuf,
        source: std::io::Error,
    },
    /// D47：契约卡解析失败。
    ContractParse {
        path: PathBuf,
        message: String,
    },
    /// D47：契约卡规范校验失败。
    ContractInvalid {
        path: PathBuf,
        message: String,
    },
    /// D47：同版本不可变——`contract_id()` 重复。
    DuplicateContract {
        contract_id: String,
        first: PathBuf,
        second: PathBuf,
    },
    /// D47：绑定声明了 `contract` 但未配置 `[ref_contracts]`。
    ContractsNotConfigured {
        references: Vec<String>,
    },
    /// D47：绑定引用的契约未加载。
    MissingContract {
        domain: String,
        contract_ids: Vec<String>,
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
            Self::ContractIo { path, source } => {
                write!(f, "读取 ContractCard {} 失败：{source}", path.display())
            }
            Self::ContractParse { path, message } => {
                write!(f, "解析 ContractCard {} 失败：{message}", path.display())
            }
            Self::ContractInvalid { path, message } => {
                write!(f, "ContractCard {} 校验失败：{message}", path.display())
            }
            Self::DuplicateContract {
                contract_id,
                first,
                second,
            } => write!(
                f,
                "契约 {contract_id} 在 {} 与 {} 中重复；同版本内容不可变，请提升 version",
                first.display(),
                second.display()
            ),
            Self::ContractsNotConfigured { references } => write!(
                f,
                "RefBindings 声明了契约 {}，但未配置 [ref_contracts].paths",
                references.join(", ")
            ),
            Self::MissingContract {
                domain,
                contract_ids,
            } => write!(
                f,
                "域 {domain} 引用的契约未加载：{}",
                contract_ids.join(", ")
            ),
        }
    }
}

impl std::error::Error for VisibilityError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } | Self::ContractIo { source, .. } => Some(source),
            _ => None,
        }
    }
}

fn expand(paths: &[PathBuf]) -> Result<Vec<PathBuf>, VisibilityError> {
    expand_paths(paths).map_err(|(path, source)| VisibilityError::Io { path, source })
}

/// 展开文件/目录；错误返回路径与 IO 原因，由调用方决定归属（绑定或卡片）。
fn expand_paths(paths: &[PathBuf]) -> Result<Vec<PathBuf>, (PathBuf, std::io::Error)> {
    let mut files = std::collections::BTreeSet::new();
    for path in paths {
        let metadata = std::fs::metadata(path).map_err(|source| (path.clone(), source))?;
        if metadata.is_dir() {
            let entries = std::fs::read_dir(path).map_err(|source| (path.clone(), source))?;
            for entry in entries {
                let entry = entry.map_err(|source| (path.clone(), source))?;
                let candidate = entry.path();
                if candidate.is_file() && is_yaml_or_json(&candidate) {
                    files.insert(candidate);
                }
            }
        } else if metadata.is_file() {
            files.insert(path.clone());
        } else {
            return Err((
                path.clone(),
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "既不是普通文件也不是目录"),
            ));
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

    const CONTRACT_BINDINGS: &str = r#"
apiVersion: pact.ref/v1
kind: RefBindings
metadata:
  domain: proj
spec:
  types:
    design:
      ref:
        contract: proj/design-contract@1
"#;

    const ARCHIVE_CARD: &str = r#"
apiVersion: pact.ref/v1
kind: ContractCard
metadata:
  id: proj/design-contract
  version: "1"
  owner: se
spec:
  artifact:
    kind: design
    mediaType: application/json
    digestPolicy: optional
  reference:
    schemes: ["https"]
    pinning:
      requiredForArchive: true
"#;

    const PLAIN_CARD: &str = r#"
apiVersion: pact.ref/v1
kind: ContractCard
metadata:
  id: proj/design-contract
  version: "2"
  owner: se
spec:
  artifact:
    kind: design
    mediaType: application/json
    digestPolicy: optional
  reference:
    schemes: ["https"]
"#;

    fn load_with_cards(
        bindings_text: &str,
        cards: &[(&str, &str)],
    ) -> (tempfile::TempDir, VisibilityProfiles) {
        let dir = tempfile::tempdir().unwrap();
        let binding_path = dir.path().join("refs.yaml");
        std::fs::write(&binding_path, bindings_text).unwrap();
        let mut card_paths = Vec::new();
        for (name, text) in cards {
            let path = dir.path().join(name);
            std::fs::write(&path, text).unwrap();
            card_paths.push(path);
        }
        let profiles =
            VisibilityProfiles::load_with_contracts(&[binding_path], &card_paths).unwrap();
        (dir, profiles)
    }

    #[test]
    fn contracts_load_and_resolve_binding_references() {
        let (_dir, profiles) = load_with_cards(CONTRACT_BINDINGS, &[("card.yaml", ARCHIVE_CARD)]);
        assert_eq!(profiles.contract_len(), 1);
        profiles
            .validate_archive_thing_ref("proj", "default", "design", Some(&pinned_uri()))
            .expect("pinned");
    }

    #[test]
    fn binding_contract_requires_ref_contracts_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("refs.yaml");
        std::fs::write(&path, CONTRACT_BINDINGS).unwrap();
        let error = VisibilityProfiles::load(&[path]).unwrap_err();
        assert!(
            error.to_string().contains("未配置 [ref_contracts]"),
            "{error}"
        );
    }

    #[test]
    fn unreferenced_contracts_are_optional() {
        let (_dir, profiles) = load_with_cards(BINDINGS, &[]);
        assert_eq!(profiles.contract_len(), 0);
    }

    #[test]
    fn missing_referenced_contract_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let binding_path = dir.path().join("refs.yaml");
        let card_path = dir.path().join("card.yaml");
        std::fs::write(&binding_path, CONTRACT_BINDINGS).unwrap();
        std::fs::write(&card_path, PLAIN_CARD).unwrap();
        let error =
            VisibilityProfiles::load_with_contracts(&[binding_path], &[card_path]).unwrap_err();
        assert!(error.to_string().contains("未加载"), "{error}");
    }

    #[test]
    fn duplicate_contract_version_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let binding_path = dir.path().join("refs.yaml");
        let first = dir.path().join("a.yaml");
        let second = dir.path().join("b.yaml");
        std::fs::write(&binding_path, CONTRACT_BINDINGS).unwrap();
        std::fs::write(&first, ARCHIVE_CARD).unwrap();
        std::fs::write(&second, ARCHIVE_CARD).unwrap();
        let error =
            VisibilityProfiles::load_with_contracts(&[binding_path], &[first, second]).unwrap_err();
        assert!(error.to_string().contains("重复"), "{error}");
    }

    #[test]
    fn card_pinning_blocks_unpinned_archive() {
        use super::RefEntryError;
        use pact_ref_spec::SpecError;

        let (_dir, profiles) = load_with_cards(CONTRACT_BINDINGS, &[("card.yaml", ARCHIVE_CARD)]);
        assert!(matches!(
            profiles.validate_archive_thing_ref(
                "proj",
                "default",
                "design",
                Some("https://example.com/design")
            ),
            Err(RefEntryError::Invalid(SpecError::PinningRequired {
                at_archive: true,
                ..
            }))
        ));
        profiles
            .validate_archive_thing_ref("proj", "default", "design", Some(&pinned_uri()))
            .expect("钉版可归档");
        profiles
            .validate_archive_thing_ref("proj", "default", "design", None)
            .expect("∅ 可归档");
    }

    #[test]
    fn card_pinning_only_tightens_binding() {
        use super::RefEntryError;
        use pact_ref_spec::SpecError;

        let bindings = r#"
apiVersion: pact.ref/v1
kind: RefBindings
metadata:
  domain: proj
spec:
  types:
    design:
      ref:
        contract: proj/design-contract@2
        pinning: required-at-archive
"#;
        let (_dir, profiles) = load_with_cards(bindings, &[("card.yaml", PLAIN_CARD)]);
        assert!(matches!(
            profiles.validate_archive_thing_ref(
                "proj",
                "default",
                "design",
                Some("https://example.com/design")
            ),
            Err(RefEntryError::Invalid(SpecError::PinningRequired {
                at_archive: true,
                ..
            }))
        ));
    }

    #[test]
    fn missing_contract_at_archive_fails_closed() {
        use super::{LoadedBindings, RefEntryError};
        use pact_ref_spec::RefBindings;

        // 绕过启动校验，模拟“绑定引用契约但索引缺失”的防御路径。
        let parsed: RefBindings = serde_yaml::from_str(CONTRACT_BINDINGS).unwrap();
        parsed.validate().unwrap();
        let mut profiles = VisibilityProfiles::empty();
        profiles.bindings.insert(
            "proj".to_owned(),
            LoadedBindings {
                path: std::path::PathBuf::from("refs.yaml"),
                bindings: parsed,
            },
        );
        let error = profiles
            .validate_archive_thing_ref(
                "proj",
                "default",
                "design",
                Some("https://example.com/design"),
            )
            .unwrap_err();
        assert!(matches!(error, RefEntryError::Invalid(_)));
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
