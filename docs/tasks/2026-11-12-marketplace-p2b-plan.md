# 集群模板 P2b：Swarm（stack 驱动）适配实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 集群蓝图可以用 `stack`（Docker Swarm）驱动部署：有状态实例各自一个 Swarm service、钉在绑定的节点上、host 模式发布端口、`dnsrr` 成员发现；无状态角色一个 service + replicas + VIP。

**Architecture:** 复用 P2a 的蓝图层与现有 stack 执行路径。绑定增加 `manager`（执行 `docker stack deploy` 的 Swarm 管理节点主机）；每个实例仍绑定一台已登记的 Ferry 主机，部署前通过远程通道读取该主机的 Swarm 节点 ID 并在管理节点上核对 `docker node ls`，放置约束使用 `node.id==<ID>`。编译结果只有一个目标（管理节点），一份 `ferry/v2 Application` 包含全部服务。ferry/v2 manifest 增加 stack 专用的端口 `mode` 与服务 `endpointMode`。

**Tech Stack:** Rust（axum、sea-orm、serde_json）、现有 stack 驱动（`docker stack deploy`）、Python CI 验收（3 节点嵌套 Swarm）。

**Spec:** `docs/应用市场与集群模板设计.md` §6.3（stack 行）、§7.2（仅 Linux）、§15.1（Swarm 节点必须登记为 Ferry 主机）。

## Global Constraints

- Swarm 节点主机仅 Linux（沿用 `blueprint.hostOsUnsupported` 规则）；管理节点主机同样适用。
- 绑定 JSON：`{"driver":"stack","manager":"<hostId>","roles":{...}}`；`manager` 仅 stack 必填，其他驱动出现即 `blueprint.bindingInvalid`。
- 有状态角色（`identity: stable`）：每实例一个 service，名称 `<role>-<index>`；`deploy.placement.constraints = ["node.id==<swarm node id>"]`；所有端口 `mode: host`；`deploy.endpoint_mode: dnsrr`；`.Host` = 绑定地址（节点可达地址），`.Port` = 渲染后的 published。
- 无状态角色（`identity: stateless`）：一个 service，名称 `<role>`，`replicas` = 绑定实例数，端口 ingress（VIP）；`.Host` = `<role>`（service DNS 名），`.Port` = container；实例绑定的主机只用于校验节点归属，不做放置约束（Swarm 自行调度）。
- 节点核对：管理节点 `docker info --format '{{.Swarm.ControlAvailable}}'` 必须为 `true`（否则 `blueprint.swarmManagerRequired`）；每个实例主机 `docker info --format '{{.Swarm.NodeID}}'` 非空且出现在管理节点 `docker node ls` 中、`Status=Ready`、`Availability=Active`（否则 `blueprint.swarmNodeUnavailable`，参数含主机名）。
- `distinctHosts` 在 stack 下按节点 ID 判定（两台 Ferry 主机若是同一节点视为重复）。
- 仍不实现：k8s、拓扑变更（P2.5）、Windows/macOS。
- 所有新文案六语（zh-CN/zh-TW/en-US/ms-MY/ja-JP/ko-KR）。
- 编译测试用 `CARGO_TARGET_DIR=D:/Projects/github/ferry-marketplace-p1/target`。

## Review Focus

1. 绑定的某台主机不在该 Swarm 中（或节点 Down/Drain）→ 预览前明确拒绝并指出主机 → Task 3 `bound_host_outside_swarm_rejected`。
2. 两台 Ferry 主机实际是同一个 Swarm 节点 + `distinctHosts` → 拒绝 → Task 3 `same_node_twice_violates_distinct_hosts`。
3. 管理节点主机不是 manager（worker 或未加入 Swarm）→ 拒绝 → Task 3 `manager_must_control_swarm`。
4. 有状态实例在 host 模式下，同一节点上两个实例发布同一端口 → 编译拒绝（复用 `blueprint.portConflict`，按节点归组）→ Task 4 `host_mode_port_conflict_on_same_node`。
5. 非 stack 驱动的 manifest 使用 `ports[].mode` 或 `endpointMode` → 拒绝 → Task 1 `stack_only_fields_rejected_elsewhere`。

---

### Task 1: manifest 支持 host 端口与 dnsrr

**Files:**
- Modify: `crates/server/src/deployments/manifest.rs`（`Port` ~321–329；`Service` ~28–54；`render_for` ~565–794 端口输出 ~677–690、deploy 输出 ~605–612；`validate_compose` ~924–1270 端口键 ~1159–1163、deploy 键 ~1040–1047）
- Modify: `crates/server/src/blueprint/compile.rs`（类型表 `SERVICE` 增加 `endpointMode` 为字符串）

**Interfaces:**
- `Port` 新字段 `mode: Option<String>`（JSON `mode`，取值 `host|ingress`）；`Service` 新字段 `endpoint_mode: Option<String>`（JSON `endpointMode`，取值 `vip|dnsrr`）。
- 仅 `apiVersion ferry/v2` 且 mode 为 `stack` 时允许；渲染为 compose `ports[].mode` 与 `deploy.endpoint_mode`；`validate_compose` 在 stack 下放行这两个键（overrides 同样受限）。
- `endpoint_mode: dnsrr` 时该服务的端口必须全部为 `mode: host`（Swarm 不允许 dnsrr + ingress 发布），否则 `deployment.portable.endpointModeInvalid`（新键，六语）。

- [ ] **Step 1: 失败测试**（manifest.rs tests）：`stack_renders_host_ports_and_dnsrr`（输出含 `"mode":"host"` 与 `deploy.endpoint_mode = "dnsrr"`）、`stack_only_fields_rejected_elsewhere`（docker/compose/native 中出现 `mode` 或 `endpointMode` → Err）、`dnsrr_requires_host_ports`、`invalid_port_mode_rejected`。
- [ ] **Step 2:** `cargo test -p ferry-server deployments::manifest` → FAIL。
- [ ] **Step 3:** 实现。
- [ ] **Step 4:** PASS；`cargo test -p ferry-server -- deployments blueprint` PASS。
- [ ] **Step 5:** Commit `feat(deployments): stack host-mode ports and dnsrr endpoint mode`

### Task 2: 绑定支持 stack 与管理节点

**Files:**
- Modify: `crates/server/src/blueprint/binding.rs`

**Interfaces:**
```rust
pub struct Binding { pub driver: String, pub manager: Option<Id>, pub roles: BTreeMap<String, Vec<InstanceBinding>> }
// validate(bp, b, hosts) 规则变化：
//   driver == "stack" 不再报 driverNotYetSupported；需 bp.drivers.stack.supported
//   stack ⇒ manager 必填且在 hosts 中、Linux；非 stack ⇒ manager 必须为 None
//   stack 下 distinctHosts 仍先按 host_id 判重（节点级判重在 Task 3）
```
- [ ] **Step 1: 失败测试**：`stack_binding_requires_manager`、`manager_only_for_stack`、`stack_manager_must_be_linux_host`、`stack_accepted_when_blueprint_supports_it`（原 `stack_and_k8s_not_available_yet` 改为只断言 k8s）。
- [ ] **Step 2:** FAIL。 **Step 3:** 实现（序列化保持 camelCase、`manager` 缺省时不输出，保证已存储的非 stack 绑定字符串不变）。 **Step 4:** `cargo test -p ferry-server blueprint::binding` PASS。
- [ ] **Step 5:** Commit `feat(blueprint): stack bindings name a swarm manager host`

### Task 3: Swarm 节点核对

**Files:**
- Create: `crates/server/src/blueprint/swarm.rs`
- Modify: `crates/server/src/hosts/fake.rs`（测试替身支持按命令返回预设输出）

**Interfaces:**
```rust
pub struct Node { pub id: String, pub hostname: String, pub ready: bool, pub active: bool }
pub async fn manager_nodes(ch: &dyn HostChannel) -> Result<Vec<Node>>;   // ControlAvailable 检查 + docker node ls --format '{{json .}}'
pub async fn node_id(ch: &dyn HostChannel) -> Result<String>;            // docker info --format '{{.Swarm.NodeID}}'，空 ⇒ swarmNodeUnavailable
/// 返回实例名 → 节点 ID；检查每个实例主机的节点在列表中且 Ready/Active，并按节点 ID 复核 distinctHosts。
pub fn assign(bp: &Blueprint, b: &Binding, nodes: &[Node], host_nodes: &BTreeMap<Id, String>, host_names: &BTreeMap<Id, String>) -> Result<BTreeMap<String, String>>;
```
- 远程命令沿用部署代码的 shell 约定（POSIX sh、`docker` CLI）；输出解析失败 ⇒ `blueprint.swarmNodeUnavailable`。
- [ ] **Step 1: 失败测试**：`manager_must_control_swarm`、`bound_host_outside_swarm_rejected`、`drained_or_down_node_rejected`、`same_node_twice_violates_distinct_hosts`、`assign_maps_instances_to_node_ids`、`node_ls_json_parsed`（真实 `docker node ls --format '{{json .}}'` 样例行）。
- [ ] **Step 2:** FAIL。 **Step 3:** 实现。 **Step 4:** `cargo test -p ferry-server blueprint::swarm` PASS。
- [ ] **Step 5:** Commit `feat(blueprint): resolve swarm node ids for stack bindings`

### Task 4: stack 驱动编译

**Files:**
- Modify: `crates/server/src/blueprint/compile.rs`

**Interfaces:**
```rust
// compile 签名扩展（P2a 调用点同步改）：
pub fn compile(bp_text: &str, binding: &Binding, nodes: Option<&BTreeMap<String, String>>) -> Result<Vec<HostPlan>>;
// stack ⇒ nodes 必填；返回唯一 HostPlan，host_id = binding.manager
```
- stable 角色按全局约束生成每实例 service（`placement.constraints`、端口 `mode: host`、`endpointMode: dnsrr`）；stateless 角色生成一个 `<role>` service（`replicas`、无 placement、ingress 端口）。
- 端口冲突：stable 实例按节点 ID 归组检查 `(published, protocol)`；stateless 的 ingress 端口在整个 stack 内唯一。
- contexts 键：stable 为实例名；stateless 为 `<role>`（所有副本共用同一渲染上下文，`.Instance.Index` = 0，`.Instance.Name` = `<role>`）。
- [ ] **Step 1: 失败测试**：`stack_stable_role_one_service_per_instance_pinned_to_node`、`stack_stateless_role_single_service_with_replicas`、`stack_members_resolve_host_and_port`（NATS routes 用绑定地址 + host 端口；无状态成员 Host = `<role>`）、`host_mode_port_conflict_on_same_node`、`stack_manifest_passes_driver_validation`、`stack_requires_nodes`。
- [ ] **Step 2:** FAIL。 **Step 3:** 实现。 **Step 4:** `cargo test -p ferry-server -- blueprint deployments` PASS。
- [ ] **Step 5:** Commit `feat(blueprint): compile stack blueprints into pinned swarm services`

### Task 5: 部署接入 stack 蓝图

**Files:**
- Modify: `crates/server/src/deployments/groups.rs`（`compile`/`targets`：stack 时打开管理节点与各实例主机通道，调用 Task 3 得到节点映射后编译）
- Modify: `crates/server/src/deployments/mod.rs`（`check_blueprint`：stack 允许；创建时只做离线校验（Task 2），节点核对放到预览，因为创建时主机可能离线）
- Modify: `crates/server/src/deployments/blueprint_tests.rs`

**Interfaces:**
- `targets()` 对 stack 返回 1 个 `HostTarget`（管理节点），`contexts` 为 Task 4 的键。
- 预览/执行沿用现有 stack 执行（`modes::stack_execute`），每主机文件隔离逻辑对单目标天然成立。
- 回滚/重启重放存储的目标，不重新核对节点。
- [ ] **Step 1: 失败测试**（fake 通道模拟 manager 与 3 个 worker 的 docker 输出）：`stack_blueprint_preview_single_manager_target`、`stack_blueprint_rejects_host_outside_swarm_at_preview`、`stack_blueprint_create_offline_then_preview_validates_nodes`、`stack_blueprint_rollback_replays_without_node_lookup`。
- [ ] **Step 2:** FAIL。 **Step 3:** 实现。 **Step 4:** 全量 `cargo test -p ferry-server` PASS。
- [ ] **Step 5:** Commit `feat(deployments): stack blueprint deployments through the swarm manager`

### Task 6: 官方模板与 Swarm 真实验收

**Files:**
- Modify: `marketplace/apps/nats-cluster/1.0.0/blueprint.yaml`、`marketplace/apps/nginx-ha/1.0.0/blueprint.yaml`（`drivers.stack.supported: true`；确认端口与地址语义在 host 模式下成立）、重新生成 `marketplace/index.json`
- Modify: `crates/server/src/templates/official_blueprints_tests.rs`（加入 stack 编译断言）
- Modify: `scripts/test-deployments.py`（`FERRY_TEST_BLUEPRINT=1` 且 mode `stack`：3 个嵌套 Swarm 节点同时登记为 Ferry SSH 主机，manager 为节点 0；部署 nats-cluster（3 实例分别钉在 3 节点）与 nginx-ha；断言 `/routez` 2 路由、JetStream 集群 3、nginx 负载均衡；改参数、回滚）
- Modify: `.github/workflows/deployment-ha.yml`（新增 stack 蓝图矩阵项，产物名唯一）
- 不新增 `verified.yaml` 的 stack 记录，直到 CI 通过。

- [ ] **Step 1:** `node scripts/build-marketplace-index.mjs` 与 `--check`；`cargo test -p ferry-server official_blueprints` PASS（含 stack）。
- [ ] **Step 2:** `python -m py_compile scripts/test-deployments.py`；`cargo fmt --all -- --check`；全量 `cargo test -p ferry-server`、`npm test`。
- [ ] **Step 3:** 推送后由 CI 运行 stack 蓝图项（推送前确认）；从公开注解读取结果，记入 `data/maintenance/marketplace-p2b/`。
- [ ] **Step 4:** 通过后追加 `verified.yaml` 的 stack 记录；ADR 追加 P2b 决策；MEMORY 更新。
- [ ] **Step 5:** Commit `feat(marketplace): official blueprints on swarm with acceptance`

## 不在本计划

- P2c：实例绑定界面（包含 stack 的管理节点选择）。
- P2.5：拓扑变更。
- k8s、Windows/macOS 驱动。
