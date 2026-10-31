//! `pact ref lint`：离线检查 RefBindings / ContractCard / Tombstone 与 `.pact` 声明的静态一致性。
//!
//! 本模块只做读取、解析与规则交叉校验，不解析 `ThingRef` 存在性、不访问网络、
//! 不进入引擎。规范类型与校验全部来自 `pact-ref-spec`。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use pact_core::{Action, Domain};
use pact_http::dto::ErrorDetail;
use pact_ref_spec::{
    CardStatus, ContractCard, DeletionPolicy, PinningRequirement, RefBindings, ReferenceForm,
    SpecError, Tombstone, UnknownSchemePolicy, Visibility, DEFAULT_MAX_REFERENCE_LENGTH,
};
use serde::Serialize;

/// 检查结果级别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Severity {
    Error,
    Warning,
}

/// 单条检查结论。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub severity: Severity,
    pub code: String,
    pub path: String,
    pub message: String,
}

/// `pact ref lint` 的完整报告。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LintReport {
    pub bindings: usize,
    pub contracts: usize,
    pub tombstones: usize,
    pub errors: usize,
    pub warnings: usize,
    pub findings: Vec<Finding>,
}

impl LintReport {
    /// 是否应使命令失败（存在 error）。
    pub fn failed(&self) -> bool {
        self.errors > 0
    }
}

/// 命令输入（路径）。
#[derive(Debug, Clone, Default)]
pub struct LintOptions {
    pub bindings: Vec<PathBuf>,
    pub contracts: Vec<PathBuf>,
    pub tombstones: Vec<PathBuf>,
    pub declaration: Option<PathBuf>,
    pub expected_domain: Option<String>,
}

/// 已读取的文本输入；路径只用于报告。
#[derive(Debug, Clone)]
pub struct TextInput {
    pub path: String,
    pub text: String,
}

/// 读取文件并展开目录（`.yaml` / `.yml` / `.json`），按路径排序去重。
pub fn lint_paths(options: &LintOptions) -> Result<LintReport, ErrorDetail> {
    let binding_files = expand(&options.bindings)?;
    let contract_files = expand(&options.contracts)?;
    let tombstone_files = expand(&options.tombstones)?;
    let declaration = match &options.declaration {
        Some(path) => Some(read_text(path)?),
        None => None,
    };
    let expected_domain = options.expected_domain.clone().or_else(|| {
        options
            .declaration
            .as_ref()
            .and_then(|path| path.file_stem())
            .and_then(|stem| stem.to_str())
            .map(str::to_owned)
    });
    if tombstone_files.is_empty() {
        return Ok(lint_texts(
            read_inputs(binding_files)?,
            read_inputs(contract_files)?,
            declaration,
            expected_domain,
        ));
    }
    Ok(lint_texts_with_tombstones(
        read_inputs(binding_files)?,
        read_inputs(contract_files)?,
        read_inputs(tombstone_files)?,
        declaration,
        expected_domain,
    ))
}

/// 纯文本入口：便于单测与嵌入其他工具（不校验 Tombstone）。
pub fn lint_texts(
    bindings: Vec<TextInput>,
    contracts: Vec<TextInput>,
    declaration: Option<TextInput>,
    expected_domain: Option<String>,
) -> LintReport {
    lint_texts_with_tombstones(
        bindings,
        contracts,
        Vec::new(),
        declaration,
        expected_domain,
    )
}

/// 纯文本入口（含 Tombstone）：便于单测与嵌入其他工具。
pub fn lint_texts_with_tombstones(
    bindings: Vec<TextInput>,
    contracts: Vec<TextInput>,
    tombstones: Vec<TextInput>,
    declaration: Option<TextInput>,
    expected_domain: Option<String>,
) -> LintReport {
    let binding_count = bindings.len();
    let contract_count = contracts.len();
    let tombstone_count = tombstones.len();
    let mut sink = Sink::default();
    let mut parsed_bindings = Vec::new();
    for input in bindings {
        match serde_yaml::from_str::<RefBindings>(&input.text) {
            Ok(parsed) => match parsed.validate() {
                Ok(()) => parsed_bindings.push((input.path, parsed)),
                Err(error) => sink.error(
                    "ref.bindings.invalid",
                    &input.path,
                    format!("RefBindings 校验失败：{error}"),
                ),
            },
            Err(error) => sink.error(
                "ref.bindings.parse",
                &input.path,
                format!("无法解析 RefBindings：{error}"),
            ),
        }
    }

    let mut declared_domains: BTreeMap<String, String> = BTreeMap::new();
    for (path, parsed) in &parsed_bindings {
        let domain = parsed.metadata.domain.clone();
        if let Some(previous) = declared_domains.insert(domain.clone(), path.clone()) {
            sink.error(
                "ref.bindings.duplicate_domain",
                path,
                format!("域 `{domain}` 已由 {previous} 声明"),
            );
        }
    }

    let mut cards: BTreeMap<String, (String, ContractCard)> = BTreeMap::new();
    for input in contracts {
        match serde_yaml::from_str::<ContractCard>(&input.text) {
            Ok(card) => match card.validate() {
                Ok(()) => {
                    let id = card.contract_id();
                    if let Some((previous, _)) = cards.get(&id) {
                        sink.error(
                            "ref.contract.duplicate",
                            &input.path,
                            format!("契约 `{id}` 已在 {previous} 定义"),
                        );
                    } else {
                        cards.insert(id, (input.path, card));
                    }
                }
                Err(error) => sink.error(
                    "ref.contract.invalid",
                    &input.path,
                    format!("ContractCard 校验失败：{error}"),
                ),
            },
            Err(error) => sink.error(
                "ref.contract.parse",
                &input.path,
                format!("无法解析 ContractCard：{error}"),
            ),
        }
    }

    for (path, card) in cards.values() {
        let archive_pinned = card
            .spec
            .reference
            .pinning
            .as_ref()
            .map(|pinning| pinning.required_for_archive)
            .unwrap_or(false);
        if archive_pinned && card.spec.lifecycle.is_none() {
            sink.warning(
                "ref.contract.retention_missing",
                path,
                format!(
                    "契约 `{}` 要求归档钉版，但未声明 lifecycle.retention",
                    card.contract_id()
                ),
            );
        }
    }

    let mut parsed_tombstones: BTreeMap<String, (String, Tombstone)> = BTreeMap::new();
    let mut tombstone_refs: BTreeMap<(String, String), String> = BTreeMap::new();
    for input in tombstones {
        match serde_yaml::from_str::<Tombstone>(&input.text) {
            Ok(tombstone) => match tombstone.validate() {
                Ok(()) => {
                    let id = tombstone.metadata.id.clone();
                    let key = (
                        tombstone.spec.contract.clone(),
                        tombstone.spec.reference.clone(),
                    );
                    if let Some(previous) = tombstone_refs.insert(key, input.path.clone()) {
                        sink.warning(
                            "ref.tombstone.duplicate_ref",
                            &input.path,
                            format!(
                                "契约 `{}` 的引用 `{}` 已有 tombstone（见 {previous}）",
                                tombstone.spec.contract, tombstone.spec.reference
                            ),
                        );
                    }
                    if let Some((previous, _)) = parsed_tombstones.get(&id) {
                        sink.error(
                            "ref.tombstone.duplicate",
                            &input.path,
                            format!("tombstone `{id}` 已在 {previous} 定义"),
                        );
                    } else {
                        parsed_tombstones.insert(id, (input.path, tombstone));
                    }
                }
                Err(error) => sink.error(
                    "ref.tombstone.invalid",
                    &input.path,
                    format!("Tombstone 校验失败：{error}"),
                ),
            },
            Err(error) => sink.error(
                "ref.tombstone.parse",
                &input.path,
                format!("无法解析 Tombstone：{error}"),
            ),
        }
    }

    for (path, tombstone) in parsed_tombstones.values() {
        check_tombstone(&mut sink, path, tombstone, &cards);
    }

    let mut declaration_domain: Option<Domain> = None;
    if let Some(input) = declaration {
        match pact_parser::load_domain(&input.text) {
            Ok(domain) => declaration_domain = Some(domain),
            Err(problem) => sink.error(
                "ref.declaration.invalid",
                &input.path,
                problem_message(&problem),
            ),
        }
    }
    // D49：声明语义接口摘要；只有声明与域名都可得时才参与比对。
    let declaration_digest = match (&declaration_domain, expected_domain.as_deref()) {
        (Some(domain), Some(name)) => Some(pact_analysis::digest::declaration_digest(name, domain)),
        _ => None,
    };

    let mut referenced: BTreeSet<String> = BTreeSet::new();
    for (path, parsed) in &parsed_bindings {
        referenced.extend(contract_refs(parsed));
        check_bindings(
            &mut sink,
            path,
            parsed,
            declaration_domain.as_ref(),
            &cards,
            expected_domain.as_deref(),
            declaration_digest.as_deref(),
        );
    }

    for id in &referenced {
        if let Some((path, card)) = cards.get(id) {
            match card.metadata.status {
                CardStatus::Retired => sink.error(
                    "ref.contract.retired",
                    path,
                    format!("被引用的契约 `{id}` 状态为 retired"),
                ),
                CardStatus::Deprecated => sink.warning(
                    "ref.contract.deprecated",
                    path,
                    format!("被引用的契约 `{id}` 状态为 deprecated"),
                ),
                CardStatus::Draft | CardStatus::Active => {}
            }
        }
    }

    if !parsed_bindings.is_empty() {
        for (id, (path, _)) in &cards {
            if !referenced.contains(id) {
                sink.warning(
                    "ref.contract.unused",
                    path,
                    format!("契约 `{id}` 未被任何 RefBindings 引用"),
                );
            }
        }
    }

    LintReport {
        bindings: binding_count,
        contracts: contract_count,
        tombstones: tombstone_count,
        errors: sink.errors,
        warnings: sink.warnings,
        findings: sink.findings,
    }
}

/// 会随结果累积的检查器；按 `(code, path, message)` 去重。
#[derive(Default)]
struct Sink {
    findings: Vec<Finding>,
    seen: BTreeSet<(String, String, String)>,
    errors: usize,
    warnings: usize,
}

impl Sink {
    fn error(&mut self, code: &str, path: &str, message: impl Into<String>) {
        self.errors += 1;
        self.push(Severity::Error, code, path, message.into());
    }

    fn warning(&mut self, code: &str, path: &str, message: impl Into<String>) {
        self.warnings += 1;
        self.push(Severity::Warning, code, path, message.into());
    }

    fn push(&mut self, severity: Severity, code: &str, path: &str, message: String) {
        let key = (code.to_owned(), path.to_owned(), message.clone());
        if !self.seen.insert(key) {
            // 重复结论只记一次；对应计数也要回退。
            match severity {
                Severity::Error => self.errors -= 1,
                Severity::Warning => self.warnings -= 1,
            }
            return;
        }
        self.findings.push(Finding {
            severity,
            code: code.to_owned(),
            path: path.to_owned(),
            message,
        });
    }
}

fn check_bindings(
    sink: &mut Sink,
    path: &str,
    bindings: &RefBindings,
    declaration: Option<&Domain>,
    cards: &BTreeMap<String, (String, ContractCard)>,
    expected_domain: Option<&str>,
    declaration_digest: Option<&str>,
) {
    let known_types: Option<BTreeSet<String>> =
        declaration.map(|domain| domain.types.iter().map(|ty| ty.0.clone()).collect());

    if let Some(expected) = expected_domain {
        if bindings.metadata.domain != expected {
            sink.warning(
                "ref.bindings.domain_mismatch",
                path,
                format!(
                    "bindings 域 `{}` 与期望域名 `{expected}` 不一致",
                    bindings.metadata.domain
                ),
            );
        }
    }
    if declaration.is_some() && bindings.metadata.declaration_digest.is_none() {
        sink.warning(
            "ref.bindings.digest_missing",
            path,
            "未记录 declarationDigest，声明漂移无法检测",
        );
    }
    if let (Some(expected), Some(recorded)) = (
        declaration_digest,
        bindings.metadata.declaration_digest.as_deref(),
    ) {
        if recorded != expected {
            sink.error(
                "ref.bindings.digest_mismatch",
                path,
                format!(
                    "declarationDigest 与声明不一致：记录 `{recorded}`，实际 `{expected}`；\
                     声明变更后请用 `pact ref digest` 重新生成"
                ),
            );
        }
    }

    if let Some(types) = known_types.as_ref() {
        for ty in bindings.spec.types.keys() {
            if !types.contains(ty) {
                sink.error(
                    "ref.bindings.unknown_type",
                    path,
                    format!("类型 `{ty}` 不在 .pact 声明中"),
                );
            }
        }
        for (instance, instance_bindings) in &bindings.spec.instances {
            for ty in instance_bindings.types.keys() {
                if !types.contains(ty) {
                    sink.error(
                        "ref.bindings.unknown_type",
                        path,
                        format!("实例 `{instance}` 的类型 `{ty}` 不在 .pact 声明中"),
                    );
                }
            }
        }
    }

    for contract in contract_refs(bindings) {
        if !cards.contains_key(&contract) {
            if cards.is_empty() {
                sink.warning(
                    "ref.bindings.contract_unverified",
                    path,
                    format!("未提供 ContractCard，无法校验契约 `{contract}`"),
                );
            } else {
                sink.error(
                    "ref.bindings.contract_unknown",
                    path,
                    format!("契约 `{contract}` 没有对应的 ContractCard"),
                );
            }
        }
    }

    let mut types: BTreeSet<String> = bindings.spec.types.keys().cloned().collect();
    for instance_bindings in bindings.spec.instances.values() {
        types.extend(instance_bindings.types.keys().cloned());
    }
    if let Some(known) = known_types.as_ref() {
        types.extend(known.iter().cloned());
    }

    let instances: Vec<Option<&str>> = std::iter::once(None)
        .chain(bindings.spec.instances.keys().map(String::as_str).map(Some))
        .collect();

    let mut schemes: BTreeSet<String> = bindings.spec.schemes.keys().cloned().collect();
    for (_, card) in cards.values() {
        schemes.extend(card.spec.reference.schemes.iter().cloned());
    }
    schemes.extend(allowed_schemes(bindings));
    if schemes.is_empty() {
        schemes.insert("https".to_owned());
    }

    let mut required_contracts: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let discoverability = DiscoverabilityCtx {
        path,
        bindings,
        schemes: &schemes,
        declaration,
    };
    for instance in &instances {
        for ty in &types {
            check_effective(sink, path, bindings, *instance, ty, &schemes, cards);
            check_discoverability(
                sink,
                &discoverability,
                *instance,
                ty,
                &mut required_contracts,
            );
        }
    }
    check_required_contracts(sink, cards, &required_contracts);
}

fn check_effective(
    sink: &mut Sink,
    path: &str,
    bindings: &RefBindings,
    instance: Option<&str>,
    ty: &str,
    schemes: &BTreeSet<String>,
    cards: &BTreeMap<String, (String, ContractCard)>,
) {
    let scope = match instance {
        Some(instance) => format!("{instance}/{ty}"),
        None => ty.to_owned(),
    };
    let contracts = contracts_for(bindings, instance, ty);

    // `∅`：观察 required / 继承后的策略；required 但没有契约时给出治理缺口。
    match bindings.resolve(instance, ty, &ReferenceForm::Absent) {
        Ok(matched) => {
            if matched.rule.allow_identifier == Some(true) {
                sink.warning(
                    "ref.bindings.legacy_identifier",
                    path,
                    format!("`{scope}` 仍允许 Identifier，兼容旧引用"),
                );
            }
            if matched.rule.on_unknown_scheme == Some(UnknownSchemePolicy::Allow) {
                sink.warning(
                    "ref.bindings.open_unknown_scheme",
                    path,
                    format!("`{scope}` 允许未注册 scheme 透传"),
                );
            }
        }
        Err(SpecError::ReferenceRequired) => {
            if contracts.is_empty() {
                sink.warning(
                    "ref.bindings.required_without_contract",
                    path,
                    format!("`{scope}` 要求引用但没有 ContractCard"),
                );
            }
        }
        Err(_) => {}
    }

    // Identifier：只有显式放行才告警；默认拒绝是规范行为。
    if bindings
        .resolve(
            instance,
            ty,
            &ReferenceForm::Identifier("legacy-id".to_owned()),
        )
        .is_ok()
    {
        sink.warning(
            "ref.bindings.legacy_identifier",
            path,
            format!("`{scope}` 仍允许 Identifier，兼容旧引用"),
        );
    }

    for scheme in schemes {
        let sample = format!("{scheme}://lint.invalid/ref");
        let Ok(reference) = ReferenceForm::classify(Some(&sample)) else {
            continue;
        };
        match bindings.resolve(instance, ty, &reference) {
            Ok(matched) => {
                if matched.rule.allow_identifier == Some(true) {
                    sink.warning(
                        "ref.bindings.legacy_identifier",
                        path,
                        format!("`{scope}` 仍允许 Identifier，兼容旧引用"),
                    );
                }
                if matched.rule.on_unknown_scheme == Some(UnknownSchemePolicy::Allow) {
                    sink.warning(
                        "ref.bindings.open_unknown_scheme",
                        path,
                        format!("`{scope}` 允许未注册 scheme 透传"),
                    );
                }
                let Some(contract) = matched.rule.contract.as_deref() else {
                    if matched.rule.required == Some(true) && contracts.is_empty() {
                        sink.warning(
                            "ref.bindings.required_without_contract",
                            path,
                            format!("`{scope}` 要求引用但没有 ContractCard"),
                        );
                    }
                    continue;
                };
                let Some((card_path, card)) = cards.get(contract) else {
                    continue;
                };
                if !card.spec.reference.schemes.iter().any(|it| it == scheme) {
                    sink.error(
                        "ref.bindings.contract_scheme_mismatch",
                        path,
                        format!(
                            "`{scope}` 在 scheme `{scheme}` 下绑定契约 `{contract}`，但该契约不允许此 scheme（见 {card_path}）"
                        ),
                    );
                }
                let effective_max = matched
                    .rule
                    .max_length
                    .unwrap_or(DEFAULT_MAX_REFERENCE_LENGTH);
                if let Some(card_max) = card.spec.reference.max_length {
                    if effective_max > card_max {
                        sink.warning(
                            "ref.bindings.max_length_exceeds_contract",
                            path,
                            format!(
                                "`{scope}` 允许引用长度 {effective_max}，超过契约 `{contract}` 的 {card_max}"
                            ),
                        );
                    }
                }
                let archive_pinned = card
                    .spec
                    .reference
                    .pinning
                    .as_ref()
                    .map(|pinning| pinning.required_for_archive)
                    .unwrap_or(false);
                if archive_pinned
                    && !matches!(
                        matched.rule.pinning,
                        Some(PinningRequirement::Required | PinningRequirement::RequiredAtArchive)
                    )
                {
                    sink.warning(
                        "ref.bindings.pinning_weaker_than_contract",
                        path,
                        format!("契约 `{contract}` 要求归档钉版，但 `{scope}` 的 pinning 未覆盖"),
                    );
                }
            }
            Err(SpecError::SchemeNotAllowed {
                scheme: rejected, ..
            }) => {
                // 绑定收窄是允许的；仅当契约明确支持该 scheme 时提示能力未启用。
                for contract in &contracts {
                    let Some((card_path, card)) = cards.get(contract) else {
                        continue;
                    };
                    if card.spec.reference.schemes.iter().any(|it| it == &rejected) {
                        sink.warning(
                            "ref.bindings.contract_scheme_narrowed",
                            path,
                            format!(
                                "`{scope}` 不接受契约 `{contract}` 支持的 scheme `{rejected}`（见 {card_path}）"
                            ),
                        );
                    }
                }
            }
            Err(_) => {}
        }
    }
}

/// 可发现性检查的公共输入：一次 lint 运行内不变的部分。
struct DiscoverabilityCtx<'a> {
    path: &'a str,
    bindings: &'a RefBindings,
    schemes: &'a BTreeSet<String>,
    declaration: Option<&'a Domain>,
}

/// 可发现性检查（FR29 延伸）：要求引用的类型，下游是否能拿到票面与保留证据。
///
/// 只对 `required=true` 生效；实例级只在实例显式声明该类型时检查，避免与类型级重复。
fn check_discoverability(
    sink: &mut Sink,
    ctx: &DiscoverabilityCtx<'_>,
    instance: Option<&str>,
    ty: &str,
    required_contracts: &mut BTreeMap<String, BTreeSet<String>>,
) {
    let DiscoverabilityCtx {
        path,
        bindings,
        schemes,
        declaration,
    } = *ctx;
    if let Some(instance) = instance {
        let explicit = bindings
            .spec
            .instances
            .get(instance)
            .is_some_and(|instance_bindings| instance_bindings.types.contains_key(ty));
        if !explicit {
            return;
        }
    }
    let scope = match instance {
        Some(instance) => format!("{instance}/{ty}"),
        None => ty.to_owned(),
    };

    let mut required = false;
    let mut hidden = false;
    let mut domain_visible = false;
    let mut read_gate = false;

    for scheme in schemes {
        let sample = format!("{scheme}://lint.invalid/ref");
        let Ok(reference) = ReferenceForm::classify(Some(&sample)) else {
            continue;
        };
        let Ok(matched) = bindings.resolve(instance, ty, &reference) else {
            continue;
        };
        if matched.rule.required != Some(true) {
            continue;
        }
        required = true;
        if let Some(contract) = matched.rule.contract.as_ref() {
            required_contracts
                .entry(contract.clone())
                .or_default()
                .insert(scope.clone());
        }
        match matched.rule.visibility {
            Some(Visibility::Hidden) => hidden = true,
            Some(Visibility::Domain) => domain_visible = true,
            Some(Visibility::ReadGate) | None => read_gate = true,
        }
    }

    if !required {
        return;
    }

    if hidden {
        sink.warning(
            "ref.discoverability.required_hidden",
            path,
            format!(
                "`{scope}` 要求引用但 effective visibility=hidden，Counter/Archive 永不返回票面；下游只能依赖 Rule-R/consume"
            ),
        );
    }
    if domain_visible {
        return;
    }
    let Some(domain) = declaration else {
        return;
    };
    if read_gate && !has_view_read_path(domain, ty) {
        sink.warning(
            "ref.discoverability.view_read_path_missing",
            path,
            format!(
                "`{scope}` 默认 read-gate，但 .pact 声明中没有对 `{ty}` 的无条件 reads/consumes，下游无法从视图获得票面"
            ),
        );
    }
    if !has_any_reader(domain, ty) {
        sink.warning(
            "ref.discoverability.no_reader",
            path,
            format!(
                "`{scope}` 要求引用，但 .pact 声明中没有参与者具备 `{ty}` 的 reads/consumes，票面无法被下游取回"
            ),
        );
    }
}

/// 视图暴露 read-gate 票面的条件：某个参与者对类型有无条件 `reads` 或 `consumes`。
fn has_view_read_path(domain: &Domain, ty: &str) -> bool {
    domain
        .gates
        .iter()
        .any(|((_, action, gate_ty), condition)| {
            gate_ty.0 == ty
                && condition.is_none()
                && matches!(action, Action::Read | Action::Consume)
        })
}

/// 是否存在任何 Rule-R / Rule-E 读取路径（条件子句静态视为“可能成立”）。
fn has_any_reader(domain: &Domain, ty: &str) -> bool {
    domain.gates.keys().any(|(_, action, gate_ty)| {
        gate_ty.0 == ty && matches!(action, Action::Read | Action::Consume)
    })
}

/// 被 `required=true` 引用到的契约：保留证据与删除可区分性。
fn check_required_contracts(
    sink: &mut Sink,
    cards: &BTreeMap<String, (String, ContractCard)>,
    required_contracts: &BTreeMap<String, BTreeSet<String>>,
) {
    for (contract, scopes) in required_contracts {
        let Some((card_path, card)) = cards.get(contract) else {
            continue;
        };
        let archive_pinned = card
            .spec
            .reference
            .pinning
            .as_ref()
            .map(|pinning| pinning.required_for_archive)
            .unwrap_or(false);
        let scope_list = scopes.iter().cloned().collect::<Vec<_>>().join(", ");
        match &card.spec.lifecycle {
            None => {
                // 归档钉版的缺失已由 `ref.contract.retention_missing` 覆盖。
                if !archive_pinned {
                    sink.warning(
                        "ref.discoverability.retention_missing",
                        card_path,
                        format!(
                            "契约 `{contract}` 被要求引用（{scope_list}）使用，但未声明 lifecycle.retention，下游无法判断保留期"
                        ),
                    );
                }
            }
            Some(lifecycle) => {
                if lifecycle.deletion_policy == DeletionPolicy::Delete {
                    sink.warning(
                        "ref.discoverability.deletion_without_tombstone",
                        card_path,
                        format!(
                            "契约 `{contract}` 被要求引用（{scope_list}）使用，但 deletionPolicy=delete 且无 tombstone，下游无法区分删除与故障"
                        ),
                    );
                }
            }
        }
    }
}

/// Tombstone 与 ContractCard 的交叉校验（D44）。
fn check_tombstone(
    sink: &mut Sink,
    path: &str,
    tombstone: &Tombstone,
    cards: &BTreeMap<String, (String, ContractCard)>,
) {
    let id = &tombstone.spec.contract;
    let Some((card_path, card)) = cards.get(id) else {
        if cards.is_empty() {
            sink.warning(
                "ref.tombstone.contract_unverified",
                path,
                format!("未提供 ContractCard，无法校验契约 `{id}`"),
            );
        } else {
            sink.error(
                "ref.tombstone.contract_unknown",
                path,
                format!("tombstone 引用的契约 `{id}` 没有对应的 ContractCard"),
            );
        }
        return;
    };

    let policy = card
        .spec
        .lifecycle
        .as_ref()
        .map(|lifecycle| lifecycle.deletion_policy);
    if policy != Some(DeletionPolicy::TombstoneThenDelete) {
        sink.error(
            "ref.tombstone.policy_conflict",
            path,
            format!("契约 `{id}` 的 deletionPolicy 不是 tombstone-then-delete（见 {card_path}）"),
        );
    }

    check_tombstone_reference(
        sink,
        path,
        id,
        card_path,
        card,
        "ref",
        &tombstone.spec.reference,
    );
    if let Some(replaced_by) = &tombstone.spec.replaced_by {
        check_tombstone_reference(sink, path, id, card_path, card, "replacedBy", replaced_by);
    }
}

fn check_tombstone_reference(
    sink: &mut Sink,
    path: &str,
    contract: &str,
    card_path: &str,
    card: &ContractCard,
    field: &str,
    value: &str,
) {
    let Ok(form) = ReferenceForm::classify(Some(value)) else {
        return;
    };
    if let Some(scheme) = form.scheme() {
        if !card.spec.reference.schemes.iter().any(|it| it == scheme) {
            sink.error(
                "ref.tombstone.scheme_not_allowed",
                path,
                format!(
                    "tombstone 的 {field} scheme `{scheme}` 不被契约 `{contract}` 允许（见 {card_path}）"
                ),
            );
        }
    }
    let max_length = card
        .spec
        .reference
        .max_length
        .unwrap_or(DEFAULT_MAX_REFERENCE_LENGTH);
    if value.len() > max_length {
        sink.error(
            "ref.tombstone.max_length_exceeded",
            path,
            format!(
                "tombstone 的 {field} 长度 {} 超过契约 `{contract}` 的 {max_length}",
                value.len()
            ),
        );
    }
}

/// 所有规则显式允许的 scheme（排除通配 `*`），用于生成采样引用。
fn allowed_schemes(bindings: &RefBindings) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut collect = |rule: &pact_ref_spec::RefRule| {
        if let Some(schemes) = &rule.schemes {
            out.extend(
                schemes
                    .iter()
                    .filter(|scheme| scheme.as_str() != "*")
                    .cloned(),
            );
        }
    };
    if let Some(entry) = &bindings.spec.defaults {
        collect(&entry.rule);
    }
    for entry in bindings.spec.types.values() {
        collect(&entry.rule);
    }
    for entry in bindings.spec.schemes.values() {
        collect(&entry.rule);
    }
    for instance in bindings.spec.instances.values() {
        for entry in instance.types.values() {
            collect(&entry.rule);
        }
    }
    out
}

/// 该 `(instance, type)` 可能命中的契约集合（忽略 scheme 条件）。
fn contracts_for(bindings: &RefBindings, instance: Option<&str>, ty: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    if let Some(entry) = &bindings.spec.defaults {
        if let Some(contract) = &entry.rule.contract {
            out.insert(contract.clone());
        }
    }
    if let Some(entry) = bindings.spec.types.get(ty) {
        if let Some(contract) = &entry.rule.contract {
            out.insert(contract.clone());
        }
    }
    if let Some(instance) = instance {
        if let Some(entry) = bindings
            .spec
            .instances
            .get(instance)
            .and_then(|it| it.types.get(ty))
        {
            if let Some(contract) = &entry.rule.contract {
                out.insert(contract.clone());
            }
        }
    }
    for entry in bindings.spec.schemes.values() {
        if let Some(contract) = &entry.rule.contract {
            out.insert(contract.clone());
        }
    }
    out
}

/// 文件中声明的全部契约引用（含 defaults / schemes / instances）。
fn contract_refs(bindings: &RefBindings) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    if let Some(entry) = &bindings.spec.defaults {
        if let Some(contract) = &entry.rule.contract {
            out.insert(contract.clone());
        }
    }
    for entry in bindings.spec.types.values() {
        if let Some(contract) = &entry.rule.contract {
            out.insert(contract.clone());
        }
    }
    for entry in bindings.spec.schemes.values() {
        if let Some(contract) = &entry.rule.contract {
            out.insert(contract.clone());
        }
    }
    for instance in bindings.spec.instances.values() {
        for entry in instance.types.values() {
            if let Some(contract) = &entry.rule.contract {
                out.insert(contract.clone());
            }
        }
    }
    out
}

fn problem_message(problem: &pact_parser::Problem) -> String {
    let (kind, count) = match problem {
        pact_parser::Problem::Syntax(errors) => ("语法错误", errors.len()),
        pact_parser::Problem::WellFormedness(errors) => ("良构违例", errors.len()),
    };
    let detail = match problem {
        pact_parser::Problem::Syntax(errors) => errors.first().map(|error| {
            format!(
                "第 {} 行第 {} 列 {}",
                error.span.line, error.span.column, error.message
            )
        }),
        pact_parser::Problem::WellFormedness(errors) => errors
            .first()
            .and_then(|error| serde_json::to_string(error).ok()),
    };
    match detail {
        Some(detail) => format!(".pact 声明{kind}（{count} 处）：{detail}"),
        None => format!(".pact 声明{kind}（{count} 处）"),
    }
}

fn expand(paths: &[PathBuf]) -> Result<Vec<PathBuf>, ErrorDetail> {
    let mut files = BTreeSet::new();
    for path in paths {
        let metadata = std::fs::metadata(path).map_err(|error| access_error(path, error))?;
        if metadata.is_dir() {
            let entries = std::fs::read_dir(path).map_err(|error| access_error(path, error))?;
            for entry in entries {
                let entry = entry.map_err(|error| access_error(path, error))?;
                let candidate = entry.path();
                if candidate.is_file() && is_yaml_or_json(&candidate) {
                    files.insert(candidate);
                }
            }
        } else if metadata.is_file() {
            files.insert(path.clone());
        } else {
            return Err(access_error(
                path,
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

fn read_inputs(paths: Vec<PathBuf>) -> Result<Vec<TextInput>, ErrorDetail> {
    paths.iter().map(|path| read_text(path)).collect()
}

fn read_text(path: &Path) -> Result<TextInput, ErrorDetail> {
    std::fs::read_to_string(path)
        .map(|text| TextInput {
            path: path.display().to_string(),
            text,
        })
        .map_err(|error| access_error(path, error))
}

fn access_error(path: &Path, error: std::io::Error) -> ErrorDetail {
    ErrorDetail {
        code: "Io".to_owned(),
        detail: serde_json::json!({ "path": path.display().to_string() }),
        message: format!("读取 {} 失败：{error}", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::{lint_texts, lint_texts_with_tombstones, Severity, TextInput};

    const DECL: &str = r#"
types { requirement; design; implementation }
preconditions { requirement → design; design → implementation }
gates {
    architect: reads requirement, consumes requirement, produces design;
    developer: consumes design, produces implementation;
    user: consumes implementation
}
seed_authority { external: seeds requirement }
"#;

    const CARD: &str = r#"
apiVersion: pact.ref/v1
kind: ContractCard
metadata:
    id: se/design-review
    version: "2"
    owner: architecture-guild
    status: active
spec:
    artifact:
        kind: document
        mediaType: text/markdown
        digestPolicy: optional
    reference:
        schemes: [https]
        pinning:
            requiredForArchive: true
    lifecycle:
        retention: P365D
        deletionPolicy: tombstone-then-delete
        tombstoneRef: urn:pact:tombstone:se/design-review
"#;

    const BINDINGS: &str = r#"
apiVersion: pact.ref/v1
kind: RefBindings
metadata:
    domain: se_lifecycle
    declarationDigest: sha256:19fe1f29f567a5314572ba8af957129e9570cff3de6ca3bc0da9523441bdcbfb
spec:
    types:
        design:
            ref:
                required: true
                schemes: [https]
                contract: se/design-review@2
                pinning: required-at-archive
"#;

    fn input(name: &str, text: &str) -> TextInput {
        TextInput {
            path: name.to_owned(),
            text: text.to_owned(),
        }
    }

    const TOMBSTONE: &str = r#"
apiVersion: pact.ref/v1
kind: Tombstone
metadata:
    id: "urn:pact:tombstone:se/design-review:req-001"
    owner: architecture-guild
spec:
    contract: se/design-review@2
    ref: https://artifacts.example.com/design/req-001
    reason: expired
    deletedAt: 2026-10-27T00:00:00Z
    retainUntil: 2027-10-27T00:00:00Z
"#;

    fn lint(bindings: &str, cards: &[&str]) -> super::LintReport {
        lint_with_tombstones(bindings, cards, &[])
    }

    fn lint_with_tombstones(
        bindings: &str,
        cards: &[&str],
        tombstones: &[&str],
    ) -> super::LintReport {
        lint_texts_with_tombstones(
            vec![input("refs.yaml", bindings)],
            cards.iter().map(|card| input("card.yaml", card)).collect(),
            tombstones
                .iter()
                .enumerate()
                .map(|(index, tombstone)| input(&format!("tombstone-{index}.yaml"), tombstone))
                .collect(),
            Some(input("se_lifecycle.pact", DECL)),
            Some("se_lifecycle".to_owned()),
        )
    }

    #[test]
    fn accepts_consistent_inputs() {
        let report = lint(BINDINGS, &[CARD]);
        assert_eq!(report.errors, 0, "findings: {:?}", report.findings);
        assert_eq!(report.warnings, 0, "findings: {:?}", report.findings);
    }

    #[test]
    fn reports_digest_missing_and_mismatch() {
        // 缺失：仍是 warning，不阻断。
        let missing = BINDINGS.replace(
            "    declarationDigest: sha256:19fe1f29f567a5314572ba8af957129e9570cff3de6ca3bc0da9523441bdcbfb\n",
            "",
        );
        let report = lint(&missing, &[CARD]);
        assert_eq!(report.errors, 0, "findings: {:?}", report.findings);
        assert!(report.findings.iter().any(|finding| {
            finding.code == "ref.bindings.digest_missing" && finding.severity == Severity::Warning
        }));

        // 不一致：error，阻断（D49 第一步）。
        let mismatched = BINDINGS.replace(
            "sha256:19fe1f29f567a5314572ba8af957129e9570cff3de6ca3bc0da9523441bdcbfb",
            "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        );
        let report = lint(&mismatched, &[CARD]);
        assert!(report.errors > 0, "findings: {:?}", report.findings);
        assert!(report.findings.iter().any(|finding| {
            finding.code == "ref.bindings.digest_mismatch" && finding.severity == Severity::Error
        }));
    }

    #[test]
    fn reports_unknown_type() {
        let bindings = BINDINGS.replace("        design:", "        desgin:");
        let report = lint(&bindings, &[CARD]);
        assert!(report.errors > 0);
        assert!(report
            .findings
            .iter()
            .any(|finding| finding.code == "ref.bindings.unknown_type"));
    }

    #[test]
    fn reports_missing_and_unused_contracts() {
        let report = lint(BINDINGS, &[]);
        assert_eq!(report.errors, 0);
        assert!(report.findings.iter().any(|finding| {
            finding.code == "ref.bindings.contract_unverified"
                && finding.severity == Severity::Warning
        }));

        let unused = r#"
apiVersion: pact.ref/v1
kind: ContractCard
metadata: { id: other/card, version: "1", owner: team }
spec:
    artifact: { kind: document, mediaType: text/markdown, digestPolicy: optional }
    reference: { schemes: [https] }
"#;
        let report = lint(BINDINGS, &[unused]);
        assert!(report
            .findings
            .iter()
            .any(|finding| finding.code == "ref.bindings.contract_unknown"));
        assert!(report
            .findings
            .iter()
            .any(|finding| finding.code == "ref.contract.unused"));
    }

    #[test]
    fn reports_scheme_conflict() {
        let card = CARD.replace("schemes: [https]", "schemes: [git]");
        let report = lint(BINDINGS, &[&card]);
        let codes: Vec<_> = report
            .findings
            .iter()
            .map(|finding| finding.code.as_str())
            .collect();
        assert!(
            codes.contains(&"ref.bindings.contract_scheme_mismatch"),
            "findings: {:?}",
            report.findings
        );
    }

    #[test]
    fn reports_legacy_identifier() {
        let bindings = r#"
apiVersion: pact.ref/v1
kind: RefBindings
metadata: { domain: se_lifecycle }
spec:
    types:
        design:
            ref:
                allowIdentifier: true
"#;
        let report = lint(bindings, &[]);
        assert!(report.findings.iter().any(|finding| {
            finding.code == "ref.bindings.legacy_identifier"
                && finding.severity == Severity::Warning
        }));
    }

    #[test]
    fn reports_duplicate_and_retired_contracts() {
        let retired = CARD.replace("status: active", "status: retired");
        let report = lint(BINDINGS, &[&retired, &retired]);
        let codes: Vec<_> = report
            .findings
            .iter()
            .map(|finding| finding.code.as_str())
            .collect();
        assert!(
            codes.contains(&"ref.contract.duplicate"),
            "codes: {codes:?}"
        );
        assert!(codes.contains(&"ref.contract.retired"), "codes: {codes:?}");
    }

    #[test]
    fn reports_required_without_contract() {
        let bindings = r#"
apiVersion: pact.ref/v1
kind: RefBindings
metadata: { domain: se_lifecycle }
spec:
    types:
        design:
            ref:
                required: true
                schemes: [https]
"#;
        let report = lint(bindings, &[]);
        assert!(report.findings.iter().any(|finding| {
            finding.code == "ref.bindings.required_without_contract"
                && finding.severity == Severity::Warning
        }));
    }

    #[test]
    fn reports_declaration_problems() {
        let report = lint_texts(
            vec![input("refs.yaml", BINDINGS)],
            vec![],
            Some(input("broken.pact", "types { }")),
            None,
        );
        assert!(report
            .findings
            .iter()
            .any(|finding| finding.code == "ref.declaration.invalid"));
    }

    #[test]
    fn accepts_consistent_tombstone() {
        let report = lint_with_tombstones(BINDINGS, &[CARD], &[TOMBSTONE]);
        assert_eq!(report.errors, 0, "findings: {:?}", report.findings);
        assert_eq!(report.warnings, 0, "findings: {:?}", report.findings);
        assert_eq!(report.tombstones, 1);
    }

    #[test]
    fn reports_retention_missing_for_archive_pinned_card() {
        let card = CARD.replace(
            "    lifecycle:\n        retention: P365D\n        deletionPolicy: tombstone-then-delete\n        tombstoneRef: urn:pact:tombstone:se/design-review\n",
            "",
        );
        let report = lint(BINDINGS, &[&card]);
        assert!(report.findings.iter().any(|finding| {
            finding.code == "ref.contract.retention_missing"
                && finding.severity == Severity::Warning
        }));
    }

    #[test]
    fn reports_tombstone_policy_conflict_and_unknown_contract() {
        let retained = CARD.replace(
            "        deletionPolicy: tombstone-then-delete\n        tombstoneRef: urn:pact:tombstone:se/design-review",
            "        deletionPolicy: retain",
        );
        let report = lint_with_tombstones(BINDINGS, &[&retained], &[TOMBSTONE]);
        assert!(report
            .findings
            .iter()
            .any(|finding| finding.code == "ref.tombstone.policy_conflict"));

        let unknown = TOMBSTONE.replace("se/design-review@2", "other/card@1");
        let report = lint_with_tombstones(BINDINGS, &[CARD], &[&unknown]);
        assert!(report
            .findings
            .iter()
            .any(|finding| finding.code == "ref.tombstone.contract_unknown"));
    }

    #[test]
    fn reports_tombstone_scheme_and_duplicates() {
        let tombstone = TOMBSTONE.replace(
            "https://artifacts.example.com/design/req-001",
            "git://host/repo",
        );
        let report = lint_with_tombstones(BINDINGS, &[CARD], &[&tombstone]);
        assert!(
            report
                .findings
                .iter()
                .any(|finding| finding.code == "ref.tombstone.scheme_not_allowed"),
            "findings: {:?}",
            report.findings
        );

        let report = lint_with_tombstones(BINDINGS, &[CARD], &[TOMBSTONE, TOMBSTONE]);
        let codes: Vec<_> = report
            .findings
            .iter()
            .map(|finding| finding.code.as_str())
            .collect();
        assert!(codes.contains(&"ref.tombstone.duplicate"), "{codes:?}");
        assert!(codes.contains(&"ref.tombstone.duplicate_ref"), "{codes:?}");
    }

    #[test]
    fn reports_invalid_tombstone() {
        let invalid = TOMBSTONE.replace("2026-10-27T00:00:00Z", "2026-02-30T00:00:00Z");
        let report = lint_with_tombstones(BINDINGS, &[CARD], &[&invalid]);
        assert!(report
            .findings
            .iter()
            .any(|finding| finding.code == "ref.tombstone.invalid"));
    }

    fn lint_with_declaration(
        bindings: &str,
        cards: &[&str],
        declaration: &str,
    ) -> super::LintReport {
        lint_texts_with_tombstones(
            vec![input("refs.yaml", bindings)],
            cards.iter().map(|card| input("card.yaml", card)).collect(),
            vec![],
            Some(input("se_lifecycle.pact", declaration)),
            Some("se_lifecycle".to_owned()),
        )
    }

    const DECL_WITHOUT_READERS: &str = r#"
types { requirement; design; implementation }
preconditions { requirement → design; design → implementation }
gates {
    architect: produces design;
    developer: produces implementation;
    user: consumes implementation
}
seed_authority { external: seeds requirement }
"#;

    #[test]
    fn reports_discoverability_visibility_gaps() {
        let hidden = BINDINGS.replace(
            "                schemes: [https]\n",
            "                schemes: [https]\n                visibility: hidden\n",
        );
        let report = lint(&hidden, &[CARD]);
        assert!(
            report
                .findings
                .iter()
                .any(|finding| finding.code == "ref.discoverability.required_hidden"),
            "findings: {:?}",
            report.findings
        );

        let no_read_path = lint_with_declaration(BINDINGS, &[CARD], DECL_WITHOUT_READERS);
        let codes: Vec<_> = no_read_path
            .findings
            .iter()
            .map(|finding| finding.code.as_str())
            .collect();
        assert!(
            codes.contains(&"ref.discoverability.view_read_path_missing"),
            "codes: {codes:?}"
        );
        assert!(
            codes.contains(&"ref.discoverability.no_reader"),
            "codes: {codes:?}"
        );

        // visibility=domain 时视图直接暴露票面，不应产生读取路径告警。
        let domain = hidden.replace("visibility: hidden", "visibility: domain");
        let report = lint_with_declaration(&domain, &[CARD], DECL_WITHOUT_READERS);
        let codes: Vec<_> = report
            .findings
            .iter()
            .map(|finding| finding.code.as_str())
            .collect();
        assert!(
            !codes.contains(&"ref.discoverability.view_read_path_missing"),
            "codes: {codes:?}"
        );
        assert!(
            !codes.contains(&"ref.discoverability.no_reader"),
            "codes: {codes:?}"
        );
    }

    #[test]
    fn reports_discoverability_retention_gaps() {
        let no_lifecycle = CARD
            .replace(
                "        pinning:\n            requiredForArchive: true\n",
                "",
            )
            .replace(
                "    lifecycle:\n        retention: P365D\n        deletionPolicy: tombstone-then-delete\n        tombstoneRef: urn:pact:tombstone:se/design-review\n",
                "",
            );
        let report = lint(BINDINGS, &[&no_lifecycle]);
        assert!(
            report
                .findings
                .iter()
                .any(|finding| finding.code == "ref.discoverability.retention_missing"),
            "findings: {:?}",
            report.findings
        );

        let deleted = CARD
            .replace(
                "        pinning:\n            requiredForArchive: true\n",
                "",
            )
            .replace(
                "        deletionPolicy: tombstone-then-delete\n        tombstoneRef: urn:pact:tombstone:se/design-review",
                "        deletionPolicy: delete",
            );
        let report = lint(BINDINGS, &[&deleted]);
        assert!(
            report
                .findings
                .iter()
                .any(|finding| finding.code == "ref.discoverability.deletion_without_tombstone"),
            "findings: {:?}",
            report.findings
        );
    }
}
