# 集群编排 P3：协调器 + 无状态/复制型自愈 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 蓝图部署（native/docker）的主机失联后，Ferry 经二次确认与隔离，在其他候选主机上以新修订重建无状态实例（A 类）或补齐复制型成员（B 类，含 leave/join/healthy 钩子），全程写入可分页的自动动作时间线；其他驱动/类型只告警；用户可手动替换实例、确认需人工处理的实例。

**Architecture:** 纯函数层（`blueprint/failover.rs`：类型判定、动作选择、主机失联判决；`placement::replace`：单实例重新放置）与 I/O 层（`deployments/{events,replace,hooks,sequence,reconciler}.rs`）分离。协调器是带 DB 租约（`locks` 表）的后台循环，每 15s 观察绑定主机的心跳/探测，确认失联后写隔离行、调用 `replace_instance` 改绑定并经现有预览 + 执行器生成 `auto-replace` 修订；B 类按「仲裁检查 → leave → 替换修订 → join → healthy」逐个成员进行。冷却、连续失败与人工确认状态全部由 `deployment_events` 推导，不另存状态。

**Tech Stack:** Rust（axum、sea-orm、tokio、async-trait）、React + 现有 ResourceForm/Dialog、vitest、Playwright、Python CI 脚本（`scripts/test-deployments.py`）。

**Spec:** `docs/集群编排与故障自愈设计.md` §2、§3.3（`failover.mode`）、§4（模板钩子）、§6（协调器、类型、隔离、动作安全）、§8（P3 行）；`ADR.md`「集群编排 P2c」。

## Global Constraints

- **驱动范围**：只有 Ferry 自己调度的 `native`/`docker` 会被自动或手动替换。`stack` 无状态角色交给 Swarm（Ferry 不动作）；`stack` 有状态（stable）实例与 `compose`（单机）只告警（`alertOnly`）。Swarm 重新固定节点推迟，写入 ADR。
- **规格字段**：`ferry/deploy-v1` 新增可选 `failover: {mode: off|alert|auto, hostDownAfter: <秒>}`；`mode` 缺省 `auto`，`hostDownAfter` 缺省 90，范围 30–3600。保持 `deny_unknown_fields`。部署后可改 `failover`：它不是拓扑，服务端 `same_topology` 与前端 `topologyKey` 都忽略它。
- **模板字段**：蓝图角色新增可选 `failover: {replace: supported|unsupported}`（缺省 unsupported）与 `hooks: {join, leave, healthy}`；每个钩子是字符串 argv 数组，1–32 项、每项 1–1024 字符、合计 ≤ 4096 字节，每项通过 `template::check`，运行时按实例上下文（`.Instance/.Role`）渲染。顶层 `hooks` 仍须为空；`backup`/`restore`/`promote` 属 P4/P5。
- **类型判定**（唯一纯函数 `failover::class_of`）：`identity: stateless` → A；`identity: stable` 且 `failover.replace: supported` → B；其他 → `manual`（`mode: auto` 时按告警处理，界面说明需要 P4 备份）。
- **失联判定**：①主机陈旧超过 `hostDownAfter`（`hosts.seen_at`：runner 由心跳写，SSH 由主机监控成功探测写，见 `hosts/monitor.rs`）；②服务端新建连接执行 `echo ferry-ok` 必须失败；③SSH 主机且同一部署有健康同伴：经同伴通道对主机 `address:port` 做 TCP 检查，必须失败。runner 主机无入站端口，跳过③，改为陈旧超过 2×`hostDownAfter`，记为单源确认。同伴能连上 → 否决（`peerVeto` 事件）。
- **隔离**：替换前若旧主机可达先停实例；无论是否停掉都写 `deployment_fences` 行（停掉则同时写 `cleared_at`）。主机恢复在线后，协调器先停掉其上被隔离的实例，再清除隔离。未清除隔离的主机不作为该部署的候选。
- **替换 = 新修订**：`replace_instance` 按角色选择器、`distinctHosts`、`colocateWith`、同一排序键选新主机，排除失联主机与隔离主机，只改该实例（及与之同序号共置的实例）的绑定项；其他成员清单随地址重新渲染。修订 `action` 为 `auto-replace` 或 `manual-replace`（`POST /deployments/{id}/instances/{role}/{index}/replace`，权限 `deployments.apply`——本仓库没有 `deployments.execute`，执行权限即 `deployments.apply`）。执行沿用现有执行器。P2c 的绑定冻结对用户编辑仍然有效，替换是唯一被允许的移动方式。
- **B 类顺序**（一次一个成员）：先检查仲裁：不计失联成员的健康成员数 ≥ ⌊n/2⌋+1 才继续，否则转人工（全停规则）；随后 `leave`（若定义，在一个健康同伴上执行）→ 替换修订启动新实例 → `join`（若定义，在新实例内执行）→ 等 `healthy` 通过（若无钩子则用监控的实例健康），超时取模板 `update.healthTimeout`。钩子在实例容器内（`docker exec`）或 native 实例环境中经主机通道运行，输出进入事件日志，按现有执行器规则脱敏（`execution_output` 的 protected 规则）。
- **协调器**：后台任务，DB 租约（单实例执行），每 15s 一轮；同一部署同时至多一个自动动作；同一实例两次自动替换间隔 ≥ 10 分钟；连续失败 3 次标记 `needs-attention`，在用户确认（ack API）前不再动作。只对有当前修订、`target_kind=blueprint` 的部署动作；`mode: alert` 记录事件并经现有告警机制告警。
- **事件**：新表 `deployment_events`（id, workspace_id, deployment_id, kind, instance, host_id, detail（JSON，TEXT）, revision_id, created_at）；kind ∈ hostSuspected、hostConfirmedDown、peerVeto、fenced、replaceStarted、replaceSucceeded、replaceFailed、hookOutput、needsAttention、unfenced、alertOnly（以及本计划新增的 acknowledged，见 Task 3）。`GET /deployments/{id}/events` 分页、按工作空间隔离。所有面向用户的文案为带参数的 i18n 键，服务端六语（`crates/server/src/i18n/deployment-*.json`）。
- **迁移**：MySQL 下 TEXT 列不能有默认值、不能建索引（`long_text(..)` 且无 `.default`，索引只建在 BIGINT/VARCHAR 列）；`down` 有数据时拒绝删除（参照 `migration/monitoring.rs`）。
- **页面**：遵守 AGENTS.md 页面门槛：Desktop ≥1200 / Pad 768–1199 / Phone 320–767 三类布局，禁止整页横向滚动，全状态（默认/空/加载/错误/极端值），前端文案六语（zh-CN/zh-TW/en-US/ms-MY/ja-JP/ko-KR）。
- **验收只在 CI**：GitHub Actions `deployment-ha.yml` 的 blueprint docker 矩阵项；推送前由控制者向用户确认。失败用 `report_log_tail` 注解。
- **编译与检查**：`CARGO_TARGET_DIR=D:/Projects/github/ferry-marketplace-p1/target`（不得落在 C:）；`cargo fmt --all -- --check`；全量 `cargo test -p ferry-server`；`npx vitest run`；`npx tsc -b`；`npm run build`；改路由时 `python scripts/generate-platform-tools.py` 后 `--check`。

## Review Focus

1. 网络抖动/误判：主机 `seen_at` 陈旧且服务端探测失败，但同伴能连上 → 只记 `peerVeto`，不隔离、不替换；同一轮抖动只记一次 → Task 2 `peer_reachable_vetoes`；Task 6 `peer_veto_records_event_and_does_not_replace`。
2. 被隔离的主机恢复上线：先停其上被隔离的实例，停成功才清除隔离并记 `unfenced`；停失败保留隔离、下一轮重试；隔离期间该主机不作为候选 → Task 4 `fenced_host_excluded_from_candidates`；Task 6 `returning_host_stops_fenced_instances_before_clearing`。
3. 3 成员 B 类同时失去 2 个：不替换任何成员，两实例记 `needsAttention{reason: quorumLost}` → Task 5 `quorum_loss_goes_manual`。
4. 冷却与连续失败：10 分钟内不重复自动替换同一实例；第 3 次连续失败记 `needsAttention`，之后不再动作，ack 后恢复 → Task 3 `summarize_cooldown_failures_and_ack`；Task 6 `cooldown_and_three_failures_need_attention`。
5. 没有可用候选主机：记 `replaceFailed{reason: noCandidate}`，不写隔离、不 panic、不在冷却期内重试 → Task 4 `replace_without_candidate_is_an_error_not_a_panic`；Task 6 `no_candidate_records_event_and_respects_cooldown`。

---

### Task 1: 规格与模板模型

**Files:**
- Modify: `crates/server/src/blueprint/spec.rs`（`DeploySpec.failover`）、`crates/server/src/blueprint/model.rs`（`Role.failover`/`Role.hooks` 与校验）、`crates/server/src/deployments/specs.rs`（`same_topology` 注释 + 测试）、`crates/server/src/i18n/deployment-*.json`（六语）
- Test: `crates/server/src/blueprint/spec.rs`、`crates/server/src/blueprint/model.rs` 内测试模块；`crates/server/src/deployments/blueprint_tests.rs`

**Interfaces:**
- Produces:
```rust
// blueprint/spec.rs
#[serde(rename_all = "lowercase")] pub enum FailoverMode { Off, Alert, #[default] Auto }
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FailoverSpec { #[serde(default)] pub mode: FailoverMode, #[serde(default = "host_down_after")] pub host_down_after: u32 }
pub const HOST_DOWN_AFTER_DEFAULT: u32 = 90;      // 30..=3600
// DeploySpec 新增：#[serde(default, skip_serializing_if = "Option::is_none")] pub failover: Option<FailoverSpec>
// blueprint/model.rs
#[serde(rename_all = "lowercase")] pub enum Replace { Supported, #[default] Unsupported }
#[serde(deny_unknown_fields)] pub struct RoleFailover { #[serde(default)] pub replace: Replace }
#[serde(deny_unknown_fields)] pub struct RoleHooks { pub join: Option<Vec<String>>, pub leave: Option<Vec<String>>, pub healthy: Option<Vec<String>> }
// Role 新增：#[serde(default)] pub failover: RoleFailover, #[serde(default)] pub hooks: RoleHooks（均 skip_serializing 缺省值）
```
- 新错误键：`blueprint.specHostDownAfterRange`（min、max）、`blueprint.hookInvalid`（role、hook）。编译器（`compile.rs`）不读这两个字段，产物不变。

- [ ] Step 1 失败测试（spec.rs）：`failover_parses_with_defaults`（`failover: {}` → mode Auto、90；缺省整个键 → `None`）、`failover_mode_and_range_checked`（`mode: sometimes` → `blueprint.specInvalid`；`hostDownAfter: 29`/`3601` → `blueprint.specHostDownAfterRange`；30 与 3600 通过）、`failover_rejects_unknown_keys`（`failover: {maxConcurrent: 1}` 被拒）、`roundtrip_to_yaml` 增加 failover 断言；把 `unknown_top_level_key_rejected` 的 `failover` 换成另一个未知键（`backup`/`storage` 保留）。
- [ ] Step 2 失败测试（model.rs）：`role_failover_and_hooks_parse`（缺省 Unsupported、hooks 全 None）、`hooks_must_be_bounded_argv`（空数组、33 项、1025 字符项、合计 > 4096、非字符串项、`{{ .Bogus }}` 模板 → `blueprint.hookInvalid` 且 params 含 role 与 hook 名）、`role_hooks_do_not_change_compiled_manifest`（同一蓝图加钩子前后 `compile_unresolved` 产物相等）。
- [ ] Step 3 失败测试（blueprint_tests.rs）：`failover_change_after_deploy_is_accepted`（部署成功后把规格 `failover.mode` 从 auto 改为 alert：更新成功、绑定不变、存储的 `deploy_spec` 含 `mode: alert`）。
- [ ] Step 4 运行 `cargo test -p ferry-server -- blueprint::spec blueprint::model failover_change_after_deploy` → FAIL。
- [ ] Step 5 实现上述类型与 `validate`；在 `specs.rs::same_topology` 文档注释写明 `failover` 不属于拓扑；六语补键。
- [ ] Step 6 同一命令 PASS；`cargo fmt --all -- --check`。
- [ ] Step 7 Commit `feat(blueprint): failover spec field and role failover/hooks model`

### Task 2: 类型判定与主机失联判决（纯函数）

**Files:**
- Create: `crates/server/src/blueprint/failover.rs`（`blueprint/mod.rs` 声明）

**Interfaces:**
- Consumes: Task 1 的 `FailoverMode`、`FailoverSpec`、`Role`、`Replace`、`Identity`。
- Produces:
```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)] #[serde(rename_all = "lowercase")]
pub enum FailoverClass { Stateless, Replicated, Manual }          // A / B / manual
pub fn class_of(role: &Role) -> FailoverClass;
#[derive(Debug, PartialEq, Eq)] pub enum AlertReason { ModeAlert, NeedsBackup, DriverUnsupported }
#[derive(Debug, PartialEq, Eq)] pub enum Response { Ignore, AlertOnly(AlertReason), Replace(FailoverClass) }
pub fn respond(mode: FailoverMode, driver: &str, role: &Role) -> Response;
/// 规格中的设置；没有规格（P2a 旧蓝图部署）→ Alert + 90。
pub fn settings(spec: Option<&DeploySpec>) -> FailoverSpec;
pub struct HostFacts { pub runner: bool, pub seen_at: i64 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum PeerCheck { Reachable(Id), Unreachable(Id), Inconclusive, NoPeer }
pub struct Evidence { pub server_ok: bool, pub peer: Option<PeerCheck> }
#[derive(Debug, PartialEq, Eq)] pub enum Verdict { Healthy, NeedsProbe, Suspected, Down { single_source: bool }, Vetoed(Id) }
pub fn is_stale(facts: &HostFacts, now_ms: i64, after_secs: u32) -> bool;   // now - seen_at > after*1000
pub fn judge(facts: &HostFacts, now_ms: i64, after_secs: u32, evidence: Option<&Evidence>) -> Verdict;
```
- `respond` 规则：`Off` → Ignore；`stack` 无状态 → Ignore（任何 mode）；`stack` 有状态、`compose` → `AlertOnly(DriverUnsupported)`（mode≠off）；`Alert` → `AlertOnly(ModeAlert)`；`Auto` + native/docker：A/B → `Replace(class)`，manual → `AlertOnly(NeedsBackup)`。
- `judge` 规则（按顺序）：不陈旧 → Healthy；`evidence` 为 None → NeedsProbe；`server_ok` → Healthy；runner：陈旧 > 2×after → `Down{single_source:true}` 否则 Suspected；SSH：`peer` 为 None → Suspected（等同伴检查）、`Reachable(p)` → `Vetoed(p)`、`Unreachable` → `Down{single_source:false}`、`NoPeer`/`Inconclusive` → 同 runner 的单源规则。

- [ ] Step 1 失败测试：`class_of_derives_a_b_manual`（stateless → Stateless；stable+supported → Replicated；stable+unsupported → Manual；stateless+supported 仍 Stateless）、`respond_by_mode_and_driver`（表驱动覆盖上面每条规则，含 `stack` 无状态在 Alert 下仍 Ignore、manual 在 Auto 下为 `AlertOnly(NeedsBackup)`）、`settings_default_without_spec_is_alert`、`fresh_host_is_healthy_without_probe`（seen_at 在 after 内 → Healthy，evidence 为 None 也不要求探测）、`server_probe_success_is_healthy`、`peer_reachable_vetoes`（`Reachable(7)` → `Vetoed(7)`）、`peer_unreachable_confirms_ssh`、`runner_needs_twice_the_window`（陈旧 1.5×after → Suspected；2×after+1ms → `Down{single_source:true}`）、`ssh_without_peer_uses_single_source_rule`（NoPeer 与 Inconclusive 同 runner）。
- [ ] Step 2 `cargo test -p ferry-server blueprint::failover` → FAIL。
- [ ] Step 3 实现。
- [ ] Step 4 同一命令 PASS。
- [ ] Step 5 Commit `feat(blueprint): failover class and host-down decision functions`

### Task 3: 事件与隔离表、事件 API

**Files:**
- Create: `crates/server/src/migration/deployment_failover.rs`（注册到 `migration.rs` 列表末尾）、`crates/server/src/deployments/events.rs`
- Modify: `crates/server/src/entities.rs`（`deployment_events`、`deployment_fences`）、`crates/server/src/deployments/mod.rs`（`mod events;` 与 handler 导出）、`crates/server/src/deployments/groups.rs::remove`（删部署时删两表行）、`crates/server/src/hosts/mod.rs`（删主机时删该主机的隔离行，`~908`）、`crates/server/src/main.rs`（路由）、`crates/server/src/i18n/deployment-*.json`、`crates/server/src/platform_tools_catalog.json`（脚本生成）

**Interfaces:**
- 表 `deployment_events`：`id` PK、`workspace_id`/`deployment_id`/`host_id`/`revision_id`/`created_at` BIGINT NOT NULL DEFAULT 0、`kind` VARCHAR(32)、`instance` VARCHAR(120) DEFAULT ''、`detail` `long_text` NOT NULL（无默认）。索引 `idx_deployment_events_deployment (workspace_id, deployment_id, created_at)`、`idx_deployment_events_instance (deployment_id, instance, created_at)`。
- 表 `deployment_fences`：`id` PK、`workspace_id`/`deployment_id`/`host_id`/`created_at`/`cleared_at` BIGINT NOT NULL DEFAULT 0、`instance` VARCHAR(120)。索引 `(workspace_id, deployment_id, cleared_at)`、`(host_id, cleared_at)`。
- `kind` 增加 `acknowledged`（用户确认需人工处理的实例；时间线可见，是 `needs-attention` 的复位点）。
- Produces（`deployments/events.rs`）：
```rust
pub(crate) enum Kind { HostSuspected, HostConfirmedDown, PeerVeto, Fenced, ReplaceStarted, ReplaceSucceeded, ReplaceFailed, HookOutput, NeedsAttention, Unfenced, AlertOnly, Acknowledged }  // as_str() = camelCase
pub(crate) struct NewEvent { pub workspace: Id, pub deployment: Id, pub kind: Kind, pub instance: String, pub host: Id, pub revision: Id, pub message: Message, pub data: Value }
/// detail = {"message": StoredText::from_message(message).stored(), ...data}
pub(crate) async fn record(s: &AppState, conn: &impl ConnectionTrait, event: NewEvent) -> Result<Id>;
pub(crate) const COOLDOWN_MS: i64 = 600_000;
pub(crate) const MAX_FAILURES: u32 = 3;
#[derive(Debug, Default, PartialEq)] pub(crate) struct InstanceHistory { pub last_started: Option<i64>, pub failures: u32, pub attention: bool, pub open: bool }
/// 纯函数：某实例事件按时间升序 → 状态。failures = 最近一次 ReplaceSucceeded/Acknowledged 之后的 ReplaceFailed 数；
/// attention = 最近一次 NeedsAttention 晚于最近一次 Acknowledged；open = 最近一次 ReplaceStarted 之后没有 Succeeded/Failed。
pub(crate) fn summarize(events: &[crate::entities::deployment_events::Model]) -> InstanceHistory;
pub(crate) fn in_cooldown(h: &InstanceHistory, now_ms: i64) -> bool;
pub(crate) async fn history(conn: &impl ConnectionTrait, workspace: Id, deployment: Id, instance: &str) -> Result<InstanceHistory>;
pub async fn list(State<AppState>, Scope, Path<(String, String)>, Query<HashMap<String,String>>) -> Result<Json<Value>>;
```
- `GET /api/workspaces/{ws}/deployments/{id}/events`：`deployments.read`；`resource::list` + `EVENT_SPEC{filters: ["deployment_id","kind","instance"], sortable: ["created_at"], default_sort: "-created_at"}`；项 `{id, kind, instance, host_id, host_name, revision_id, created_at, message /*当前语言渲染*/, detail /*去掉 message 的其余数据*/}`；响应 `{items,total,page,page_size}`；`host_name` 用 `resources::names::attach`（Kind::Host）。
- 消息键 `deployment.event.<kind>`（六语），参数：hostSuspected{host}、hostConfirmedDown{host,peer} / `deployment.event.hostConfirmedDownSingleSource`{host,window}、peerVeto{host,peer}、fenced{host,instance}、replaceStarted{instance,from,trigger}、replaceSucceeded{instance,to,revision}、replaceFailed{instance,reason}、hookOutput{hook,instance,code}、needsAttention{instance,reason}、unfenced{host,instance}、alertOnly{host,instance,reason}、acknowledged{instance,user}；原因键 `deployment.failoverReason.<reason>`（noCandidate、quorumLost、hookFailed、revisionFailed、unhealthy、interrupted、actorRevoked、driverUnsupported、needsBackup、modeAlert、tooManyFailures）。

- [ ] Step 1 失败测试（migration）：`failover_tables_migrate_and_refuse_destructive_down`（sqlite 内存库 up 两次幂等；插一行事件后 down 返回 Err；空表 down 成功）。
- [ ] Step 2 失败测试（events.rs）：`summarize_cooldown_failures_and_ack`——序列 Started(t0)/Failed/Started/Failed/Started/Failed/NeedsAttention → `failures==3`、`attention`、`in_cooldown(now=t_last+9min)` 为真、`+11min` 为假；追加 Acknowledged → `failures==0`、`!attention`；追加 Started → `open`；Succeeded 后 `!open`、failures 归零。
- [ ] Step 3 失败测试（`deployments/failover_tests.rs`，新文件，`#[cfg(test)] mod failover_tests;`；共用的 `blueprint_tests` 夹具改为 `pub(super)`）：`events_api_paginates_and_is_workspace_scoped`（25 条事件 page_size=10 → 3 页、倒序；另一工作空间同 ID 请求 404；无 `deployments.read` 403；`message` 按 `Accept-Language: ja-JP` 渲染为日文）、`deleting_deployment_removes_events_and_fences`、`deleting_host_removes_its_fences`。
- [ ] Step 4 `cargo test -p ferry-server -- deployment_failover summarize_ failover_tests::events deleting_` → FAIL。
- [ ] Step 5 实现迁移、实体、`events.rs`、路由与删除清理；六语补键；`python scripts/generate-platform-tools.py`。
- [ ] Step 6 同一命令 PASS；`python scripts/generate-platform-tools.py --check`；全量 `cargo test -p ferry-server` PASS。
- [ ] Step 7 Commit `feat(deployments): failover events and fences tables with events API`

### Task 4: `replace_instance` 与手动替换 API

**Files:**
- Modify: `crates/server/src/blueprint/placement.rs`（`replace`）、`crates/server/src/deployments/mod.rs`（`trigger` 接受 action；`scheduled_replace`）、`crates/server/src/deployments/groups.rs`（目标顺序；替换修订跳过未变主机；成功后清理不再绑定主机的 `deployment_target_states` 行）、`crates/server/src/hosts/fake.rs`（`unreachable` 开关）、`crates/server/src/main.rs`、`crates/server/src/i18n/deployment-*.json`
- Create: `crates/server/src/deployments/replace.rs`

**Interfaces:**
- Consumes: Task 2 `class_of`/`settings`；Task 3 `events::record`、`deployment_fences` 实体。
- Produces:
```rust
// blueprint/placement.rs —— 纯函数
/// 把 role[index]（以及 colocateWith 链上同序号的实例；若 role 自身 colocateWith 某角色，则从链根开始）移到新主机。
/// 候选 = eligible ∧ 匹配该角色选择器 ∧ 不在 excluded ∧（distinctHosts 时）不承载该角色其他实例；
/// 排序键同 place：违反 spread 少 → 本部署已有实例少（不计被移动实例）→ 主机 ID。只改被移动实例的 InstanceBinding。
pub fn replace(bp: &Blueprint, spec: Option<&DeploySpec>, binding: &Binding, role: &str, index: usize,
               candidates: &[Candidate], excluded: &BTreeSet<Id>) -> Result<Binding>;   // 无候选 → blueprint.replaceNoCandidate{role,instance,selector}
// deployments/replace.rs
pub(crate) enum ReplaceTrigger { Auto { failed_host: Id }, Manual }
impl ReplaceTrigger { pub fn action(&self) -> &'static str }   // "auto-replace" | "manual-replace"
/// 校验（blueprint、driver ∈ {native,docker}、class ≠ Manual、有当前修订、状态不在 queued/running/needs-review）→
/// placement::replace（排除 failed_host 与未清除隔离的主机）→ 旧主机隔离（见下）→ 事务内按 version 写 blueprint_binding
/// 与（若规格有该角色的人工 placement）镜像写 deploy_spec.placement，version+1 → scheduled_replace。返回修订 ID。
pub(crate) async fn replace_instance(s: &AppState, scope: &Scope, deployment: Id, role: &str, index: u32, trigger: ReplaceTrigger) -> Result<Id>;
/// 旧主机可达（Manual 且主机不在新绑定中，或 Auto 下探测成功）时先执行 stop_command；
/// 总是插入隔离行，停成功则同时写 cleared_at；记 fenced 事件（detail.stopped）。
pub(crate) async fn fence(s: &AppState, app: &apps::Model, host: Id, instances: &[String]) -> Result<()>;
pub(crate) fn stop_command(app: &apps::Model, instance: &str) -> String;   // docker: 标签属主校验后 docker rm -f；容器不存在视为成功。native: systemctl disable --now <project>-<instance>.service
pub(crate) async fn fenced_hosts(conn: &impl ConnectionTrait, workspace: Id, deployment: Id) -> Result<BTreeSet<Id>>;
#[derive(Deserialize)] #[serde(deny_unknown_fields)] pub struct ReplaceBody { version: i64 }
pub async fn replace(State<AppState>, Scope, Path<(String, String, String, u32)>, Json<ReplaceBody>) -> Result<(StatusCode, Json<Value>)>;  // 202 {"id": revision}
// deployments/mod.rs
pub(crate) async fn scheduled_replace(s: &AppState, scope: Scope, id: &str, action: &'static str) -> Result<Id>;  // = scheduled() 的预览+trigger，revision.action = action
// hosts/fake.rs
pub unreachable: AtomicBool   // 为真时 exec/打开通道返回连接错误
```
- 决定：放置位置以 `blueprint_binding` 为准；规格里该角色若有人工 `placement`，同步改写为新主机（地址取主机地址），保证 `same_topology(stored, 新规格)` 与绑定一致（写入 ADR，Task 10）。
- 决定：`groups::targets` 对蓝图部署把「尚无 `deployment_target_states` 行的主机」排在最前，其余仍按主机 ID；新成员因此先于其他成员重建（预览与 `enqueue` 调用同一函数，顺序一致）。
- 决定：`groups::execute` 遇到 `action` 以 `-replace` 结尾且 `hash(row.artifact) == row.expected_hash` 的目标不执行远程命令，目标记 `succeeded`，输出 `deployment.output.hostUnchanged`，并照常更新该主机的当前修订指针。普通 apply 行为不变。
- 决定：修订成功后删除不再出现在绑定中的主机的 `deployment_target_states` 行（否则监控与删主机检查一直认为它承载部署）。
- 手动 API：`POST /api/workspaces/{ws}/deployments/{id}/instances/{role}/{index}/replace`，`deployments.apply`；stack/compose → 400 `deployment.replaceUnsupportedDriver`；class Manual → 400 `deployment.replaceNeedsBackup`；version 不符 409 `deployment.error.030`。本任务直接调用 `replace_instance(Manual)`；Task 5 换成完整序列。

- 决定：回滚目标若早于该部署最近一次成功的替换修订，拒绝 400 `deployment.rollbackAcrossReplace`（旧修订的产物属于旧放置，而绑定已是新放置；回滚放置推迟）。

- [ ] Step 1 失败测试（placement.rs）：`replace_moves_only_that_instance`（3 主机 + 备用主机，nats-1 换到备用，其余项逐字不变）、`replace_respects_selector_distinct_and_spread`（备用主机缺标签不选；已有该角色实例的主机不选；两个备用按 spread → 负载 → ID 选择）、`replace_moves_colocated_group`（sentinel colocateWith redis：替换 redis-1 同时移动 sentinel-1；对 sentinel-1 调用等同对 redis-1）、`replace_without_candidate_is_an_error_not_a_panic`（返回 `blueprint.replaceNoCandidate`，含 role 与 instance）、`fenced_host_excluded_from_candidates`（excluded 中的唯一备用主机不被选）。
- [ ] Step 2 失败测试（failover_tests.rs）：`manual_replace_creates_manual_replace_revision`（docker 部署成功后调用 API：202、新修订 `action=="manual-replace"`、绑定只变 nats-1、`version`+1、事件 replaceStarted 与 fenced 存在）、`replace_targets_new_host_first_and_skips_unchanged`（修订目标首位是新主机；未变的主机目标 `succeeded` 且其 FakeHost 未收到 `docker run`）、`manual_replace_stops_instance_on_reachable_old_host`（旧主机 FakeHost 收到 `docker rm -f deployment-…-nats-1`；隔离行 `cleared_at>0`）、`manual_replace_rejects_stack_compose_and_manual_class`、`manual_replace_requires_apply_permission`（缺 `deployments.apply` 403）、`spec_placement_mirrors_replaced_binding`（规格人工 placement 中该实例的 hostId 被改写；随后改 values 的更新不报 `topologyChangeRequiresPlan`）、`replace_success_drops_old_host_target_state`、`rollback_across_replace_rejected`（替换成功后对替换前修订预览/回滚返回 `deployment.rollbackAcrossReplace`；对替换修订本身回滚允许）。
- [ ] Step 3 `cargo test -p ferry-server -- blueprint::placement::tests::replace fenced_host_excluded failover_tests::manual_replace failover_tests::replace_ failover_tests::spec_placement failover_tests::rollback_across` → FAIL。
- [ ] Step 4 实现；六语补键；`python scripts/generate-platform-tools.py`。
- [ ] Step 5 同一命令 PASS；全量 `cargo test -p ferry-server`、`python scripts/generate-platform-tools.py --check` PASS。
- [ ] Step 6 Commit `feat(deployments): replace one blueprint instance as a new revision`

### Task 5: 钩子执行与 B 类替换序列

**Files:**
- Create: `crates/server/src/deployments/hooks.rs`、`crates/server/src/deployments/sequence.rs`
- Modify: `crates/server/src/deployments/replace.rs`（手动 API 改走序列）、`crates/server/src/i18n/deployment-*.json`

**Interfaces:**
- Consumes: Task 1 `RoleHooks`；Task 2 `class_of`/`respond`；Task 3 `events::record`/`history`；Task 4 `replace_instance`、`ReplaceTrigger`。
- Produces:
```rust
// hooks.rs
pub(crate) enum Hook { Join, Leave, Healthy }
/// 用 groups::compiled(app) 中该实例的 Context 以 Phase::Instance 渲染每个 argv 项，然后：
/// docker: `timeout <secs> docker exec <project>-<instance> <argv…>`；native: `cd <dir> && FERRY_INSTANCE=<i> FERRY_ROLE=<r> timeout <secs> <argv…>`（均经 quote）。
pub(crate) fn hook_command(app: &apps::Model, instance: &str, argv: &[String], ctx: &Context, timeout_secs: u64) -> Result<String>;
pub(crate) struct HookRun { pub code: i32, pub output: String /*StoredText*/ }
/// 经 remote::open 执行；输出按 execution_output 的 protected 规则脱敏（当前修订有变量快照或变量绑定即 protected，只保留退出码），截断 8 KiB；记 hookOutput 事件。
pub(crate) async fn run(s: &AppState, app: &apps::Model, host: Id, instance: &str, hook: Hook, argv: &[String], timeout_secs: u64) -> Result<HookRun>;
pub(crate) const HOOK_TIMEOUT_SECS: u64 = 120;
pub(crate) const HEALTHY_ATTEMPT_SECS: u64 = 15;
// sequence.rs
pub(crate) struct Job { pub workspace: Id, pub deployment: Id, pub role: String, pub index: u32, pub trigger: ReplaceTrigger, pub actor: Id }
pub(crate) enum Outcome { Succeeded { revision: Id }, Failed { reason: &'static str }, Manual { reason: &'static str } }
pub(crate) fn quorum_ok(members: usize, healthy_excluding_failed: usize) -> bool;   // healthy >= members/2 + 1
/// 获取部署动作锁 `failover:<deployment>`（locks 表，TTL 120s，执行中每 30s 续期）；若有未结束的 replaceStarted 先记 replaceFailed{interrupted}；
/// 记 replaceStarted；B 类：统计健康成员（有 healthy 钩子则逐个执行，否则用监控实例状态 ≠ down 且主机不陈旧）→ !quorum_ok 时记 needsAttention{quorumLost} 返回 Manual；
/// 有 leave 钩子则在第一个健康同伴上执行，失败 → Failed{hookFailed}；然后 replace_instance。返回修订 ID。
pub(crate) async fn begin(s: &AppState, scope: &Scope, job: &Job) -> Result<std::result::Result<Id, Outcome>>;
/// 轮询修订终态（间隔 2s，上限 2×healthTimeout+600s）→ 失败 Failed{revisionFailed}；B 类：join 钩子（新实例内）→ healthy 钩子每 5s 一次直至 healthTimeout，
/// 无钩子时用监控实例状态 up，监控无数据（unknown）则以修订自身的就绪等待为准 → 记 replaceSucceeded / replaceFailed；第 3 次连续失败追加 needsAttention{tooManyFailures}；释放锁。
pub(crate) async fn finish(s: AppState, job: Job, revision: Id) -> Outcome;
```
- 手动 API（控制决策：请求不得长时间阻塞）：同步部分只做校验、类型/驱动判定、取动作锁与仲裁检查（不执行任何远程钩子），随即返回 202 `{"id": revision}`；leave 钩子、修订执行、join、healthy 全部在 `tokio::spawn` 中完成，进度与失败只通过事件时间线呈现；`Manual{quorumLost}` → 409 `deployment.replaceQuorumLost`；锁被占用 → 409 `deployment.failoverBusy`。手动替换不受冷却限制，但仍计入连续失败。

- [ ] Step 1 失败测试（sequence.rs 单元）：`quorum_rule`（(3,2)→true、(3,1)→false、(5,3)→true、(5,2)→false、(1,0)→false）。
- [ ] Step 2 失败测试（hooks.rs 单元）：`hook_command_docker_and_native`（argv 中 `{{ .Instance.Name }}` 渲染为 `nats-1`；含空格与单引号的项被 quote；docker 形如 `timeout 120 docker exec 'deployment-1-9-nats-1' …`；native 以 `cd '<dir>' &&` 开头）。
- [ ] Step 3 失败测试（failover_tests.rs）：`class_b_runs_leave_replace_join_healthy_in_order`（蓝图 nats 角色带三个钩子；FakeHost 记录：leave 在健康同伴、之后新主机先出现 `docker run`，再 join、healthy 在新主机；事件顺序 replaceStarted → hookOutput(leave) → hookOutput(join) → hookOutput(healthy) → replaceSucceeded）、`quorum_loss_goes_manual`（3 成员中两台主机 `unreachable`：不产生新修订；两实例分别记 needsAttention{quorumLost}；API 返回 409）、`leave_hook_failure_stops_before_replacing`（leave 退出 1 → replaceFailed{hookFailed}，绑定不变，无新修订）、`healthy_timeout_records_failure`（healthy 始终退出 1，`update.healthTimeout` 设 2 → replaceFailed{unhealthy}）、`hook_output_redacted_when_protected`（部署引用秘密变量：hookOutput 的 detail 只含退出码，不含 FakeHost 返回的 stdout 文本）、`concurrent_manual_replace_is_busy`（锁已被占 → 409 `deployment.failoverBusy`）。
- [ ] Step 4 `cargo test -p ferry-server -- deployments::sequence deployments::hooks failover_tests::class_b failover_tests::quorum failover_tests::leave failover_tests::healthy failover_tests::hook_output failover_tests::concurrent` → FAIL。
- [ ] Step 5 实现；六语补键。
- [ ] Step 6 同一命令 PASS；全量 `cargo test -p ferry-server` PASS。
- [ ] Step 7 Commit `feat(deployments): hook runner and one-member-at-a-time replacement sequence`

### Task 6: 协调器循环、租约、冷却/人工确认、恢复后解除隔离、告警

**Files:**
- Create: `crates/server/src/deployments/reconciler.rs`
- Modify: `crates/server/src/main.rs`（spawn + 路由）、`crates/server/src/deployments/mod.rs`（`get` 返回 `failover` 摘要）、`crates/server/src/notifications/alerts.rs`（新指标）、`crates/server/src/i18n/deployment-*.json`（含告警文案 `deployment.alert.failoverAttention`/`failoverRecovered`）

**Interfaces:**
- Consumes: Task 2 全部；Task 3 `events::*`；Task 4 `fence`/`stop_command`/`fenced_hosts`；Task 5 `sequence::{Job,begin,finish}`。
- Produces:
```rust
pub(crate) const TICK: Duration = Duration::from_secs(15);
pub(crate) const LEASE_NAME: &str = "deployment-reconciler";
pub(crate) const LEASE_MS: i64 = 60_000;
#[async_trait::async_trait]
pub(crate) trait Probe: Send + Sync {
    async fn server(&self, s: &AppState, workspace: Id, host: Id) -> bool;                                     // remote::open + `echo ferry-ok`，10s
    async fn peer(&self, s: &AppState, workspace: Id, peer: Id, address: &str, port: u16) -> PeerCheck;   // 经同伴通道执行 peer_check_command
}
pub(crate) struct LiveProbe;
/// nc 优先：`timeout 3 nc -z <addr> <port>`；否则 `timeout 3 bash -c 'exec 3<>/dev/tcp/<addr>/<port>'`；两者都没有 exit 127 → Inconclusive。0 → Reachable，其他 → Unreachable。
pub(crate) fn peer_check_command(address: &str, port: u16) -> String;
#[derive(Default)] pub(crate) struct Episodes(HashMap<(i64 /*deployment*/, i64 /*host*/), Episode>);   // 同一失联期内 hostSuspected/peerVeto/hostConfirmedDown/alertOnly 各记一次；主机恢复 Healthy 时清除
/// 一轮：遍历有当前修订的 blueprint 部署 → settings/respond → 本轮内每台主机至多探测一次 → judge →
/// Vetoed: peerVeto；Suspected: hostSuspected；Down: hostConfirmedDown，再按每个受影响实例：AlertOnly → alertOnly；Replace → 若 history 非 attention/open、不在冷却、部署无运行中动作，则产出 Job（每部署每轮至多一个）。
/// 另外：对所有未清除隔离行，若主机已不陈旧且 server 探测成功 → stop_command，成功则写 cleared_at + unfenced，失败保留（不记事件，下轮重试）。
pub(crate) async fn tick(s: &AppState, probe: &dyn Probe, now_ms: i64, episodes: &mut Episodes) -> Result<Vec<sequence::Job>>;
/// lock::acquire(LEASE_NAME, s.instance_id, now, LEASE_MS) 成功才执行 tick；产出的 Job 以 actor = 部署当前修订的 triggered_by 构造 Scope（失效 → replaceFailed{actorRevoked}），
/// 顺序执行 begin + spawn(finish)；drain 时退出并释放租约。
pub async fn run(s: AppState, drain: tokio::sync::watch::Receiver<bool>);
// ack
#[derive(Deserialize)] #[serde(deny_unknown_fields)] pub struct AckBody { version: i64 }
pub async fn acknowledge_instance(State<AppState>, Scope, Path<(String, String, String, u32)>, Json<AckBody>) -> Result<StatusCode>;  // 204；deployments.apply；无 attention → 409 deployment.failoverNotAttention
pub(crate) async fn summary(conn: &impl ConnectionTrait, app: &apps::Model) -> Result<Value>;
// {"mode","host_down_after","fences":[{host_id,instance,created_at}],"attention":[{instance,since}]}
```
- 路由：`POST /api/workspaces/{ws}/deployments/{id}/instances/{role}/{index}/acknowledgements`。`GET /deployments/{id}` 增加 `failover` 字段（非蓝图部署为 null）。
- 告警：`target_kind = "deployment"` 新增指标 `failover-attention`（`alerts::validate` 允许；`abnormal`：存在 attention 实例，或存在 alertOnly 事件而其主机仍陈旧超过 hostDownAfter → `(true, deployment.alert.failoverAttention{count})`，否则 `(false, deployment.alert.failoverRecovered)`）。用户按现有方式为部署建告警规则即可收到通知。

- [ ] Step 1 失败测试（reconciler.rs 单元）：`peer_check_command_prefers_nc_then_bash`（字符串含 `nc -z`、`/dev/tcp/10.0.0.5/22`、`exit 127`；地址与端口经 quote）。
- [ ] Step 2 失败测试（failover_tests.rs，`StubProbe` 以表驱动返回 server/peer 结果，`now` 由测试推进）：`fresh_hosts_are_not_probed`（StubProbe 调用计数为 0）、`peer_veto_records_event_and_does_not_replace`（SSH 主机陈旧、server 失败、peer Reachable：两轮 tick 只有一条 peerVeto，无 fenced/Job）、`confirmed_down_stateless_produces_job_then_replaces`（tick 返回 1 个 Job；执行 begin/finish（测试内用 `execute`+`finish` 推进修订）→ 事件 hostConfirmedDown → fenced → replaceStarted → replaceSucceeded，修订 `action=="auto-replace"`）、`runner_host_is_confirmed_by_single_source_after_twice_the_window`、`alert_mode_records_alert_only_and_does_not_move`（alertOnly 一次，绑定不变，`failover-attention` 告警 abnormal）、`stack_stateless_is_ignored_and_compose_alerts`、`one_action_per_deployment`（同一主机承载 web-0 与 lb-0：一轮只产出 1 个 Job，下一轮（前一个 open 结束后）产出第 2 个）、`cooldown_and_three_failures_need_attention`（让替换修订失败 3 次，每次推进 11 分钟：第 3 次后 needsAttention，第 4 轮不产出 Job；9 分钟内不产出 Job；ack 后下一轮产出 Job）、`no_candidate_records_event_and_respects_cooldown`（无备用主机：replaceFailed{noCandidate}，无隔离行，无新修订，10 分钟内再 tick 不重复记录）、`returning_host_stops_fenced_instances_before_clearing`（隔离主机恢复：先收到 stop 命令，命令成功后才有 cleared_at 与 unfenced；stop 失败时 cleared_at 保持 0、无 unfenced）、`lease_lets_only_one_instance_act`（两个 instance_id 竞争租约，仅一个 `acquire` 为真）、`ack_requires_attention_and_apply_permission`、`deployment_get_includes_failover_summary`。
- [ ] Step 3 `cargo test -p ferry-server -- deployments::reconciler failover_tests` → FAIL。
- [ ] Step 4 实现；`main.rs` 在 `deployment_task` 旁 `tokio::spawn(deployments::reconciler::run(s.clone(), drain_rx.clone()))` 并加入关停等待；六语补键；`python scripts/generate-platform-tools.py`。
- [ ] Step 5 同一命令 PASS；全量 `cargo test -p ferry-server`、`python scripts/generate-platform-tools.py --check`、`cargo fmt --all -- --check` PASS。
- [ ] Step 6 Commit `feat(deployments): leased failover reconciler with fencing, cooldown and acknowledgement`

### Task 7: 官方模板更新与市场索引

**Files:**
- Create: `marketplace/apps/nats-cluster/1.1.0/`（复制 1.0.0 的 `blueprint.yaml`、`parameters.yaml`、`files/nats.conf`、`README.md`，新增 `verified.yaml` 仅含说明注释）
- Modify: `marketplace/index.json`（脚本生成）、`marketplace/apps/nats-cluster/1.0.0/README.md` 不改
- Test: 现有 `official_blueprints_compile_for_supported_drivers`（读取索引，覆盖 1.1.0）

**Interfaces:**
- 1.1.0 的 `nats` 角色新增：
```yaml
failover:
  replace: supported
hooks:
  healthy: ["wget", "-q", "-O", "/dev/null", "http://127.0.0.1:8222/healthz?js-enabled-only=true"]
```
- 决定：NATS 不设 `leave`/`join`。替换成员沿用同一 `server_name`（`{{ .Instance.Name }}`，JetStream 以服务器名派生 peer ID），新主机上为空卷，按路由重新加入并追平副本；`nats:2.10-alpine` 镜像不含 `nats` CLI，无法在镜像内执行 `peer-remove`，因此不依赖它。`healthy` 只用镜像自带的 busybox `wget`。
- nginx-ha 不改：web/lb 均为 stateless，即 A 类，无钩子。
- README（1.1.0，六语中的现有语言段落）补一段「故障自愈」：A/B 类说明、`failover.mode` 与 `hostDownAfter`。

- [ ] Step 1 确认镜像工具（本地或 CI 均可，结果写进 1.1.0 README 的说明）：`docker run --rm nats:2.10-alpine sh -c 'command -v wget; command -v nats || echo no-nats-cli'` → 输出含 `/usr/bin/wget` 与 `no-nats-cli`。无法运行 Docker 时在本任务报告中标注「待 CI 核实」，Task 10 的 CI 步骤会在新主机上执行该钩子。
- [ ] Step 2 失败测试：在 `crates/server/src/blueprint/model.rs` 测试模块增加 `official_nats_declares_replicated_failover`（从 `marketplace/index.json` 取 nats-cluster 最新版本：`class_of(nats) == Replicated`、`hooks.healthy` 首项为 `wget`、`hooks.leave` 为 None）。
- [ ] Step 3 `cargo test -p ferry-server official_` → FAIL。
- [ ] Step 4 建 1.1.0 目录；`node scripts/build-marketplace-index.mjs`。
- [ ] Step 5 `node scripts/build-marketplace-index.mjs --check`；`cargo test -p ferry-server official_` PASS。
- [ ] Step 6 Commit `feat(marketplace): nats-cluster 1.1.0 declares member replacement`

### Task 8: 前端规格 failover 与向导

**Files:**
- Modify: `src/features/deployments/blueprint-spec.ts`、`src/features/deployments/blueprint-spec.test.ts`、`src/features/deployments/BlueprintWizard.tsx`、`src/features/deployments/BlueprintWizard.test.tsx`、`src/shared/i18n/locales/resource-domain.json`（`wizard.failover*` 六语）

**Interfaces:**
- Produces:
```ts
export type FailoverMode = "off" | "alert" | "auto";
export interface FailoverSpec { mode?: FailoverMode; hostDownAfter?: number }
// DeploySpec 新增 failover?: FailoverSpec；toSpec 允许顶层键 failover，只允许 mode/hostDownAfter，mode 枚举、hostDownAfter 为 30–3600 整数，否则 fail("invalidField")
export const HOST_DOWN_AFTER = { min: 30, max: 3600, default: 90 } as const;
export type FailoverClass = "stateless" | "replicated" | "manual";
// BlueprintRole 新增 failoverReplace: boolean（role.failover.replace === "supported"）
export function failoverClass(role: BlueprintRole): FailoverClass;   // 与服务端 class_of 相同
// topologyKey 解构时去掉 failover
```
- 向导：「角色与主机」步骤顶部加「故障处理」`<select>`（自动/仅告警/关闭，缺省显示自动，写入 `failover.mode`）；未冻结时在角色步骤，冻结（已部署）时在唯一的参数步骤显示，可修改。提示：driver 为 stack/compose 时显示 `wizard.failoverDriverAlertOnly`；auto 且存在 manual 类角色时逐角色显示 `wizard.failoverNeedsBackup`（说明需 P4 备份，当前只告警）。`hostDownAfter` 只在 YAML 中编辑，表单显示当前值（只读）。Phone 下 select 全宽、最小触控高度 44px。

- [ ] Step 1 失败测试（blueprint-spec.test.ts）：`failover_roundtrip_and_validation`（读写保留注释；`mode: sometimes`、`hostDownAfter: 10` 报错；未知子键报错）、`topology_key_ignores_failover`、`failover_class_matches_server_rules`（stateless/stable+supported/stable）。
- [ ] Step 2 失败测试（BlueprintWizard.test.tsx）：`failover_select_writes_spec`（选择「仅告警」后 YAML 含 `mode: alert`）、`failover_editable_after_deploy`（冻结编辑时 select 可用，改动不触发 locked/拓扑错误，提交体含新 failover）、`manual_class_role_explains_backup_needed`、`stack_driver_explains_alert_only`。
- [ ] Step 3 `npx vitest run src/features/deployments` → FAIL。
- [ ] Step 4 实现；六语补键。
- [ ] Step 5 `npx vitest run src/features/deployments`、`npx tsc -b` PASS。
- [ ] Step 6 Commit `feat(deployments): failover mode in the blueprint wizard`

### Task 9: 前端详情：自动动作时间线、替换、需人工处理、拓扑标记

**Files:**
- Create: `src/features/deployments/FailoverTimeline.tsx`、`src/features/deployments/FailoverTimeline.test.tsx`
- Modify: `src/features/deployments/deployment.ts`（`failover` 字段、revision action 标签）、`src/features/deployments/blueprint-hooks.ts`（`useFailoverEvents`）、`src/features/deployments/BlueprintTopology.tsx` + `.test.tsx`（徽标、替换动作）、`src/features/deployments/DeploymentDetail.tsx` + `.test.tsx`、`src/features/alerts/resource-config.tsx`（`failover-attention` 指标选项）、`src/shared/i18n/locales/resource-domain.json`（`failover.*`、`topology.fenced`、`topology.needsAttention`）、`scripts/test-blueprint-wizard-ui.mjs`

**Interfaces:**
- Consumes: Task 3 事件 API、Task 4 替换 API、Task 6 ack API 与 `failover` 摘要。
- Produces:
```ts
export interface FailoverSummary { mode: string; host_down_after: number; fences: { host_id: string; instance: string; created_at: number }[]; attention: { instance: string; since: number }[] }
// Deployment 新增 failover?: FailoverSummary | null
export interface FailoverEvent { id: string; kind: string; instance: string; host_id: string; host_name?: string; revision_id: string; created_at: number; message: string; detail: Record<string, unknown> }
export function useFailoverEvents(workspace: string, id: string, page: number, reload: unknown): { loading: boolean; error: string; items: FailoverEvent[]; total: number; retry: () => void };
export function FailoverTimeline(props: { workspace: string; deployment: Deployment }): JSX.Element;
export function FailoverAttention(props: { workspace: string; deployment: Deployment; permissions: Set<string>; onDone: () => void }): JSX.Element | null;
export const revisionActionLabel: (action: string) => string;   // apply/rollback/restart/auto-replace/manual-replace
```
- 时间线：倒序列表，每项 时间 · 种类标签 · 实例 · 主机名 · 服务端渲染的 `message`；`revision_id` 非 0 时链接到修订；分页（每页 20，上一页/下一页）；状态：加载骨架、空（`failover.empty`）、错误 + 重试、超长消息折行不撑破布局。仅蓝图部署显示。
- 需人工处理横幅：`failover.attention` 非空时置顶，列出实例与起始时间；有 `deployments.apply` 时每个实例一个「确认」按钮（POST acknowledgements，带 version，成功后刷新）。
- 替换动作：拓扑行内「替换」按钮，仅当 `mode ∈ {native, docker}`、`permissions.has("deployments.apply")`、`failoverClass(role) !== "manual"`、部署非 queued/running/needs-review；用 `DeleteConfirmDialog`（action=「替换」，description 说明会在其他主机重建、B 类会执行钩子）确认；成功后刷新详情。
- 拓扑徽标：实例在 `fences` 中显示「已隔离」（按 host_id+instance 匹配），在 `attention` 中显示「需人工处理」，与健康徽标并列，Phone 下换行不溢出。
- 修订标题使用 `revisionActionLabel`。

- [ ] Step 1 失败测试（FailoverTimeline.test.tsx）：`timeline_renders_events_with_host_names_and_revision_links`、`timeline_states_loading_empty_error_retry`、`timeline_paginates`（total=45 → 3 页，按钮可用性正确）、`attention_banner_acknowledges_with_version`（POST 路径与 body `{version}`；无 apply 权限不显示按钮）。
- [ ] Step 2 失败测试（BlueprintTopology.test.tsx）：`fenced_and_attention_badges_shown`、`replace_action_only_for_supported_driver_class_and_permission`（stack、compose、manual 类、无 `deployments.apply` 时都不显示）、`replace_confirms_then_posts`（确认对话框后 POST `…/instances/nats/1/replace` body `{version}`；取消不请求）。
- [ ] Step 3 失败测试（DeploymentDetail.test.tsx）：`revision_title_labels_replace_actions`；alerts：`deployment_target_offers_failover_attention_metric`（加在 `src/features/alerts` 现有测试文件中）。
- [ ] Step 4 `npx vitest run src/features/deployments src/features/alerts` → FAIL。
- [ ] Step 5 实现；六语补键。
- [ ] Step 6 `npx vitest run`、`npx tsc -b`、`npm run build` PASS。
- [ ] Step 7 扩展 `scripts/test-blueprint-wizard-ui.mjs`：在已创建的部署详情页用 `page.route` 为 `…/events` 与部署 GET 的 `failover` 字段注入固定数据（含 1 条超长消息、1 个隔离实例、1 个需人工处理实例），在 1440/900/390/320 截图到 `data/maintenance/orchestration-p3/`，断言无整页横向滚动、320 下「确认」按钮可达；本地运行通过。
- [ ] Step 8 Commit `feat(deployments): failover timeline, replace action and attention banner`

### Task 10: CI 验收、ADR 与设计文档

**Files:**
- Modify: `scripts/test-deployments.py`（`FERRY_TEST_BLUEPRINT=1` docker 分支）、`.github/workflows/deployment-ha.yml`（paths 增加 `crates/server/src/migration/**`、`crates/server/src/hosts/**`、`crates/server/src/notifications/**`）、`ADR.md`（新增「集群编排 P3」条目）、`docs/集群编排与故障自愈设计.md`（§6/§7/§8 实现状态）

**Interfaces:**
- 夹具：docker 分支创建第 4 台 SSH 夹具 `blueprint-ssh-3`（F3）。标签：`bp-nats` → F0..F3（NATS 3 个，落在 F0/F1/F2，F3 备用）；`bp-web` → F1/F2/F3（web 2 个，落在 F1/F2，F3 备用）；`bp-lb` → F0。原有「实例恰好落在打标签主机」断言改为「落在打标签主机的子集，且等于放置预览」。
- 规格：nginx 与 NATS 均写 `failover: {mode: auto, hostDownAfter: 30}`；另建 `nginx-alert`（`mode: alert`，web 由新标签 `bp-alert-web` 选 F1/F2，lb 选 F2，端口 19080/19000）。
- 故障注入（放在 docker 分支末尾、compose 段之后）：先向 NATS 流 `FERRY_FAILOVER`（3 副本）发布 20 条消息并记录；`docker stop` F1（承载 web-0 与 nats-1，不承载 lb）。
- 诊断：失败时把三个部署的 `GET …/events?page_size=100` 写入 `work/failover-events.log` 并 `report_log_tail`；保留现有 `diagnostics()`。

- [ ] Step 1 修改脚本，断言（均用 `eventually`，上限 300s）：
  - nginx 与 NATS 部署事件依次含 `hostConfirmedDown`、`fenced`、`replaceStarted`、`replaceSucceeded`，且 `replaceStarted` 的 instance 分别为 `web-0`、`nats-1`；最新修订 `action == 'auto-replace'` 且 `succeeded`。
  - nginx：绑定中 web-0 的 hostId 为 F3；`balanced(lb_url, 当前 message)` 通过；F3 上 `owned(F3, nginx_id) == ['web-0']`。
  - NATS：绑定中 nats-1 在 F3；三个成员 `meta_cluster(..)['cluster_size'] == 3`；`replicated_stream(..)` 的 `cluster.replicas` 全部 `current == true`；从 F3 上的 nats-1 读取流状态 `state.messages == 20`；`hookOutput` 事件存在且 healthy 退出码 0。
  - `nginx-alert`：事件含 `alertOnly`（host 为 F1）且不含 `replaceStarted`；绑定与故障前相同。
  - 恢复：`docker start` F1 并重新 `docker exec -d … nested-daemon …`；等待其 SSH 连接测试成功；nginx 与 NATS 均出现 `unfenced`；`docker ps -a` 在 F1 上不再有这两个部署标签的容器；`GET /deployments/{id}` 的 `failover.fences` 为空。
  - `checks` 下限按新增断言数上调。
- [ ] Step 2 本地：`python -m py_compile scripts/test-deployments.py`；全量 `cargo test -p ferry-server`；`cargo fmt --all -- --check`；`npx vitest run`；`npx tsc -b`；`npm run build`；`python scripts/generate-platform-tools.py --check`；`node scripts/build-marketplace-index.mjs --check`。
- [ ] Step 3 ADR 条目「集群编排 P3：协调器 + 无状态/复制型自愈（2026-11-12，分支 feature/orchestration-p3）」，至少写明：驱动范围与 Swarm 重新固定推迟；类型判定；三步失联确认、runner 与无同伴 SSH 的单源规则、`/dev/tcp` 用 bash 而非 sh；隔离与恢复顺序；放置以 binding 为准并镜像人工 placement；新主机优先的目标顺序与替换修订跳过未变主机；回滚不得跨越替换修订；动作执行身份 = 当前修订触发人；事件推导冷却/失败/确认（新增 `acknowledged` 种类）；无规格的旧蓝图部署按 alert；告警指标 `failover-attention`；NATS 1.1.0 不设 leave 钩子的理由；未覆盖边界（stack 有状态重新固定、C/D 类、maxConcurrent、资源打分仍未做）。
- [ ] Step 4 设计文档：§6 各小节标注 P3 已实现/未实现部分；§7 第 3 条「自动动作时间线」改为已实现并说明替换与确认入口；§8 P3 行补「实现状态」与 CI 验收待推送。
- [ ] Step 5 Commit `test(deployments): failover acceptance on a killed fixture host; P3 ADR`
- [ ] Step 6 CI：不得自行推送；向控制者报告就绪，由其征得用户同意后推送到 `ci/scenario-templates-*` 分支，读取公开注解；通过后另行补 nats-cluster 1.1.0 的 `verified.yaml` 证据（单独提交）。

## 不在本计划

- Swarm 有状态实例重新固定节点、`compose` 自动迁移（仅告警）。
- C/D 类（共享存储切换、备份恢复）、`backup`/`restore`/`promote` 钩子、`storage` 字段（P4/P5）。
- `failover.maxConcurrent`、按资源余量打分、扩缩容等一般拓扑变更；回滚到替换前的放置。
- 实例级故障升级（§6.2：驱动重启后仍连续不健康 → 替换）；P3 只处理主机失联。
