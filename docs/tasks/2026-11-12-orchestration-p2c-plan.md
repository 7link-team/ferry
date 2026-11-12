# 集群编排 P2c：部署规格 + 标签调度 + 部署向导 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 用户在应用市场选一个集群模板 → 填参数 → 设每个角色实例数 → 用主机标签/主机名/主机组圈定可用主机 → 系统自动放置并预览 → 部署；全程有等价的 `deploy.yaml` 可在 YAML 模式下编辑；部署后详情页显示角色/实例/主机拓扑与健康。

**Architecture:** 新增「部署规格」（`ferry/deploy-v1`，存 `deployments.deploy_spec`）：引用本空间应用模板 + 参数 + 角色实例数 + 主机选择器。服务端渲染模板参数（复用 `templates::render_content`）得到蓝图，调度器（纯函数）根据主机候选集（在线/Linux/标签/主机组/资源）生成放置（即现有 `Binding`），之后沿用 P2a/P2b 的编译、预览、执行、回滚。Swarm 下把主机标签同步为节点 label，供无状态角色约束。监控模块按每主机编译产物理解蓝图部署。前端新增部署向导页（多步）与规格 YAML 双向编辑、详情页拓扑视图。

**Tech Stack:** Rust（axum、sea-orm、serde_yaml_ng）、React + 现有 ResourceForm/CodeEditor（Monaco + monaco-yaml）、`yaml` npm 包（保留注释）、vitest、Playwright。

**Spec:** `docs/集群编排与故障自愈设计.md` §2、§3、§5、§7（P2c 行）；上位 `docs/应用市场与集群模板设计.md`。

## Global Constraints

- 部署规格 `apiVersion: ferry/deploy-v1`；本期字段：`template{id,version}`、`driver`、`manager`（仅 stack）、`values`、`roles.<r>.instances`、`roles.<r>.hosts{tags[],names[],group,spread{prefix}}`、`placement`（可选，人工覆盖的放置，形如现有 Binding.roles）。`failover`/`backup`/`storage` **不在本期**（P3/P4 增加，规格解析对未知顶层键报错，后续以新增字段扩展）。
- 选择器语义：`tags` 为主机必须**同时**具备的标签名（现有主机标签是纯文本名）；`names` 为主机名列表；`group` 为主机组名；三者同时给出时取交集；都不给时候选为全部可用主机。`spread.prefix`（如 `zone:`）：同一角色实例尽量分布到带不同 `<prefix>*` 标签的主机，软约束，不满足只警告。
- 候选主机必须：属于本工作空间、`state = online`、`os` 为空或 `linux`；Swarm 下还需为管理节点所在 Swarm 的 Ready/Active 节点（沿用 P2b 预览期核对）。
- 调度结果确定：同输入同输出（排序键：违反 spread 少 → 本部署已有实例少 → 内存余量多 → 主机 ID 升序）。
- 失败时给出可读原因，按角色列出（例：`nats` 需要 3 台满足 `tags=[cache]` 的在线 Linux 主机，当前 2 台：host-a、host-b）。
- 首次修订后放置冻结（沿用 `blueprint.topologyChangeRequiresPlan`）；规格中的 `values` 改变 = 普通升级（重新渲染、预览、执行）。
- 页面遵守 AGENTS.md 页面门槛：Desktop ≥1200 / Pad 768–1199 / Phone 320–767 三类布局，禁止整页横向滚动，全状态（默认/空/加载/错误/极端值），全部文案六语（zh-CN/zh-TW/en-US/ms-MY/ja-JP/ko-KR）。
- 编译与测试：`CARGO_TARGET_DIR=D:/Projects/github/ferry-marketplace-p1/target`；`cargo fmt --all -- --check` 必须通过。

## Review Focus

1. 选择器匹配的主机少于角色最少实例数 / 只够 distinctHosts 不够 → 明确拒绝并列出匹配到的主机 → Task 3 `insufficient_hosts_explained`。
2. 用户在 YAML 里手写 `placement` 覆盖，包含离线或不匹配选择器的主机 → 拒绝（不静默改写）→ Task 4 `manual_placement_must_satisfy_selector`。
3. 主机标签改名/删除后再预览已有部署 → 冻结的放置照常使用，不重新调度 → Task 4 `frozen_placement_ignores_tag_changes`。
4. 向导 YAML 模式写入语法错误或未知字段 → 表单不被破坏、提交被阻止、错误可见 → Task 7 `yaml_errors_block_submit_and_keep_form`。
5. Phone 320px 下向导每一步核心操作可达、无横向滚动 → Task 9 浏览器验收。

---

### Task 1: 部署规格模型

**Files:**
- Create: `crates/server/src/blueprint/spec.rs`（`blueprint/mod.rs` 声明；`i18n.rs` include 列表）

**Interfaces:**
```rust
pub struct DeploySpec { pub template: TemplateRef, pub driver: String, pub manager: Option<String /* 主机名或ID */>, pub values: BTreeMap<String, Value>, pub roles: BTreeMap<String, RoleSpec>, pub placement: Option<BTreeMap<String, Vec<binding::InstanceBinding>>> }
pub struct TemplateRef { pub id: Id, pub version: i64 }
pub struct RoleSpec { pub instances: u32, pub hosts: HostSelector }
pub struct HostSelector { pub tags: Vec<String>, pub names: Vec<String>, pub group: Option<String>, pub spread: Option<Spread> }
pub struct Spread { pub prefix: String }
pub fn parse(text: &str) -> Result<DeploySpec>;   // YAML 或 JSON；apiVersion 必须 ferry/deploy-v1；deny_unknown_fields；camelCase
pub fn to_yaml(spec: &DeploySpec) -> String;      // 规范化输出（向导 YAML 模式的初值）
```
- 校验：`instances` 1–100；`tags`/`names` 各 ≤ 32 项、每项 1–60 字；`spread.prefix` 1–30 字；`manager` 仅 stack。新键 `blueprint.spec*`（六语）。
- [ ] Step 1 失败测试：`spec_parses_yaml_and_json`、`unknown_top_level_key_rejected`（如 `failover`）、`bounds_enforced`、`manager_only_for_stack`、`roundtrip_to_yaml`。
- [ ] Step 2 FAIL → Step 3 实现 → Step 4 `cargo test -p ferry-server blueprint::spec` PASS → Step 5 Commit `feat(blueprint): deployment spec model`

### Task 2: 调度候选集

**Files:**
- Create: `crates/server/src/deployments/candidates.rs`

**Interfaces:**
```rust
pub struct Candidate { pub id: Id, pub name: String, pub address: String, pub os: String, pub online: bool, pub group: Option<String>, pub tags: Vec<String>, pub mem_free: Option<u64>, pub disk_free: Option<u64> }
pub async fn load(conn: &impl ConnectionTrait, workspace: Id) -> Result<Vec<Candidate>>;
```
- 来源：`hosts`（state/os/address/group_id/metrics JSON 的 `mem_total-mem_used`、`disk_total-disk_used`）、`host_tags`+`tags`（名字）、`host_groups`（名字）。metrics 缺失或解析失败 → `None`。
- [ ] 失败测试 `candidates_include_tags_group_and_free_memory`（agents::tests::setup 插入两台主机、标签、主机组、metrics）→ 实现 → PASS → Commit `feat(deployments): placement candidates from hosts and tags`

### Task 3: 调度器（纯函数）

**Files:**
- Create: `crates/server/src/blueprint/placement.rs`

**Interfaces:**
```rust
pub struct Placed { pub binding: binding::Binding, pub warnings: Vec<Message> }
pub fn place(bp: &model::Blueprint, spec: &spec::DeploySpec, candidates: &[deployments::candidates::Candidate], existing: &BTreeMap<Id, u32>) -> Result<Placed>;
```
- 步骤：按角色过滤候选（选择器 + online + Linux）；按 `colocateWith` 先放目标角色再对齐序号；`distinctHosts` 同角色不重主机；按排序键（Global Constraints）逐实例选主机；地址取主机 `address`；Swarm 时 `manager` 解析为主机 ID。
- 失败错误：`blueprint.placementInsufficientHosts`（参数 role、needed、matched、hosts 列表 ≤ 10 个名称）、`blueprint.placementSelectorEmpty`。spread 不满足 → warning `blueprint.placementSpreadUnmet`。
- [ ] 失败测试：`places_distinct_hosts_deterministically`、`colocate_follows_target_role`、`tags_are_and_semantics`、`names_and_group_intersect`、`offline_and_non_linux_excluded`、`insufficient_hosts_explained`、`spread_prefers_different_zone_tags`、`spread_unmet_is_warning_not_error`、`prefers_more_free_memory_then_lower_id`。
- [ ] FAIL → 实现 → `cargo test -p ferry-server blueprint::placement` PASS → Commit `feat(blueprint): deterministic tag-aware placement`

### Task 4: 部署规格接入部署 API

**Files:**
- Create: `crates/server/src/migration/deploy_specs.rs`（`deployments.deploy_spec` TEXT NULL；MySQL longtext；down 有非空值时拒绝，参照 `blueprint_deployments.rs`）；注册到迁移列表末尾；`entities.rs`
- Modify: `crates/server/src/deployments/mod.rs`（Body 增加 `deploy_spec: Option<String>`；`check_blueprint`）、`crates/server/src/main.rs`（路由）

**Interfaces:**
- `POST /api/workspaces/{ws}/deployment-placements`，body `{spec: String}` → `{manifest: String /* 渲染后的蓝图 */, binding: Binding, warnings: [..]}`；不落库。权限：`deployments.create` + `app-templates.read` + `hosts.read`。
- 创建/更新：Body 带 `deploy_spec` 时，服务端：解析规格 → 读取本空间应用模板（ID + 版本必须一致，否则 `templates.changed` 409）→ `templates::render_content` 渲染 `values` 得蓝图 → 若规格含 `placement` 则校验其满足选择器/在线/Linux 后作为 binding，否则 `placement::place` → 写入 `manifest`、`blueprint_binding`、`deploy_spec`，`target_kind = blueprint`、`mode = driver`；随后走现有 `check_blueprint`。不带 `deploy_spec` 的旧蓝图部署行为不变。
- 首次修订后：`values` 变化允许（新的 manifest）；`roles`/`hosts`/`placement`/`driver` 变化 → `blueprint.topologyChangeRequiresPlan`；放置使用已存 binding，不重新调度。
- [ ] 失败测试（`deployments/blueprint_tests.rs`）：`placement_endpoint_renders_and_places`、`create_from_spec_stores_spec_manifest_and_binding`、`manual_placement_must_satisfy_selector`、`template_version_mismatch_conflicts`、`values_change_after_deploy_is_upgrade`、`topology_change_after_deploy_rejected`、`frozen_placement_ignores_tag_changes`、`spec_migration_roundtrip`。
- [ ] FAIL → 实现 → 全量 `cargo test -p ferry-server` PASS → Commit `feat(deployments): create blueprint deployments from a deployment spec`

### Task 5: Swarm 标签约束

**Files:**
- Modify: `crates/server/src/blueprint/swarm.rs`、`crates/server/src/blueprint/compile.rs`、`crates/server/src/deployments/groups.rs`

**Interfaces:**
- 节点 label 名：`ferry.w<工作空间ID>.tag.<sha256(tag名)[..12]>=true`（标签名可含任意字符，哈希保证 label 合法；按工作空间隔离，共享同一 Swarm 的工作空间互不调度到、也不改写对方的 label）。
- 仅执行时（P2b 执行期节点探测处，预览只读）：先做请求的廉价校验（expected_hash 格式、预览匹配），再对规格中任一角色选择器用到的标签，在管理节点执行 `docker node update --label-add ferry.w<ws>.tag.<h>=true <node>`（主机有该标签）或 `--label-rm`（没有），仅涉及本部署绑定主机对应的节点；先读取现有 label，已一致则跳过；失败则执行失败（`blueprint.swarmLabelSyncFailed`），不部署。
- 编译：无状态 stack 角色按选择器追加 `node.labels.ferry.w<ws>.tag.<h>==true` 约束（每个标签一条）；有状态角色已固定节点，不追加。
- [ ] 失败测试：`stateless_stack_role_gets_tag_label_constraints`、`label_sync_adds_and_removes_on_manager`（FakeHost 记录命令）、`label_name_is_stable_hash`。
- [ ] FAIL → 实现 → PASS → Commit `feat(blueprint): map host tags to swarm node labels`

### Task 6: 监控理解蓝图部署

**Files:**
- Modify: `crates/server/src/monitoring/mod.rs`（`applications` ~813–991、`validate` ~281–331）、`crates/server/src/monitoring/runtime.rs`（`validate_host` ~25–48、`probe` ~75–97）

**Interfaces:**
- 蓝图部署的服务清单与主机来自当前修订 `rendered` 中的每主机 manifest（P2a 已存 `{"targets":[{host_id,manifest,files}]}`）。
- `applications`：蓝图部署返回每个实例（服务名、主机名、状态）；健康聚合规则与主机组相同。
- `validate_host`：蓝图部署的主机若在当前修订目标中则允许 runtime 检查；`service` 角色检查的 `service_name` 必须出现在某个主机的 manifest 中。
- [ ] 失败测试（`monitoring/tests.rs`）：`blueprint_application_lists_instances_per_host`、`runtime_check_allowed_for_blueprint_target_host`、`service_check_validates_against_compiled_manifests`。
- [ ] FAIL → 实现 → 全量 `cargo test -p ferry-server` PASS → Commit `feat(monitoring): health for blueprint deployments`

### Task 7: 前端部署向导与规格 YAML

**Files:**
- Create: `src/features/deployments/BlueprintWizard.tsx`、`src/features/deployments/blueprint-spec.ts`（规格 ↔ YAML：`yaml` 包 `parseDocument` 保留注释；`readSpec(text) -> {spec, errors}`、`writeSpec(doc, patch) -> text`）、`src/features/deployments/BlueprintWizard.test.tsx`、`src/features/deployments/blueprint-spec.test.ts`
- Modify: 路由（`src/app/App.tsx` 增加 `deploy/apps` 的 `new?wizard=blueprint&template=<id>&version=<n>` 渲染向导）、`src/features/marketplace/resource-config.tsx`（市场条目「部署」动作：蓝图条目先导入（已有 install）再进入向导；非蓝图条目沿用现有「使用模板」）、`src/features/templates/resource-config.tsx`（应用模板详情增加「部署」）、i18n `src/shared/i18n/locales/resource-domain.json` 新增 `wizard.*` 六语

**Interfaces:**
- 步骤（顶部步骤条；Phone 下为纵向单列 + 底部固定操作栏）：① 模板与版本（只读，显示角色、驱动） ② 参数（复用 `templateParameterFields`） ③ 角色与主机（每角色实例数步进器带 min/max/odd 提示；主机选择：标签多选（`tagOptions`）、主机名多选、主机组单选、分散前缀；Swarm 选管理节点） ④ 放置预览（调用 `deployment-placements`，表格：实例 → 主机 → 地址；可逐实例改主机/地址，改动写入规格 `placement`；显示 warnings） ⑤ 确认并创建（名称、工作目录；提交 `deploy_spec`）。
- 任意步骤可切换「YAML」：Monaco（`CodeEditor` language yaml）编辑整份规格，与表单双向同步；解析错误时显示错误、阻止下一步与提交，切回表单不丢失未出错部分。
- [ ] 失败测试：`spec_roundtrip_preserves_comments`、`instances_stepper_respects_odd_and_bounds`、`yaml_errors_block_submit_and_keep_form`、`placement_preview_renders_rows_and_warnings`、`submits_deploy_spec`、`market_blueprint_entry_opens_wizard`。
- [ ] FAIL → 实现 → `npx vitest run src/features/deployments src/features/marketplace` PASS；`npm run build` PASS → Commit `feat(deployments): blueprint deployment wizard with YAML spec`

### Task 8: 详情页拓扑与编辑

**Files:**
- Create: `src/features/deployments/BlueprintTopology.tsx`、`.test.tsx`
- Modify: `src/features/deployments/DeploymentDetail.tsx`（蓝图部署显示拓扑：角色 → 实例 → 主机名 → 健康（来自 `application-health`）；规格查看/编辑入口：首次部署前可改全部，之后仅 `values`）

- [ ] 失败测试：`topology_groups_instances_by_role_with_host_names`、`health_badges_from_application_health`、`spec_edit_limited_to_values_after_deploy`。
- [ ] FAIL → 实现 → vitest PASS；`npm run build` → Commit `feat(deployments): blueprint topology and spec editing on detail page`

### Task 9: 真实验收与文档

**Files:**
- Create: `scripts/test-blueprint-wizard-ui.mjs`（Playwright：给 3 台测试主机打标签 → 市场选 nats-cluster → 向导各步 → YAML 往返 → 放置预览 → 创建；1440/900/390/320 截图，无横向滚动，320 下底部操作可达）
- Modify: `scripts/test-deployments.py`（`FERRY_TEST_BLUEPRINT=1` docker 场景改为通过 `deploy_spec` + 标签选择器创建 nats/nginx，断言放置结果与标签一致；stack 场景同样用标签）、`.github/workflows/build.yml`（加入向导 UI 脚本）、`ADR.md`（P2c 决策）、设计文档 §7 标注已实现部分。
- [ ] 本地：`python -m py_compile scripts/test-deployments.py`；`cargo test -p ferry-server`；`npm test`；`npm run build`；向导 UI 脚本本地跑通（不需要 Docker 的部分）并保存截图到 `data/maintenance/orchestration-p2c/`。
- [ ] CI：推送前确认；通过 `ci/scenario-templates-*` 分支触发，读取公开注解结果。
- [ ] Commit `test(deployments): wizard UI and tag-selector acceptance`

## 不在本计划

- `failover`、`backup`、`storage` 字段与协调器（P3/P4）。
- 拓扑变更（扩缩容/换主机，P2.5 并入 P3 的替换能力）。
- K8s 驱动。
