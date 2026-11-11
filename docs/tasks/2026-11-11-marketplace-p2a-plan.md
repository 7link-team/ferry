# 集群模板 P2a：蓝图核心实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 应用模板可以是集群蓝图（角色 + 实例数 + 放置约束 + 成员引用）；创建部署时为每个实例绑定 Linux 主机与通告地址，系统按驱动（native / docker / compose）编译为每台主机一份 `ferry/v2 Application`，复用现有主机组的逐主机预览、执行、状态与回滚。

**Architecture:** 新模块 `crates/server/src/blueprint/`（模型校验、受限模板引擎、绑定校验、驱动适配器编译）。模板渲染分两阶段：第一阶段沿用 `templates::render_content` 替换 `.Values`、保留实例级表达式原文；第二阶段在部署预览/执行时按实例上下文渲染服务与配置文件。部署新增 `target_kind = "blueprint"` 与 `deployments.blueprint_binding`，预览/入队/执行走与 `host-group` 相同的 `targets` 产物路径，只是每台主机的 manifest 由编译器给出。

**Tech Stack:** Rust（axum、sea-orm、serde_json）、现有部署驱动、Python 真实驱动验收（CI Linux）。

**Spec:** `docs/应用市场与集群模板设计.md` §6、§7（不含 §7.1 拓扑变更）、§7.2（Linux 分期说明）、§9.4、§15。

## Global Constraints

- 目标主机仅 Linux：`hosts.os` 非空且 ≠ `linux` → 拒绝绑定（`blueprint.hostOsUnsupported`）；`os` 为空（SSH 主机）放行。
- 本计划驱动：`native`、`docker`、`compose`。`stack` → `blueprint.driverNotYetSupported`（P2b）；`k8s` → `blueprint.driverNotImplemented`。
- `apiVersion: ferry/blueprint-v1`，`kind: Blueprint`；实例名 `<role>-<index>`，index 从 0 开始。
- 实例数：`1 ≤ min ≤ default ≤ max ≤ 100`；`odd: true` 时实例数必须为奇数；角色 ≤ 10；总实例 ≤ 100。
- 有状态角色（`identity: stable`）的 `update.maxParallel` 强制为 1（P2a 只校验，不执行滚动）。
- `hooks` 本计划必须为空或缺省（P2.5 实现），否则 `blueprint.hooksNotYetSupported`。
- 通告地址：每个绑定实例必须带 `address`（前端用主机 `address` 预填）；拒绝空、`0.0.0.0`、`::`、`127.0.0.0/8`、`::1`、`localhost`；长度 ≤ 253；IP 或主机名字符集 `[A-Za-z0-9.-:]`、IPv6 需方括号或纯 IPv6 文本。
- 模板子集：`{{ .Values.x }}`、`{{ .Instance.Name|Index|Role|Host }}`、`{{ .Instance.Port "p" }}`、`{{ range .Role "r" }}…{{ end }}`（块内 `.Name .Index .Host` 与 `.Port "p"`）、`{{ if X }}…{{ else }}…{{ end }}`、`{{ .Deployment.Driver }}`、`{{ add A B }}`（整数）、`{{ join (.Role "r") "," "<fmt>" }}` 不做；循环/条件嵌套深度 ≤ 8；不支持任意函数。
- secret 参数只能作为整串出现在 `/services/*/env/*/valueFrom`（沿用现规则）；配置模板中 secret 不得出现在 `range`/`if` 块内。
- 部署的 `blueprint_binding` 在首次修订后不可修改（拓扑变更走 P2.5）：`blueprint.topologyChangeRequiresPlan`。
- 所有新文案六语（zh-CN/zh-TW/en-US/ms-MY/ja-JP/ko-KR）。
- 编译测试用 worktree 自己的 `CARGO_TARGET_DIR=D:/Projects/github/ferry-marketplace-p1/target`（不得与主仓库共用 target）。

## Review Focus

1. 同一主机上两个实例发布同一端口（`colocateWith` 或主机重复）→ 编译拒绝并指出实例名 → Task 5 `colocated_instances_with_same_published_port_rejected`。
2. 非蓝图部署引用了含 `.Instance`/`range` 的配置模板 → 绑定阶段明确报错而不是把模板文本原样写入主机 → Task 6 `instance_tokens_outside_blueprint_rejected`。
3. 预览后修改了蓝图模板或绑定再执行 → 现有预览令牌/绑定比较拒绝（error.040/042）→ Task 7 `blueprint_apply_rejects_stale_preview`。
4. 绑定的主机被删除或换了工作空间 → 预览报错且不入队 → Task 7 `blueprint_preview_rejects_missing_host`。
5. 回滚到旧修订 → 按旧修订存储的逐主机产物重放，不重新编译 → Task 7 `blueprint_rollback_replays_stored_targets`。

---

### Task 1: 蓝图文档模型与校验

**Files:**
- Create: `crates/server/src/blueprint/mod.rs`（`pub mod model; pub mod template; pub mod binding; pub mod compile;`）
- Create: `crates/server/src/blueprint/model.rs`
- Modify: `crates/server/src/main.rs`（`mod blueprint;`）、`crates/server/src/i18n.rs`（include 新文件）、后端六语 `deployment-*.json`（新 key 前缀 `blueprint.`）

**Interfaces:**
```rust
pub struct Blueprint { pub metadata: Metadata, pub roles: Vec<Role>, pub endpoints: Vec<Endpoint>,
    pub drivers: BTreeMap<String, DriverSupport>, pub update: Update, pub strategy: Option<Value>, pub allow_single_host: bool }
pub struct Role { pub name: String, pub instances: Instances, pub placement: Placement, pub identity: Identity,
    pub ports: Vec<RolePort>, pub service: serde_json::Value /* ferry/v2 Service 字段模板，不含 name/ports */ }
pub struct Instances { pub min: u32, pub max: u32, pub default: u32, pub odd: bool }
pub struct Placement { pub distinct_hosts: bool, pub colocate_with: Option<String> }
pub enum Identity { Stable, Stateless }
pub struct RolePort { pub name: String, pub container: u16, pub published: serde_json::Value /* 整数或模板串 */, pub protocol: Option<String> }
pub struct Endpoint { pub name: String, pub role: String, pub port: String, pub kind: String /* discovery|vip|direct */ }
pub struct DriverSupport { pub supported: bool }
pub struct Update { pub max_parallel: u32, pub min_healthy_seconds: u32, pub health_timeout: u32, pub auto_revert: bool }
pub fn parse(text: &str) -> Result<Blueprint>;           // 结构 + 规则校验；文本仍可含实例级表达式
pub fn is_blueprint(text: &str) -> bool;                  // kind == "Blueprint"
```
JSON 键为 camelCase（`distinctHosts`、`colocateWith`、`allowSingleHost`、`maxParallel`…），`deny_unknown_fields`；`hooks` 键允许但必须为空数组。

- [ ] **Step 1: 失败测试**（`model.rs` tests）：`nats_blueprint_parses`（3 角色字段齐全）、`instance_bounds_enforced`（min>default、max>100、odd 且 default 偶数）、`colocate_target_must_exist_and_differ`、`duplicate_role_or_port_names_rejected`、`hooks_not_yet_supported`、`stable_role_forces_max_parallel_one`（maxParallel 2 + stable → Err）、`endpoint_must_reference_role_port`、`unknown_driver_key_rejected`（只允许 native/docker/compose/stack/k8s）。
- [ ] **Step 2:** `cargo test -p ferry-server blueprint::model` → FAIL（模块不存在）。
- [ ] **Step 3:** 实现。
- [ ] **Step 4:** 同命令 PASS。
- [ ] **Step 5:** Commit `feat(blueprint): document model and validation`

### Task 2: 受限模板引擎

**Files:**
- Create: `crates/server/src/blueprint/template.rs`

**Interfaces:**
```rust
pub enum Phase { Values, Instance }
pub struct Context { pub values: BTreeMap<String, Value>, pub instance: Option<Instance>, pub members: BTreeMap<String, Vec<Member>>, pub driver: String }
pub struct Instance { pub role: String, pub index: u32, pub name: String, pub host: String, pub ports: BTreeMap<String, u16> }
pub struct Member { pub name: String, pub index: u32, pub host: String, pub ports: BTreeMap<String, u16> }
pub fn check(text: &str) -> Result<()>;                         // 语法：配对、嵌套 ≤ 8、表达式白名单
pub fn render_text(text: &str, ctx: &Context, phase: Phase) -> Result<String>;
/// 整串是单个表达式时返回类型值（端口为整数），否则返回字符串。
pub fn render_value(text: &str, ctx: &Context, phase: Phase) -> Result<Value>;
pub fn has_instance_tokens(text: &str) -> bool;
```
`Phase::Values`：只替换 `.Values.*`，其余表达式（含块标签）原样输出；`Phase::Instance`：全部求值，残留任何 `{{` → `blueprint.templateUnresolved`。`if` 的真值：非空字符串、非 0、`true`、非空列表。`add` 仅整数。

- [ ] **Step 1: 失败测试**：`values_phase_keeps_instance_tokens`、`nats_routes_render_with_range`（3 成员 → `nats://10.0.0.1:6222,…`，用 `{{ range .Role "nats" }}{{ if .Index }},{{ end }}nats://{{ .Host }}:{{ .Port "cluster" }}{{ end }}`）、`if_else_by_index`（index 0 输出 master 否则 replica）、`whole_port_expression_is_typed`（`"{{ .Instance.Port \"data\" }}"` → `6379` 整数）、`add_offsets_port`、`unbalanced_or_deep_blocks_rejected`、`unknown_function_rejected`、`unresolved_after_instance_phase_rejected`、`unknown_role_or_port_rejected`。
- [ ] **Step 2:** `cargo test -p ferry-server blueprint::template` → FAIL。
- [ ] **Step 3:** 实现（手写词法 + 递归下降，不引入新依赖）。
- [ ] **Step 4:** PASS。
- [ ] **Step 5:** Commit `feat(blueprint): restricted template engine`

### Task 3: 应用/配置模板接受蓝图，市场放开蓝图包

**Files:**
- Modify: `crates/server/src/templates.rs`（`expand` 字符串分支 517–656；`validate_template` 668–704；`render_content` 705–786）
- Modify: `crates/server/src/marketplace/index.rs`（`compatibility` 去掉 `blueprintRequiresUpgrade`；蓝图版本走 `validate_app`）

**Interfaces / 规则：**
- `expand` 遇到 `{{ … }}` 内容以 `.Instance`、`.Role`、`.Deployment`、`range `、`if `、`else`、`end`、`add ` 开头时：`Kind::App`（文档为 Blueprint）与 `Kind::Config` 原样保留；其他 Kind 维持 `templates.expressionUnsupported`。
- `Kind::App`：文档 `kind == "Blueprint"` 时，`render_content` 输出经 `blueprint::model::parse` 校验，不调用 `deployments::manifest::parse`；保留实例级表达式。
- `Kind::Config`：secret 占位位于 `range`/`if` 块内 → `templates.secretInBlock`。

- [ ] **Step 1: 失败测试**：`app_template_accepts_blueprint_with_instance_tokens`、`config_template_keeps_instance_tokens`、`secret_inside_config_block_rejected`、`pipeline_template_still_rejects_instance_tokens`、`marketplace_blueprint_version_is_compatible`（index.rs）。
- [ ] **Step 2:** FAIL。
- [ ] **Step 3:** 实现。
- [ ] **Step 4:** `cargo test -p ferry-server -- templates marketplace blueprint` PASS。
- [ ] **Step 5:** Commit `feat(templates): blueprint app templates and instance-aware config templates`

### Task 4: 绑定模型与校验

**Files:**
- Create: `crates/server/src/blueprint/binding.rs`

**Interfaces:**
```rust
pub struct Binding { pub driver: String, pub roles: BTreeMap<String, Vec<InstanceBinding>> }
pub struct InstanceBinding { pub host_id: Id, pub address: String }
pub struct HostInfo { pub id: Id, pub os: String }
/// 不访问数据库：调用方先查出本空间主机。
pub fn validate(bp: &Blueprint, b: &Binding, hosts: &BTreeMap<Id, HostInfo>) -> Result<()>;
pub fn parse(text: &str) -> Result<Binding>;
```
规则：driver 在 `bp.drivers` 中 `supported` 且属于本计划驱动；每角色实例数在范围内且满足 odd；主机必须在 `hosts` 中且 Linux 规则；`distinctHosts` 同角色主机不重复（compose 驱动仅当 `allowSingleHost`）；`colocateWith` 第 i 个实例主机等于目标角色第 i 个实例主机（实例数必须相等）；地址规则见全局约束；compose 驱动所有实例必须同一主机。

- [ ] **Step 1: 失败测试**：`valid_three_host_binding`、`count_and_odd_enforced`、`distinct_hosts_enforced`、`colocate_requires_same_host_and_count`、`non_linux_host_rejected_empty_os_allowed`、`loopback_and_empty_addresses_rejected`、`compose_requires_single_host_and_allow_single_host`、`stack_and_k8s_not_available_yet`。
- [ ] **Step 2:** FAIL。 **Step 3:** 实现。 **Step 4:** PASS。
- [ ] **Step 5:** Commit `feat(blueprint): instance binding validation`

### Task 5: 驱动适配器与按主机编译

**Files:**
- Create: `crates/server/src/blueprint/compile.rs`

**Interfaces:**
```rust
pub struct HostPlan { pub host_id: Id, pub manifest: String /* ferry/v2 Application JSON */, pub contexts: BTreeMap<String, template::Context> /* 服务名 → 实例上下文 */ }
pub trait DriverAdapter { fn id(&self) -> &'static str; fn resolve(&self, bp: &Blueprint, b: &Binding, values: &BTreeMap<String,Value>) -> Result<Topology>; }
pub struct Topology { pub members: BTreeMap<String, Vec<Member>>, pub placement: Vec<(Id, String /*service*/, template::Instance)> }
pub fn compile(bp_text: &str, binding: &Binding, app_name: &str) -> Result<Vec<HostPlan>>;
```
- native / docker：`.Host` = 绑定 address；`.Port "p"` = 渲染后的 `published`（无则 container）。
- compose：单主机；`.Host` = 服务名 `<role>-<i>`；`.Port` = container。
- 每主机 Application：`apiVersion ferry/v2`、`metadata.name = app_name`、`strategy` 取蓝图、`services` = 该主机所有实例（name = 实例名，role.service 字段按实例渲染，`ports` 由 `RolePort` 生成）；同主机 `(published, protocol)` 重复 → `blueprint.portConflict`（参数含实例名）。输出前用 `deployments::manifest::parse_for(_, driver)` 校验。
- `.Values` 已在第一阶段替换；compile 只做 `Phase::Instance`。

- [ ] **Step 1: 失败测试**：`native_three_hosts_one_application_each`（NATS 3 实例 → 3 个 HostPlan，routes 含 3 个地址）、`compose_single_host_uses_service_names`、`colocated_sentinel_and_redis_share_host_application`、`colocated_instances_with_same_published_port_rejected`、`compiled_manifest_passes_driver_validation`、`stack_rejected_until_p2b`。
- [ ] **Step 2:** FAIL。 **Step 3:** 实现。 **Step 4:** PASS。
- [ ] **Step 5:** Commit `feat(blueprint): driver adapters compile per-host applications`

### Task 6: 配置文件按实例渲染

**Files:**
- Modify: `crates/server/src/deployments/mod.rs`（`bindings` 583–670 增加参数；`config_files` 677–716 不变）

**Interfaces:**
```rust
async fn bindings(conn, r: &apps::Model, manifest: &str, contexts: Option<&BTreeMap<String, blueprint::template::Context>>)
    -> Result<(Value, BTreeMap<String, variables::Model>, ConfigContents)>;
```
渲染顺序：`templates::config_content`（`.Values` + secret 标记）→ 若该文件所属服务有上下文：`blueprint::template::render_text(_, ctx, Phase::Instance)`；若无上下文且 `has_instance_tokens` → `deployment.blueprintContextRequired`。现有调用点传 `None`。

- [ ] **Step 1: 失败测试**（`deployments/mod.rs` 新 tests 模块 `blueprint_binding_tests`，夹具 `agents::tests::setup`）：`config_rendered_per_instance_context`、`instance_tokens_outside_blueprint_rejected`。
- [ ] **Step 2:** FAIL。 **Step 3:** 实现。 **Step 4:** `cargo test -p ferry-server deployments` PASS。
- [ ] **Step 5:** Commit `feat(deployments): render config templates with blueprint instance context`

### Task 7: 部署接入 `target_kind = "blueprint"`

**Files:**
- Create: `crates/server/src/migration/blueprint_deployments.rs`（`deployments.blueprint_binding` TEXT NULL；MySQL longtext；down 删除列）并注册在迁移列表末尾
- Modify: `crates/server/src/entities.rs`（`deployments.blueprint_binding: Option<String>`）
- Modify: `crates/server/src/deployments/mod.rs`（`Body` 加 `blueprint_binding: Option<Value>`；`check` 120–196；`create_in` 352–399；`update` 400–458；`preview` 969–1015；`trigger` 1211–1399；`execute_inner` 1602–1616）
- Modify: `crates/server/src/deployments/groups.rs`（`preview` 73–155 与 `enqueue` 156–229 改为接收逐主机计划）

**Interfaces:**
```rust
// groups.rs
pub(super) struct HostTarget { pub host_id: Id, pub app: apps::Model /* manifest=该主机 manifest, target_kind="host", target_ref=host */, pub contexts: Option<BTreeMap<String, Context>> }
pub(super) async fn targets(conn, scope, app: &apps::Model) -> Result<Vec<HostTarget>>; // host-group: 成员×同一 manifest；blueprint: compile()
```
- `check`：`target_kind ∈ {host, host-group, blueprint}`；blueprint 时 `target_ref` 必须为 `"0"`、`mode` 为驱动、`manifest` 是蓝图、`blueprint_binding` 经 `binding::validate`（主机从本空间查询）。
- `update`：`revision_number > 0` 时 `blueprint_binding` 变化 → `blueprint.topologyChangeRequiresPlan`。
- `preview`：blueprint 与 host-group 同走 `groups::preview`，按 `targets()` 逐主机 `bindings(.., Some(contexts))` + `prepare_artifact`；`x-variable-bindings` 为所有主机元数据的并集（按 `kind+service+path` 去重）。
- `trigger`：apply 时 `files` = 所有主机 `environment_files + config_files` 并集；绑定比较同样用并集；入队走 `groups::enqueue`（成员一致性检查改为比较 `targets()` 的主机集合）。
- `execute_inner`：`target_kind ∈ {host-group, blueprint}` → `groups::execute`。
- 回滚/重启：沿用旧修订存储的 `targets`，不重新编译。

- [ ] **Step 1: 失败测试**（`deployments` 新测试模块，用 `remote` 测试替身或现有 `HostChannel` 测试实现；若现有无替身，在本任务新增 `#[cfg(test)]` 的 `FakeChannel` 返回固定 `docker` 输出）：`create_blueprint_deployment_validates_binding`、`blueprint_preview_produces_one_target_per_host`、`blueprint_apply_rejects_stale_preview`、`blueprint_preview_rejects_missing_host`、`binding_frozen_after_first_revision`、`blueprint_rollback_replays_stored_targets`、`host_group_behaviour_unchanged`（现有 host-group 测试全绿）。
- [ ] **Step 2:** FAIL。 **Step 3:** 实现（含迁移 roundtrip 测试 `blueprint_binding_migration_roundtrip`）。 **Step 4:** `cargo test -p ferry-server` 全量 PASS。
- [ ] **Step 5:** Commit `feat(deployments): blueprint deployments reuse per-host targets`

### Task 8: 官方蓝图模板与真实驱动验收

**Files:**
- Create: `marketplace/apps/nats-cluster/{package.json,1.0.0/blueprint.json,1.0.0/parameters.json,1.0.0/files/nats.conf,1.0.0/README.md}`（3 实例 JetStream，`distinctHosts`，客户端/集群端口经 `add` 按序号偏移以便单机 CI 模拟多主机）
- Create: `marketplace/apps/nginx-ha/…`（stateless 2 实例 + upstream 配置文件 `range .Role "web"`）
- Modify: `scripts/test-deployments.py`（新增 `FERRY_TEST_BLUEPRINT=1`：在同一 Linux 机上登记 3 个 runner 主机记录，地址用同机私网 IP，分别以 native 与 docker 驱动部署 nginx-ha 与 nats-cluster：HTTP 可达、NATS 3 路由全连通、JetStream 副本 3、一次升级参数、一次回滚）
- Modify: `.github/workflows/deployment-ha.yml`（矩阵新增 blueprint 项）
- Modify: `scripts/build-marketplace-index.mjs` 输出更新；`marketplace/index.json`

- [ ] **Step 1:** `node scripts/build-marketplace-index.mjs`；`cargo test -p ferry-server marketplace::index::tests::repository_bundle_parses` PASS（蓝图包 compatible）。
- [ ] **Step 2:** 本地 `cargo test -p ferry-server blueprint` 加编译快照测试 `official_blueprints_compile_for_supported_drivers`。
- [ ] **Step 3:** 推送分支后由 CI 跑 `deployment-ha.yml` blueprint 项（需要 Linux/Docker，本地 Windows 不跑）；结果与 run ID 记入 `data/maintenance/marketplace-p2a/`。CI 未通过前不标记模板 `verified`。
- [ ] **Step 4:** `npm test`、`cargo test -p ferry-server` 全量；ADR 追加决策；MEMORY 更新。
- [ ] **Step 5:** Commit `feat(marketplace): official NATS and nginx blueprints with driver acceptance`

## 不在本计划

- P2b：Swarm/stack 适配（`docker node ls` 节点映射、`ports.mode: host`、`endpoint_mode: dnsrr`）。
- P2c：实例绑定界面（前端），在此之前通过 API 与验收脚本创建蓝图部署。
- P2.5：拓扑变更计划、滚动、hooks。
- Windows/macOS 部署驱动（单独立项）。
