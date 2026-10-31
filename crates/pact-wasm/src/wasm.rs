//! wasm-bindgen 包装（仅 wasm32 目标）。
//!
//! ABI 约定（Q2 冻结）：所有方法参数与返回值均为 JSON 字符串；错误以
//! `{"code":"…","message":"…"}` 字符串抛出，`code` 与原生接口层同名。
//! 方法名直接作为 wasm 导出名，便于 Node/浏览器与原生 CLI 对账。
//!
//! P3 写路径：`prepare_*` → 宿主 append/fsync → `commit`；`abort` 丢弃待提交事务。

use super::{FacadeError, PactHost, DEFAULT_TS};
use pact_store_model::InstanceKey;
use serde::Serialize;
use wasm_bindgen::prelude::*;

/// 浏览器/Node 侧句柄：构造后生命周期内持有声明注册表与全部实例会话。
#[wasm_bindgen]
pub struct PactWasm {
    host: PactHost,
}

#[wasm_bindgen]
impl PactWasm {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            host: PactHost::new(),
        }
    }

    // —— 域与实例 ——

    /// `load_domain(name, declaration, instance) -> DomainSummary JSON`
    pub fn load_domain(
        &mut self,
        name: &str,
        declaration: &str,
        instance: &str,
    ) -> Result<String, JsValue> {
        to_json(
            self.host
                .load_domain(name, declaration, instance)
                .map_err(to_js)?,
        )
    }

    /// `load_domain_at(name, declaration, instance, ts)`：确定性时间戳注入（对账用）。
    pub fn load_domain_at(
        &mut self,
        name: &str,
        declaration: &str,
        instance: &str,
        ts: &str,
    ) -> Result<String, JsValue> {
        to_json(
            self.host
                .load_domain_at(name, declaration, instance, ts, false)
                .map_err(to_js)?,
        )
    }

    /// `load_domain_with(name, declaration, instance, ts, force)`：显式替换占用实例。
    pub fn load_domain_with(
        &mut self,
        name: &str,
        declaration: &str,
        instance: &str,
        ts: &str,
        force: bool,
    ) -> Result<String, JsValue> {
        to_json(
            self.host
                .load_domain_at(name, declaration, instance, ts, force)
                .map_err(to_js)?,
        )
    }

    /// `instantiate(domain, instance, ts)`：从已登记声明创建额外实例。
    pub fn instantiate(
        &mut self,
        domain: &str,
        instance: &str,
        ts: &str,
    ) -> Result<String, JsValue> {
        to_json(self.host.instantiate(domain, instance, ts).map_err(to_js)?)
    }

    /// `validate(declaration) -> ValidationView JSON`（不登记实例）
    pub fn validate(&self, declaration: &str) -> Result<String, JsValue> {
        to_json(self.host.validate(declaration).map_err(to_js)?)
    }

    /// `analyze(declaration) -> AnalysisReport JSON`（六项静态分析）
    pub fn analyze(&self, declaration: &str) -> Result<String, JsValue> {
        to_json(self.host.analyze(declaration).map_err(to_js)?)
    }

    /// `compose(name, fragments_json, instance) -> DomainSummary JSON`
    pub fn compose(
        &mut self,
        name: &str,
        fragments_json: &str,
        instance: &str,
    ) -> Result<String, JsValue> {
        to_json(
            self.host
                .compose(name, fragments_json, instance, DEFAULT_TS)
                .map_err(to_js)?,
        )
    }

    // —— D50：宿主注入的 RefBindings ——

    /// `set_ref_bindings(bindings_json)`：注入/替换一个域的绑定。
    pub fn set_ref_bindings(&mut self, bindings_json: &str) -> Result<(), JsValue> {
        self.host.set_ref_bindings(bindings_json).map_err(to_js)
    }

    /// `remove_ref_bindings(domain) -> bool`：移除一个域的绑定。
    pub fn remove_ref_bindings(&mut self, domain: &str) -> bool {
        self.host.remove_ref_bindings(domain)
    }

    /// `clear_ref_bindings()`：清空全部绑定。
    pub fn clear_ref_bindings(&mut self) {
        self.host.clear_ref_bindings();
    }

    // —— 四类动作（两阶段提交）——

    /// `prepare_seed(key, participant, ty, thing_ref, token_id, ts) -> Prepared JSON`
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_seed(
        &mut self,
        key: &str,
        participant: &str,
        ty: &str,
        thing_ref: Option<String>,
        token_id: &str,
        ts: &str,
    ) -> Result<String, JsValue> {
        let key = parse_key(key)?;
        to_json(
            self.host
                .prepare_seed(&key, participant, ty, thing_ref.as_deref(), token_id, ts)
                .map_err(to_js)?,
        )
    }

    /// `prepare_produce(key, participant, ty, predecessors_json, thing_ref, token_id, ts)`
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_produce(
        &mut self,
        key: &str,
        participant: &str,
        ty: &str,
        predecessors_json: &str,
        thing_ref: Option<String>,
        token_id: &str,
        ts: &str,
    ) -> Result<String, JsValue> {
        let key = parse_key(key)?;
        to_json(
            self.host
                .prepare_produce(
                    &key,
                    participant,
                    ty,
                    predecessors_json,
                    thing_ref.as_deref(),
                    token_id,
                    ts,
                )
                .map_err(to_js)?,
        )
    }

    /// `prepare_consume(key, participant, token_id, request_id, ts)`
    pub fn prepare_consume(
        &mut self,
        key: &str,
        participant: &str,
        token_id: &str,
        request_id: Option<String>,
        ts: &str,
    ) -> Result<String, JsValue> {
        let key = parse_key(key)?;
        to_json(
            self.host
                .prepare_consume(&key, participant, token_id, request_id.as_deref(), ts)
                .map_err(to_js)?,
        )
    }

    /// `commit(tx_id, terminated_ts) -> Committed JSON`（`terminated_ts` 可省略）
    pub fn commit(
        &mut self,
        tx_id: &str,
        terminated_ts: Option<String>,
    ) -> Result<String, JsValue> {
        to_json(
            self.host
                .commit(tx_id, terminated_ts.as_deref())
                .map_err(to_js)?,
        )
    }

    /// `abort(tx_id)`：丢弃待提交事务，状态与日志均不变。
    pub fn abort(&mut self, tx_id: &str) -> Result<(), JsValue> {
        self.host.abort(tx_id).map_err(to_js)
    }

    /// `read(key, participant, token_id) -> Token JSON`（Rule-R，无事件）
    pub fn read(&self, key: &str, participant: &str, token_id: &str) -> Result<String, JsValue> {
        let key = parse_key(key)?;
        to_json(self.host.read(&key, participant, token_id).map_err(to_js)?)
    }

    // —— 查询 ——

    /// `counter(key, filter) -> CounterView JSON`
    pub fn counter(&self, key: &str, filter: Option<String>) -> Result<String, JsValue> {
        let key = parse_key(key)?;
        to_json(self.host.counter(&key, filter.as_deref()).map_err(to_js)?)
    }

    /// `archive(key, filter) -> ArchiveView JSON`
    pub fn archive(&self, key: &str, filter: Option<String>) -> Result<String, JsValue> {
        let key = parse_key(key)?;
        to_json(self.host.archive(&key, filter.as_deref()).map_err(to_js)?)
    }

    /// `gates(key, participant) -> GateClause[] JSON`
    pub fn gates(&self, key: &str, participant: &str) -> Result<String, JsValue> {
        let key = parse_key(key)?;
        to_json(self.host.gates(&key, participant).map_err(to_js)?)
    }

    /// `facets(key) -> DomainFacets JSON`（参与者、种子授权、前置图）
    pub fn facets(&self, key: &str) -> Result<String, JsValue> {
        let key = parse_key(key)?;
        to_json(self.host.facets(&key).map_err(to_js)?)
    }

    /// `enabled(key, participant) -> ProductionOption[] JSON`
    pub fn enabled(&self, key: &str, participant: &str) -> Result<String, JsValue> {
        let key = parse_key(key)?;
        to_json(self.host.enabled(&key, participant).map_err(to_js)?)
    }

    /// `status(key) -> StatusView JSON`
    pub fn status(&self, key: &str) -> Result<String, JsValue> {
        let key = parse_key(key)?;
        to_json(self.host.status(&key).map_err(to_js)?)
    }

    /// `list_domains() -> DomainListing[] JSON`
    pub fn list_domains(&self) -> Result<String, JsValue> {
        to_json(self.host.list_domains())
    }

    // —— 导出与恢复 ——

    /// `export_events(key) -> JSONL 字符串`（逐行含 `version`，宿主可直接写文件）
    pub fn export_events(&self, key: &str) -> Result<String, JsValue> {
        let key = parse_key(key)?;
        self.host.export_events(&key).map_err(to_js)
    }

    /// `export_snapshot(key) -> Snapshot JSON`
    pub fn export_snapshot(&self, key: &str) -> Result<String, JsValue> {
        let key = parse_key(key)?;
        to_json(self.host.export_snapshot(&key).map_err(to_js)?)
    }

    /// `restore(name, instance, events_jsonl, snapshot_json) -> StatusView JSON`
    pub fn restore(
        &mut self,
        name: &str,
        instance: &str,
        events_jsonl: &str,
        snapshot_json: Option<String>,
    ) -> Result<String, JsValue> {
        to_json(
            self.host
                .restore(name, instance, events_jsonl, snapshot_json.as_deref())
                .map_err(to_js)?,
        )
    }

    /// 版本探针：P3 增加实例会话与两阶段提交，ABI 递增。
    pub fn abi_version(&self) -> String {
        "pact-wasm/0.2".to_owned()
    }
}

impl Default for PactWasm {
    fn default() -> Self {
        Self::new()
    }
}

fn parse_key(text: &str) -> Result<InstanceKey, JsValue> {
    PactHost::parse_key(text).map_err(to_js)
}

fn to_json<T: Serialize>(value: T) -> Result<String, JsValue> {
    serde_json::to_string(&value)
        .map_err(|e| JsValue::from_str(&format!("{{\"code\":\"ModelJson\",\"message\":\"{e}\"}}")))
}

fn to_js(err: FacadeError) -> JsValue {
    let json = serde_json::to_string(&err).unwrap_or_else(|_| {
        "{\"code\":\"Internal\",\"message\":\"error serialization failed\"}".to_owned()
    });
    JsValue::from_str(&json)
}
