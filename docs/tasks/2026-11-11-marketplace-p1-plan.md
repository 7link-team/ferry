# 应用市场 P1：平台来源与索引 v2 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 超级管理员可在平台设置添加多个公开 Git 模板来源，所有工作空间的应用市场合并显示「官方/社区/本空间」条目，并可按版本导入（连同附带配置文件模板）；空间来源与 v1 索引保持兼容。

**Architecture:** 复用 `marketplace_sources`/`marketplace_entries` 两表（平台来源 `workspace_id=0`），新增纯函数索引解析模块（v1+v2），平台来源 CRUD/同步走独立的 Admin 接口与租约式后台调度，空间市场查询合并 `workspace_id IN (当前, 0)` 且来源启用的条目，导入在一个事务里创建应用模板与配置模板并记录来源。

**Tech Stack:** Rust（axum、sea-orm、semver、sha2）、React + 共享 ResourceTable/ResourceForm、vitest、Python/Node 真实验收脚本。

**Spec:** `docs/应用市场与集群模板设计.md`（本计划覆盖 §5、§8、§9.1–9.3、§10 中与平台来源/导入相关部分；§6–7 蓝图与拓扑变更属 P2/P2.5，另写计划）。

## Global Constraints

- 平台来源只读公开仓库：不绑定代码连接/凭据，请求不带 token；仅超级管理员（`accounts::platform::Admin`）。
- 平台来源 `trust` 只能为 `official` 或 `community`；空间来源固定 `workspace`。
- 同步间隔 1–168 小时，默认 6 小时，存秒。
- bundle 上限 8MiB；包 ≤500；每包版本 ≤20；每版本文件 ≤20；单文件 ≤256KiB；icon ≤64KiB；readme ≤64KiB。
- 包 key：`[A-Za-z0-9._-]{1,100}`；版本：semver；`kind` 本期仅 `app` 可导入。
- 未知 `schemaVersion`→整批失败保留旧目录；未知 `kind` 或 `requires.ferry` 不满足→该版本 `compatible=0` 入库、不可导入。
- 名称多语言回退：请求语言 → en-US → zh-CN → key。
- 迁移只 ADD COLUMN（带默认值）；down 删除新增列（与 `identity_avatars` 同风格），不改写已有数据。
- 新文案进 zh-CN/en-US/ms-MY/ja-JP/ko-KR/zh-TW 六份 locale（前端 `src/shared/i18n/locales/*.json` 与后端 `crates/server/src/i18n/*-<locale>.json`）。
- 全平台：服务端/前端无 OS 相关代码；验收脚本在 Windows（Git Bash/PowerShell）与 Linux CI 都能跑（路径用 `pathlib`/`path`，不写死 `/tmp`）。
- 响应式：平台来源页、市场页 1440/900/390/320 无整页横向滚动。

## Review Focus

1. 平台来源被停用/删除时，空间市场里该来源条目立即不可见、不可导入，已导入副本不变 → Task 6 测试 `disabled_platform_source_hidden_and_install_rejected`。
2. 同一包 key 在平台来源与空间来源同时存在 → 两条都显示、按来源区分，导入互不影响 → Task 6 测试 `same_key_in_platform_and_workspace_sources_are_distinct`。
3. 导入时配置模板名与空间已有模板重名 → 追加 `-2`、`-3`… 且 app 模板中 `fromTemplate` 改写为新名称 → Task 6 测试 `install_renames_conflicting_config_templates`。
4. 两个服务实例同时到期调度同一平台来源 → 只有一个实例真正请求上游 → Task 5 测试 `lease_claim_is_exclusive`。
5. 上游返回旧 v1 索引（现有空间来源）→ 同步与导入行为与改造前一致 → Task 3 现有 `scripts/test-marketplace.py` 全量通过 + 单元测试 `v1_index_maps_to_single_version_package`。

---

### Task 1: 迁移与实体字段

**Files:**
- Create: `crates/server/src/migration/marketplace_platform.rs`
- Modify: `crates/server/src/migration.rs`（`mod marketplace_platform;` 与迁移列表末尾 `Box::new(marketplace_platform::Migration)`，紧随 `identity_avatars`）
- Modify: `crates/server/src/entities.rs`（`marketplace_sources`、`marketplace_entries`、`app_templates`、`config_templates` 的 Model 增加字段）
- Modify: 所有构造上述 ActiveModel 的位置补新字段默认值（`marketplace.rs`、`templates.rs` 宏与 seed、测试夹具；用 `cargo check` 找全）

**Interfaces:**
- Produces（列名即 Rust 字段名）：
  - `marketplace_sources`: `trust: String`('workspace')、`enabled: i64`(1)、`sync_interval: i64`(0)、`next_sync_at: i64`(0)、`sync_owner: String`('')、`sync_lease_until: i64`(0)、`created_by: i64`(0)、`updated_by: i64`(0)、`synced_by: i64`(0)
  - `marketplace_entries`: `package_version: String`('0.0.0')、`kind: String`('app')、`meta: String`('{}')、`files: String`('[]')、`readme: String`('')、`compatible: i64`(1)、`digest: String`('')、`created_by: i64`(0)、`updated_by: i64`(0)
  - `app_templates`、`config_templates`: `origin_source_id: i64`(0)、`origin_key: String`('')、`origin_version: String`('')、`origin_digest: String`('')
  - 索引：`idx_marketplace_entries_source (workspace_id, source_id)`、`idx_marketplace_entries_key (workspace_id, entry_key, package_version)`、`idx_marketplace_sources_due (workspace_id, enabled, next_sync_at)`

- [ ] **Step 1: 写失败测试** `marketplace_platform_migration_roundtrip_preserves_rows`（放在新迁移文件 `#[cfg(test)]`，夹具用 `crate::agents::tests::setup()`）：down→up 后已有用户数不变；插入一条 `workspace_id=0, trust="official"` 的来源后能读回 `trust=="official"`、`enabled==1`。
- [ ] **Step 2:** `cargo test -p ferry-server marketplace_platform_migration` → FAIL（模块不存在）。
- [ ] **Step 3:** 实现迁移：文本列用迁移里现有 `long_text` 帮助函数风格（短 VARCHAR 用 `.string_len(n)`），所有列 `NOT NULL DEFAULT`；down 逐列 drop。更新实体与所有 ActiveModel 构造点。
- [ ] **Step 4:** `cargo test -p ferry-server marketplace_platform_migration` → PASS；`cargo test -p ferry-server templates marketplace migration` → 全 PASS。
- [ ] **Step 5:** Commit `feat(marketplace): add platform source and package version columns`

### Task 2: 索引解析模块（v1 + v2，纯函数）

**Files:**
- Create: `crates/server/src/marketplace/index.rs`（把 `marketplace.rs` 改为 `marketplace/mod.rs` 目录模块；保持 `crate::marketplace::*` 对外路径不变）
- Test: 同文件 `#[cfg(test)] mod tests`

**Interfaces:**
- Produces:
```rust
pub struct PackageVersion {
    pub key: String, pub kind: String, pub version: String,      // semver 文本
    pub name: String, pub description: String,                    // 已按 locale 回退选定
    pub meta: serde_json::Value,                                  // package.json + icon + verified + 原始多语言 name/description
    pub content: serde_json::Value, pub parameters: serde_json::Value,
    pub files: Vec<PackageFile>, pub readme: String,
    pub compatible: bool, pub incompatible_reason: Option<String>, // i18n key，如 "marketplace.kindUnsupported"
    pub digest: String,                                           // hex sha256
}
pub struct PackageFile { pub name: String, pub content: String, pub parameters: serde_json::Value }
pub fn parse_bundle(text: &str, ferry_version: &semver::Version, locale: &str) -> Result<Vec<PackageVersion>>;
```
- v1：`{version:1,templates:[...]}` → 每模板一条 `kind="app"`、`version="0.0.0"`、`files=[]`、`meta={}`，沿用现有 `parse_index` 全部校验（把现有函数移入本文件复用）。
- v2：`{schemaVersion:2,packages:[...]}`，字段见设计 §5.2/§5.3；app 版本的 `blueprint` 进 `content`，按现有 `crate::templates::validate_app` 校验（P1 只接受 `kind: Application`；`kind: Blueprint` 记为 `compatible=false, reason="marketplace.blueprintRequiresUpgrade"`，P2 放开）。
- digest = sha256(`serde_json::to_string(&json!({"content":..,"parameters":..,"files":..}))`)。

- [ ] **Step 1: 写失败测试**：
  - `v1_index_maps_to_single_version_package`：现有 fixture（`scripts/test-marketplace.py` 第 18 行同款）→ 1 条，`version=="0.0.0"`、`kind=="app"`、`compatible`。
  - `v2_bundle_parses_versions_files_and_locale_fallback`：一个包两版本、一个文件，`locale="ms-MY"` 时 name 取 en-US 值。
  - `v2_unknown_schema_version_rejected`：`schemaVersion:3` → Err。
  - `v2_unknown_kind_or_newer_ferry_marked_incompatible`：`kind:"pipeline"` 与 `requires.ferry:">=99.0.0"` 各得 `compatible=false` 且 reason 非空，不报错。
  - `v2_limits_enforced`：21 个版本 / 21 个文件 / 文件 256KiB+1 / 非法 key `a b` / 非 semver 版本 → 各自 Err。
  - `digest_is_stable`：同输入两次解析 digest 相等，改一个字节不等。
- [ ] **Step 2:** `cargo test -p ferry-server marketplace::index` → FAIL。
- [ ] **Step 3:** 实现 `parse_bundle`；`MAX_INDEX_BYTES` 改为 8MiB（`fetch` 中 `*2` 响应上限随之变为 16MiB，保持现状比例）。
- [ ] **Step 4:** `cargo test -p ferry-server marketplace` → 全 PASS（含原 `marketplace.rs` 内 tests）。
- [ ] **Step 5:** Commit `feat(marketplace): parse v2 package bundles alongside v1`

### Task 3: 条目存储共用 + 空间同步改用新解析

**Files:**
- Create: `crates/server/src/marketplace/store.rs`
- Modify: `crates/server/src/marketplace/mod.rs`（`synchronize` 中 `parse_index` + 插入循环替换为 `parse_bundle` + `store::replace_entries`）

**Interfaces:**
- Consumes: Task 2 `parse_bundle`, `PackageVersion`
- Produces:
```rust
pub(crate) async fn replace_entries(
    s: &AppState, tx: &sea_orm::DatabaseTransaction,
    workspace_id: i64, source_id: i64, actor: i64,
    packages: Vec<PackageVersion>, deadline: std::time::Instant,
) -> Result<usize>;   // 删除该来源旧条目后插入；返回条数；超时返回 marketplace.syncTimeout
```
- 空间同步 locale：用 `scope` 请求语言（沿用 `crate::i18n` 当前 locale 获取方式）；平台同步 locale 固定存原始多语言于 `meta`，`name/description` 存 en-US 回退值，列表接口按请求语言从 `meta` 重新选（Task 6）。

- [ ] **Step 1: 写失败测试** `workspace_sync_accepts_v2_bundle`（复用 `marketplace.rs` 现有 `real_http_transport...` 的本地 HTTP 夹具风格）：v2 bundle 同步后条目数 = 版本总数，`package_version`、`files`、`digest` 落库。
- [ ] **Step 2:** 运行 → FAIL。
- [ ] **Step 3:** 实现 `replace_entries`，`synchronize` 改调用。
- [ ] **Step 4:** `cargo test -p ferry-server marketplace` PASS；`python scripts/test-marketplace.py` 全部 PASS（v1 回归）。
- [ ] **Step 5:** Commit `refactor(marketplace): share entry storage and accept v2 in workspace sources`

### Task 4: 平台来源 CRUD 接口

**Files:**
- Create: `crates/server/src/marketplace/platform.rs`
- Modify: `crates/server/src/main.rs`（路由，放在 `/api/platform/settings` 附近）
- Modify: 后端六份 `crates/server/src/i18n/*` 新 key

**Interfaces:**
- 路由：
  - `GET/POST /api/platform/marketplace-sources`
  - `GET/PATCH/DELETE /api/platform/marketplace-sources/{id}`
  - `POST /api/platform/marketplace-sources/{id}/synchronizations`
- 请求体：
```rust
#[derive(Deserialize)] #[serde(deny_unknown_fields)]
pub struct PlatformBody {
    name: String, trust: String, enabled: bool, sync_interval_hours: i64,
    #[serde(flatten)] source: Source,   // provider/api_base/project/organization/branch/index_path，复用现有 Source::url() 校验
    #[serde(default)] version: i64,
}
```
- 响应 JSON：`source_json` 同形 + `trust`、`enabled`、`sync_interval_hours`、`next_sync_at`、`entry_count`。
- 校验：name 1–60；trust ∈ {official, community}；sync_interval_hours 1–168；`api_base` 必须 https（平台来源无仓库同源校验，改为 https 强制）。
- 审计：`crate::accounts::platform::audit(&s, &tx, &admin.0.user, action, "marketplace-sources", id, details)`。
- 删除：同步中（租约未过期）拒绝 `marketplace.waitForSync`；删其条目；已导入副本保留。
- 列表用 `resource::global_list` 并额外 `.filter(WorkspaceId.eq(0))`。

- [ ] **Step 1: 写失败测试**（HTTP 级，放 `platform.rs` tests，用现有 axum 测试夹具）：
  - `platform_sources_require_superadmin`：普通用户 403/401，超管 201。
  - `platform_source_validation`：trust=`workspace`、interval 0/169、http api_base → 400。
  - `platform_source_update_version_conflict`：旧 version PATCH → 409。
  - `platform_sources_not_listed_in_workspace_sources`：空间 `GET /api/workspaces/{w}/marketplace-sources` 不含平台来源。
- [ ] **Step 2:** 运行 → FAIL。
- [ ] **Step 3:** 实现。
- [ ] **Step 4:** 测试 PASS。
- [ ] **Step 5:** Commit `feat(marketplace): platform-level public template sources API`

### Task 5: 平台同步（租约 + 后台调度）

**Files:**
- Modify: `crates/server/src/marketplace/platform.rs`
- Modify: `crates/server/src/marketplace/mod.rs`（`recover` 只处理 `workspace_id != 0`）
- Modify: `crates/server/src/main.rs`（`tokio::spawn(marketplace::platform::scheduler(s.clone(), drain_rx.clone()))`，与 `cron_task` 并列并在退出时 await）

**Interfaces:**
```rust
const LEASE_MS: i64 = 60_000;  // 同步超时 20s 的 3 倍
pub(crate) async fn claim(s: &AppState, id: i64) -> Result<bool>;
// UPDATE marketplace_sources SET sync_owner=?, sync_lease_until=now+LEASE_MS, state='syncing'
// WHERE id=? AND workspace_id=0 AND enabled=1 AND sync_lease_until<=now ; rows_affected==1
pub(crate) async fn synchronize_platform(s: &AppState, id: i64, actor: i64) -> Result<usize>;
// claim → fetch(token=None) → parse_bundle → tx: replace_entries(workspace_id=0) + state idle, synced_at, synced_by, next_sync_at=now+interval, 清租约 → commit
// 失败：state failed, last_error, next_sync_at=now+interval, 清租约；旧条目保留
pub async fn scheduler(s: AppState, drain: tokio::sync::watch::Receiver<bool>);
// 每 60s：查 workspace_id=0 AND enabled=1 AND next_sync_at<=now，逐个 synchronize_platform(actor=0)
```
- 手动同步接口调用 `synchronize_platform(actor=admin id)`；未拿到租约返回 409 `marketplace.syncing`。
- 新建来源 `next_sync_at = 0`（下一轮立即同步）。

- [ ] **Step 1: 写失败测试**：
  - `lease_claim_is_exclusive`：同一 id 连续两次 `claim` → `true, false`；把 `sync_lease_until` 改为过去 → 再次 `true`。
  - `platform_sync_success_and_failure_keep_previous_entries`：本地 HTTP 夹具先返回有效 bundle（条目 N）→ 再返回 500 → 条目仍 N、state=`failed`、`last_error` 非空、`next_sync_at>now`。
  - `recover_ignores_platform_rows`：平台来源 state=`syncing` 且租约未过期，调用 `recover` 后仍为 `syncing`。
- [ ] **Step 2:** 运行 → FAIL。
- [ ] **Step 3:** 实现。
- [ ] **Step 4:** PASS；`cargo test -p ferry-server marketplace` 全 PASS。
- [ ] **Step 5:** Commit `feat(marketplace): lease-based platform source synchronization`

### Task 6: 空间市场合并列表与导入

**Files:**
- Modify: `crates/server/src/marketplace/mod.rs`（`list`、`get`、`install`、`entry_json`、`ENTRIES` spec）
- Modify: 后端 i18n

**Interfaces:**
- 列表/详情查询条件：`(workspace_id = 当前) OR (workspace_id = 0 AND source_id IN (SELECT id FROM marketplace_sources WHERE workspace_id=0 AND enabled=1))`。不能用 `resource::list`（会强加 workspace 过滤）：在 `resources/resource.rs` 新增
```rust
pub async fn list_with<E: EntityTrait, M: FromQueryResult + Send + Sync>(
    conn: &impl ConnectionTrait, spec: &Spec, select: Select<E>, scope_condition: sea_orm::Condition, query: &ListQuery,
) -> Result<(Vec<M>, u64)>;   // 与 list 相同，只是工作空间条件由调用方给
```
- `entry_json` 增加：`source_name`、`trust`（来源的）、`package_version`、`kind`、`compatible`、`incompatible_reason`（已本地化）、`meta`、`readme`、`files`（仅名称列表）；`name/description` 按请求语言从 `meta` 回退选取。
- `ENTRIES.filters` 增加 `trust`、`package_version`；`trust` 过滤在 SQL 层按来源 id 集合实现。
- 导入 `POST /api/workspaces/{w}/marketplace/{id}/installations`，body 不变 `{version}`（行乐观锁版本）：
  - 权限：`marketplace.install`、`app-templates.create`，有 files 时加 `config-templates.create`。
  - `compatible=0` → 400 `marketplace.incompatible`。
  - 一个 `variables::transaction`：逐个文件调用 `templates::configs::create_in`（名称冲突时尝试 `name`、`name-2`…`name-20`，仍冲突报 409 `marketplace.configNameExhausted`），建立 `原名→新名` 映射，改写 content 中所有 `services[].files[].fromTemplate` 后调用 `templates::apps::create_in`；之后 UPDATE 两类模板的 `origin_*` 字段（`origin_source_id`=来源 id，`origin_key`、`origin_version`、`origin_digest`）。
  - 审计沿用 `create_in` 内部 activity，另记一条 `install` activity：`{"source_id","key","version"}`。

- [ ] **Step 1: 写失败测试**：
  - `workspace_market_lists_platform_and_own_entries`：平台来源 1 条 + 本空间 1 条 → 列表 total=2，trust 分别为 `official`/`workspace`；另一空间看到平台 1 条、看不到本空间那条。
  - `disabled_platform_source_hidden_and_install_rejected`：停用后列表不含、`GET /marketplace/{id}` 404、导入 404；已导入模板仍在。
  - `same_key_in_platform_and_workspace_sources_are_distinct`：两来源同 key → 两条，各自导入成功生成两个模板。
  - `install_renames_conflicting_config_templates`：空间已有 `redis-start` 配置模板 → 导入后新建 `redis-start-2`，app 模板 content 中 `fromTemplate=="redis-start-2"`，`origin_key`/`origin_version` 正确。
  - `install_incompatible_entry_rejected`：compatible=0 → 400。
  - `install_requires_config_create_permission_when_files_present`：仅有 marketplace.install + app-templates.create 的角色导入带文件包 → 403，且无任何模板被创建（事务回滚）。
  - `list_name_follows_request_locale`：Accept-Language ms-MY → en-US 名称。
- [ ] **Step 2:** 运行 → FAIL。
- [ ] **Step 3:** 实现。
- [ ] **Step 4:** PASS；`python scripts/test-marketplace.py` PASS。
- [ ] **Step 5:** Commit `feat(marketplace): merge platform entries into workspace market and import packages with config files`

### Task 7: 前端：平台来源页 + 市场列表增强

**Files:**
- Create: `src/features/marketplace/platform-sources.tsx`（平台来源 list/form 配置，导出 `platformMarketplaceList`、`platformMarketplaceForm`）
- Modify: `src/app/Platform.tsx`（`labels` 增加 `"marketplace-sources": "resource-domain:platform.marketplaceSources"`，`lists`/`forms` 注册，`permissions` 加 `"marketplace-sources.create","marketplace-sources.update","marketplace-sources.delete","marketplace-sources.sync"`）
- Modify: `src/app/router.ts:66`（平台 item 白名单加 `"marketplace-sources"`）
- Modify: `src/features/marketplace/resource-config.tsx`（市场列表：新增列「来源」(`source_name` + 标识徽标：官方/社区/本空间)、「版本」(`package_version`)；筛选加 `trust` 下拉；导入动作在 `!r.compatible` 时 `disabled` + `disabledReason: r => r.incompatible_reason`；详情页增加只读字段 readme（`textarea`）与附带文件名列表）
- Modify: `src/shared/i18n/locales/resource-domain.json` 及其余 locale 文件（项目现有前端 locale 组织方式）
- Test: `src/features/marketplace/platform-sources.test.tsx`、`src/app/router.test.ts`

**Interfaces:**
- 平台来源表单字段：name(text,必填)、trust(select official/community,必填)、enabled(switch)、sync_interval_hours(number 1–168,默认6)、provider/api_base/project/organization(codeup 时显示)/branch/index_path（复用 `marketProviders` 与现有 help key）。
- 列表列：名称、标识、平台、状态（idle/syncing/failed 复用 `resource-domain:market.*`）、条目数、上次成功、同步错误；动作：查看、编辑、立即同步（`action: r => api(\`/platform/marketplace-sources/${encodeURIComponent(r.id)}/synchronizations\`, {method:"POST", body:"{}", headers:{"Content-Type":"application/json"}})`，`api` 来自 `src/shared/api/api.ts`；`disabled: r => r.state==="syncing"`）、删除（确认）。

- [ ] **Step 1: 写失败测试**：
  - router：`parse('/platform/marketplace-sources')` → `{workspace:"_platform", item:"marketplace-sources", mode:"list"}`；`/platform/marketplace-sources/123/edit` → edit。
  - `platform-sources.test.tsx`：渲染列表配置，syncing 行「立即同步」禁用；表单 provider 选 codeup 才出现组织字段；interval 0 与 169 提交被校验拦截。
  - 市场列表配置测试：`compatible:false` 行导入动作禁用且原因文本显示；trust 徽标三种文案。
- [ ] **Step 2:** `npx vitest run src/features/marketplace src/app/router.test.ts` → FAIL。
- [ ] **Step 3:** 实现。
- [ ] **Step 4:** 同命令 PASS；`npm run build` 通过（含 typecheck）。
- [ ] **Step 5:** Commit `feat(marketplace): platform template sources page and source-aware market list`

### Task 8: 真实验收脚本与文档

**Files:**
- Modify: `scripts/test-marketplace.py`（新增平台段：本地 HTTP 夹具提供 v2 bundle；超管创建平台来源→手动同步→两个空间都看到→导入带文件包→停用后不可见→夹具 500 时保留旧目录；v1 原有段不改）
- Modify: `scripts/test-marketplace-ui.mjs`（平台来源页与市场页 1440/900/390/320 截图、无 `document.documentElement.scrollWidth > innerWidth`）
- Create: `marketplace/README.md` + `marketplace/apps/http-service/package.json` + `marketplace/apps/http-service/1.0.0/{blueprint.json,parameters.json,README.md}`（以现有 `templates/examples/http-service.json` 为内容）+ `scripts/build-marketplace-index.mjs`（遍历 `marketplace/apps/*/*/`，校验上限，输出 `marketplace/index.json`，Node 内置模块、跨平台路径）
- Modify: `ADR.md`（追加决策：平台来源同表 workspace_id=0、独立同步、租约、只公开仓库、索引 v2 兼容 v1）
- Modify: `MEMORY.md`（MARKET-SOURCES-65 状态与证据路径）

- [ ] **Step 1:** `node scripts/build-marketplace-index.mjs` 生成 `marketplace/index.json`；`cargo test -p ferry-server marketplace::index` 增加用例 `repository_bundle_parses` 读取该文件 → PASS。
- [ ] **Step 2:** `python scripts/test-marketplace.py` → 全 PASS，输出保存到 `data/maintenance/marketplace-p1/http.log`。
- [ ] **Step 3:** `node scripts/test-marketplace-ui.mjs` → PASS，截图保存到 `data/maintenance/marketplace-p1/`，人工查看 390/320 两张。
- [ ] **Step 4:** `cargo test -p ferry-server` 与 `npm test` 全量；失败项逐个记录，不覆盖原始日志。
- [ ] **Step 5:** Commit `test(marketplace): platform source acceptance and official repository layout`

## 不在本计划（后续计划）

- P2：Blueprint 协议、受限模板子集、驱动适配器、绑定 UI（含 §7.2 全平台规则）。
- P2.5：拓扑变更计划、滚动、hooks、回滚。
- P3：六个 HA 模板逐驱动实测；官方仓库拆分。
