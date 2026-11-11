# Architecture Decision Records

## Chat collaboration and personal activity state (2026-11-08)
- Decision: extend the existing SQL chat store with optimistic message versions, edit/retraction timestamps, explicit reply links and Human mentions. Per-user saves and monotonic read cursors are separate from shared messages. Conversation membership is checked on every interaction and attachment request; only the author can edit/retract Human messages, while active executions block these mutations. Executed prompts and native CLI history are not rewritten.
- Decision: room owners can manage Human/Agent participants. Preserve existing participants' execution bindings and history; removed participants immediately lose access. Membership concurrency uses a separate token derived from membership rows, so incoming chat messages do not invalidate the editor. A real membership change invalidates stale submissions. This supersedes fixed-at-creation membership below.
- Decision: retraction leaves a tombstone and hides attachment metadata. Retain asset references for established cleanup, but reject download URLs whose only message references have been retracted. A live shared reference remains usable.
- Implementation/validation: `conversations/interactions.rs`, `migration/chat_interactions.rs` and the existing conversation components; reproducible HTTP/browser coverage is in `scripts/test-chat.mjs`. No new messaging service, local source dependency or log-retention policy.

## Agent identity independent of execution nodes (2026-11-08)
- Decision: an Agent selects a reusable model profile/catalog model; its management form has no Runner, execution-tool or working-directory binding. New CLI conversations resolve the profile and allocate an available authorized workspace Runner and private managed directory. Immutable execution snapshots remain evidence of the actual runtime used.
- Compatibility/boundaries: migrate old Agent model settings into profiles without rewriting old native sessions. Existing conversations retain their execution snapshots. Direct API chat execution, explicit conversation model switching, automatic CLI installation, persistent memory and cross-node native resumption remain separate pending capabilities. This supersedes profile-registration-only and fixed-host creation requirements below.

## Durable chat content independent of execution records (2026-11-08)
- Decision (user-confirmed): persist human messages and assistant text for private/group conversations in ai_chat_messages using the existing SQL database. Execution events, tool diagnostics, approvals and usage stay in separate execution records. No additional database or middleware is required.
- Implementation: additive ai_chat_messages_v3 migration backfills private prompts and assistant text from legacy turns/events, preserves existing group user messages and links replies to their parent message and actual Agent. Each text event updates durable text parts and the display body within the execution lease transaction; native final replacements overwrite earlier deltas without duplicate text. Chat rendering reads messages independently of events.
- Boundaries: no automatic log retention/deletion policy introduced. File bytes remain in the existing asset store; messages store authorized attachment references. Existing native session/model bindings are unchanged. Cross-database live verification requires dedicated test databases.

## Conversation files, deletion and explicit group participation (2026-11-08)
- Decision: preserve owner-only private conversations and add rooms visible only to explicit current workspace members. Only explicitly targeted Agents execute; Agent replies never implicitly trigger other Agents. Each room Agent owns a separate frozen configuration and native session; admission and completion reuse existing turns, leases and events. Approval and cancellation belong to the triggering human.
- Decision: conversation files reuse encrypted content assets, quota, reference GC and the shared media response. Canonical metadata and authorization belong to the server; previews/downloads are conversation-scoped. Deleting an idle conversation releases records/references, never supplied source directories. Draft uploads can be deleted only before they are referenced.
- Tradeoffs/status: real local chat and private native regressions passed; user-facing preview updated. Current CLI media understanding is bounded text/metadata, not native image/audio/video comprehension. Initial room membership is fixed at creation. This decision supersedes the older private-only/text-only scope below without weakening private-room authorization.

## Reusable model profile registry (2026-11-08)
- Decision: use a small workspace-scoped ai_model_profiles registry for API/CLI profile identity, model catalogs, defaults, CLI version requirements and enabled state. Reference existing ai_providers; do not duplicate credentials, provider protocols, budget policy or API invocation/accounting. Reuse immutable definition revisions, workspace mutation locks, shared forms/lists and the existing provider editor.
- Decision: model discovery requires profile creation/update and credential read/use grants, uses the existing encrypted AI token, bounded responses and no redirects; errors never include upstream response bodies. Manual catalogs remain usable when discovery is unavailable. Registration never modifies local CLI state or requires a Runner.
- Scope: this stage is configuration and discovery. Agent profile selection/frozen execution snapshots, direct API conversation execution, managed CLI installation and node decoupling remain separate implementation stages; saving a profile does not claim those execution paths are ready.

## Model profiles and execution lifecycle (2026-11-08, user requirements/design)
- Decision: add reusable model profiles for API (OpenAI-compatible/Anthropic) and CLI (Codex/Claude Code), with discovered or explicit model catalogs. Agents select a profile/model; profile selection is per Agent/execution, never a global CLI configuration switch. Reuse existing ai_providers, encrypted credentials and accounting where applicable. Agent management separates human-created and AI-derived provenance; temporary agents require parent task/permission/lifecycle records.
- Confirmed by user: missing CLI tools install automatically into Ferry-owned directories with isolated configuration, without changing existing CLI login or cc-switch global state. API mode does not install CLI tools. Installation and dependency preparation must report readiness/failure and lock concurrent installation by version/platform.
- Decision: separate writeable directories by pipeline run/job/attempt, conversation and subagent task; preserve session state for resume. Archive validated outputs/logs before cleaning temporary runtime directories and secrets, after native process cleanup. Shared tool/dependency caches have separate reference/TTL/quota policies. Unknown execution/failed archive cannot authorize deletion. Cleanup acts only on owned managed paths and never follows links outside them.
- Reason/scope: user wants independent execution without file/configuration collisions and automatic post-execution cleanup. Existing Actions private job directories/AutoRemove are reusable foundations; current Agent directory lock and process cleanup are not a full workspace/cache garbage collector. cc-switch MIT Provider/configuration mappings are references, not a new global config authority.
- Status: model profile registration/discovery is implemented and locally verified; Agent profile integration/provenance execution, managed installation and cleanup state machine remain pending. No source-history/credential migration or production rollout authorized by this record.

## Private durable Agent conversations (2026-11-08, P2)
- Decision: reuse workspace Scope, resource DSL/AutoForm, activity audit and HostChannel; only the conversation owner can read its messages, attachments and events, including when the caller is an administrator. Agent sharing does not share conversations. Audit records contain action/turn identifiers, never private message text.
- Decision: freeze Agent revision/configuration, Runner registration hash, execution account/environment fingerprint, canonical directory and CLI executable. Recheck current membership/use grants and Agent visibility; disabled Agents refuse new conversations/turns while admitted execution retains approvals and explicit cancellation. Configuration edits do not rewrite saved conversations.
- Decision: Server owns execution independently of browser connections. Persist messages, ordered events, request decisions and native completion receipts. Workspace transactions serialize admission; renewable owner-checked SQL leases fence event writes. Runner canonical-directory kernel locks exclude simultaneous native runs; Windows Job cleanup is reused and Unix native children inherit the lock descriptor.
- Reason: native CLI history and tool side effects cannot safely be reconstructed by retrying an unknown turn. Expired execution becomes interrupted without replay; only confirmed completion advances the saved native session. Unknown/interrupted conversations cannot be reopened for execution. Additional running input is explicitly unsupported; only bounded UTF-8 text attachments are accepted.
- Alternatives/tradeoffs: no second execution service, browser-owned execution, automatic replay, shared-chat exception or source-history import. SQLite/Windows/current-user Claude is the real product acceptance target; Codex product calls require an existing valid login. Cross-database, clustered execution, Unix/service identities and desktop shell remain separate verification boundaries. No release or existing-service update implied.
- Status: implementation and local regressions passed; final browser/native acceptance evidence and remaining boundaries are recorded in MEMORY.md and the migration plan.

## Workspace Agent management and environment inspection (2026-11-08, P1)
- Decision: reuse workspace Scope/roles, resource DSL, definition revisions and activity audit. Agent use scope is owner-only or workspace members, intersected with role permissions; managers can maintain configurations without gaining access to future private conversations. Execution binding changes require host administration. Disable/archive preserves history.
- Decision: native CLI login remains on the Runner account. An independent default-false Probe capability opens a dedicated inspection session, returning only execution account, CLI path/version, parsed login state and directory availability. It never calls the model or returns raw authentication diagnostics. Old Runners fail with an upgrade message.
- Verification: Windows/SQLite real Server/Runner/Chrome management, permission boundaries, six locales, missing CLI/directory, offline and old Runner plus restart persistence PASS. MySQL/PostgreSQL and other execution environments remain unverified. P2 owns use admission, frozen execution identity, directory exclusion, conversations and durable execution events.

## Rust native Agent execution foundation (2026-11-08, P0)
- Decision: extend the existing Rust Runner session protocol with additive Agent frames and a default-false capability bit. Adapt Codex app-server and Claude bidirectional stream-json directly in Rust; use the installed CLI's current execution account and native login. Keep the wire version check unchanged.
- Reason: the user confirmed Rust for backend/execution and authorized the minimal CLI adapter. Existing Runner transport and process ownership already supply the lifecycle boundary. Go/TS sources serve as protocol and behavior references; no additional worker service is introduced.
- Alternatives: copying the Go executor adds another execution stack; unstructured shell output cannot reliably identify approvals or native completion. Neither is used.
- Scope: bounded events, explicit native completion, native resume, per-request permission decisions, cancellation/timeout and owned-process cleanup. Native account settings remain unchanged. The caller must authorize workspace/host/user/workdir/session binding and serialize conflicting runs before exposing an API.
- Tradeoffs: CLI installation/authentication detection and product persistence are P1/P2 work. Claude tool disabling is not an OS sandbox; native local permission rules remain effective. Unknown structured-input/permission requests fail closed. DecisionSent acknowledges transport to the native process only. P0 transport disconnect cancels execution; browser-independent runs need Server ownership in P2.
- Status: P0 verified on Windows; real Codex and Claude reply/resume, allow/deny, pending-approval cancellation and Runner WebSocket forwarding passed. Legacy local/cluster capability refusal, related Runner/server regressions and workspace compilation passed; raw evidence and unsupported environments are recorded in MEMORY.md. No product conversation loop or release implied.

## 工作流导航与普通/高级编辑（2026-11-04，用户确认）
- 长期使用一级“工作流”，下分“构建流水线（Actions）”与“节点工作流”；资源URL与既有权限不变，旧build书签继续映射。
- 两类编辑基于同一份定义：Actions可视化表单↔高级YAML，节点参数表单＋默认折叠高级JSON。保留未知字段与注释，不在创建时要求不可切换的编辑视图选择。
- GitHub Actions与阿里云Flow交互可融合，YAML方言与执行语义不默认等价；格式兼容需逐字段验证。
- 新建时文件选择和预览暂存于当前表单内存，首次保存才通过既有资产接口上传；不隐藏创建，不放宽归属和下载授权。失败保留文件及已创建ID/版本供重试，离开释放预览；刷新前未保存文件需重选。
- 页面保存顶部右侧，面板删除/关闭标题右侧，弹窗取消/确认底部右侧；画布右/下及连线＋保留空间语义。删除键受输入焦点、只读和弹窗保护，仍需确认，并复用撤销历史。

## Notification provider forms and independent navigation feedback (2026-11-04)
- Notification configuration follows Uptime Kuma's provider-first interaction (https://github.com/louislam/uptime-kuma/tree/master/src/components/notifications), while retaining automatic URL detection and legacy channel records. The implementation is independent; no Kuma runtime or source code is bundled.
- Use official endpoint terminology and protocols: WeCom/DingTalk Markdown links, Feishu/Lark rich-text post links, Telegram HTML, Slack/Google Chat native links, Discord Markdown with automatic mentions disabled. Generic Webhook retains its original JSON.
- Reuse encrypted repository-monitor configuration and per-channel language/delivery results. Optional DingTalk/Feishu/Lark signing, Telegram token/chat/topic, Discord thread, and generic Bearer authorization live in that encrypted configuration; readers receive redacted secrets. Platform/URL mismatch is rejected. Refer to each provider's linked official setup guide in the form; no live external delivery is implied by local contract tests.
- Top feedback occupies exactly 1 CSS px. Route progress is the opaque theme-colored bottom layer; every application API request has its own translucent layer (opacity .62), lifetime and animation. URL-path-derived session colors rotate the theme hue by 75–285 degrees. Raw streams finish at response headers; ordinary JSON calls finish after body parsing. These animations indicate pending activity, not measured transfer percentages.
- Workspace preferences persist through ordinary same-workspace navigation. Maintenance transitions refresh in the background; changing workspaces hides the old preferences until the new data arrives. Shared table empty states retain business-specific explanations and distinguish empty datasets from active filters.

## Host execution fencing after server suspension (2026-11-04, implementation)
- Scope: close the authorized execution-capacity boundary where SQL slot leases expire while a remote Actions worker is still executing.
- Decision: retain SQL scheduling and owner-checked leases; additionally hold kernel file locks in the actual Actions worker for its host/slot throughout execution and cancellation cleanup. A short allocation lock serializes occupancy checks across all 64 slots, including slots above a reduced capacity. Files remain in the system shared directory and are never unlinked; otherwise separate inodes could defeat exclusion.
- Reason: database TTL alone cannot fence a suspended remote process. Kernel locks continue to exclude successors during a server pause and release on worker exit; this works through both SSH and HostAgent without a second execution engine or clock synchronization.
- Alternatives: a heartbeat watchdog still needs proof of child cleanup before its deadline; never reclaiming SQL slots prevents automatic recovery. Neither is used as the physical exclusion mechanism.
- Tradeoffs: an expired SQL lease does not authorize killing the old worker; a successor waits up to 60 seconds for physical capacity, then fails closed with a diagnostic. Shared-directory permission failures also fail closed. Existing standalone worker requests without host-slot fields remain compatible. This fences worker invocations; it does not claim to prevent arbitrary detached processes deliberately created by repository scripts.
- Verification: native Windows cross-process exclusion/crash recovery and capacity-reduction tests passed; full worker/server and real remote execution regression pending.


## Pipeline object grants and ownership (2026-11-04, verification in progress)
- Decision: extend the existing definition ACL query/API to pipeline and pipeline_group subjects. Intersect the caller's live workspace capabilities with owner/direct/current-group grants; keep Workflow and pipeline subject kinds distinct even when IDs coincide. Filter before pagination and count, including run and artifact lists.
- Ownership: append a nullable owner_id migration; historical owners remain unknown rather than being guessed from editors. Every create_in caller records the actual creator. Only the current owner may transfer, under the workspace mutation lock, to an active member with live workspace and object Edit. CAS and the transfer audit commit together; revisions do not change and ordinary saves cannot inject an owner.
- Reuse/reason: share EXISTS-based authorization, ACL persistence, membership validation and workspace locking with Workflow. Group grant management requires the existing roles.update authority and group update permission.
- Scope/status: local implementation and acceptance in progress. This change alone does not implement production publication, pause legacy scheduled/event triggers, or fulfill PDR-04. No running environment migration is authorized or performed.

## Execution packet assets: encrypted locators and physical generations (2026-11-04)

- Decision: canonical packet plaintext determines the workspace-scoped SHA256 dedup identity. A randomly encrypted object is bound to workspace, asset ID and hash; the execution column stores a separately encrypted locator bound to the existing execution/node/field AAD. Retaining the asset reference and changing the node result share the fenced journal transaction.
- Reason: random encryption must not defeat dedup or allow moving an execution snapshot into a different identity. Creating a candidate/quota reservation before object I/O makes an interrupted or rolled-back upload discoverable by GC.
- Alternatives considered: plaintext objects violate output encryption; hashing random ciphertext loses content dedup; overwriting a deterministic physical key after uncertain deletion allows an old DELETE to erase a new upload.
- Tradeoff: a physical key includes the asset ID generation. Same-workspace live content still has one metadata/hash identity; rebuilding after collection gets a new key. Object storage and database commit are explicitly separate, recoverable steps.
- Scope/status: implementation of internal journal params/output assets and local/S3 storage; pending full platform acceptance, public scheduler/API integration, retention cleanup and pin provenance. Does not convert pipeline path addressing or claim a cross-system atomic transaction.

## Workflow object grants (2026-11-04, locally verified)
- Scope: shared definition_acl storage and the Workflow/resource-group API adapter; no legacy Actions access change, ownership transfer, publication or executable graph registration.
- Decision: intersect live workspace permissions with owner or direct/current-resource-group view/run/all grants. Use capability sets and EXISTS rather than ordering role names or joining/paging before authorization. Only the existing platform superadmin is an object override; an ordinary workspace read/update permission is not one.
- Administration: Workflow ACL management requires workspace update and object all/owner. Resource-group grants use the existing workspace roles.update administration authority plus workflow-groups.update. A grant recipient must be an active account and active member in the same workspace; granting cannot mutate owner. Immutable grants use idempotent creation and version-checked deletion, both audited under the workspace transaction lock. Parent deletion cleans only that parent's grants.
- Consistency: parent/detail/history authorization and data reads use one snapshot; list total/items use the same authorized snapshot (SQLite transaction, PostgreSQL/MySQL repeatable read). Writes and ACL mutations use the existing workspace lock and re-authorize after acquiring it, so stale queued requests cannot reuse revoked authority. No new product dependency.
- Verification: 454 workspace tests passed, 0 failed, 12 existing ignored. Rebuilt server passed all 17 HTTP/draft/ACL checks on isolated SQLite and MySQL 8.4.8, plus 8 real Actions Runner/history/restart regression checks. Evidence: data/maintenance/workflow-acl-workspace-results.json and workflow-acl-live-results.json, including source/runtime hashes and retained fixtures. Test MySQL was shut down through mysqladmin and its port release checked.
- Boundary: PostgreSQL runtime behavior, pipeline ACL adapter, ownership transfer confirmation (PDR-02), publication, graph execution and UI remain unverified or pending. Schema/SQL generation for three backends is not a substitute for PostgreSQL runtime acceptance. No commit, push, merge, deployment or existing service update.

## Workflow draft resources and save-time validation (2026-11-04, locally verified)
- Scope: Workflow draft CRUD, immutable history, lifecycle and separate resource groups. Saving a draft never creates production authority; publication, object ACL and graph execution remain separate pending requirements.
- Decision: reuse shared REST list/query contracts, workspace transaction lock, fresh transaction authorization, definition revision ledger and context groups. CAS definition/version/revision pointer/audit commit together; archive and restore clear publication; deletion retains immutable revisions and refuses execution or cross-definition revision references.
- Validation: execute the bundled graph and 53 node parameter schemas through jsonschema 0.60.2 with default features disabled (no default HTTP/file resolver); no preexisting dependency version upgrades. Only trusted bundled schemas are compiled. Structural save validation is distinct from resolved runtime validation, credential authorization and executor registration.
- Verification: 450 workspace tests passed, 0 failed, 12 existing ignored; rebuilt server passed 11 isolated SQLite HTTP checks plus all 8 real Actions Runner/history/restart regressions. Evidence: data/maintenance/workflow-drafts-workspace-results.json and workflow-drafts-live-results.json. First failed fixture used main instead of the declared Merge input port input; corrected the fixture without weakening validation and retained the failure log.
- Boundary: no PostgreSQL/MySQL draft CRUD verification, publication, object ACL, graph execution or new UI acceptance. No commit, push, merge, deployment or existing service update.

## Workflow persistence foundation (2026-11-04, locally verified)
- Scope: production Workflow definition/execution/node-attempt tables and workspace-wide execution numbering. CRUD, graph validation/planning, publication and object authorization remain pending; no legacy pipeline authorization or trigger policy changed.
- Decision: implement the model's baseline plus §10.11 fields together, using BIGINT identities, nullable unknown owner/publication/run-as links, immutable revision references, durable context/resolved-parameter fields and attempt identity `(execution_id,node_id,iteration_hash,attempt)`. Reuse the existing lease implementation against the actual migrated Workflow table. Preserve all data on down by refusing destructive removal.
- Numbering: the new workspace counter starts at 0 and is incremented under the existing workspace transaction lock, in the same transaction as an execution insert. Rollback restores both; reject i64 overflow. Do not use MAX()+1 or share the Actions number counter.
- Verification: 444 workspace tests passed with 12 existing ignored; real prior-binary-to-current upgrades passed 4 checks each on SQLite and isolated MySQL 8.4.8, and the rebuilt server passed 8 real Runner/revision regression checks. Evidence: data/maintenance/workflow-storage-workspace-results.json and workflow-storage-live-results.json. Current server SHA256 4fb5039fa6b8e6e3de8ca2a6e4be9fc48666058ed81939a2ed73c8c9fc6b91d0. PostgreSQL and real SQL contention/MVCC remain unverified. No commit, push, merge or service update.


## Immutable Actions definition history (2026-11-04, locally verified)
- Scope: shared revision ledger and existing Actions save/run adapters; publication pointers, object ACL and Workflow execution are separate pending requirements.
- Decision: reuse the workspace transaction lock and shared REST list contract. Canonicalize JSON object keys while preserving YAML string bytes. Only substantive definition changes append a revision; normal save of older content creates a new revision. Save validation, optimistic version checks and revision insertion share one transaction. A run freezes the source revision before analysis-specific checkout specialization.
- Migration: backfill one current revision per legacy pipeline using its existing Snowflake in the separate revision table, unknown author 0 and migration timestamp. Never invent a publication or historical author; unknown old run provenance remains revision 0. Repeated upgrade must not overwrite history. No destructive down migration.
- Verification: 440 workspace tests passed with 12 existing ignored; real HTTP/Windows Runner/restart acceptance passed all 8 checkpoints on the rebuilt server. Evidence: data/maintenance/workflow-revisions-workspace-final.log and workflow-revisions-live-results.json. PostgreSQL/MySQL runtime migration and new UI remain unverified. No commit, push, merge or service update.


## Shared execution ownership integration (2026-11-04, worktree implementation)
- Scope: workflow-platform execution foundation and compatibility with existing Actions runs; no change to other workspace sessions or the running local service.
- Decision: reuse the workspace row lock to serialize concurrency limits, group exclusion and lease claim in one transaction. Every scheduler attempt has a per-boot identity and epoch and runs under the independent heartbeat supervisor. Child mutations use an ownership predicate; completion uses the execution-row CAS and child updates in one transaction. Artifact publication locks and checks the execution row before publishing staged files.
- Recovery: existing Actions commands have no durable replay checkpoint, so an expired owned Actions run is claimed before being finalized under its existing interruption policy, without repeating external commands. Live leases are not swept at startup. Resumable graph/node execution remains required and is not replaced by this compatibility policy.
- Cancellation: database queries and log persistence are polled alongside the remote stream and stop channel; SQL pool contention cannot suppress the cancellation signal or drop the stream before cleanup acknowledgement. Transport output remains bounded through backpressure.
- Expiry: runtime SQL guards and renewal compare against the database execution clock, rather than a timestamp captured before waiting for a connection. PostgreSQL uses clock_timestamp to avoid transaction-start time; MySQL uses SYSDATE(3); SQLite uses millisecond strftime. Deterministic lease tests inject their logical clock separately. Real PostgreSQL/MySQL waiting/MVCC boundaries remain unverified.
- Reservation acquisition: poll the pending host reservation and persistent status reads concurrently; both may need the default single SQLite connection. Drop acquisition/check futures before cancellation finalization. The original four-instance run exposed a stalled queued command; retained evidence is workflow-cluster-executions-live.log. Acceptance requires rerunning the unchanged cluster assertions with rebuilt code.
- Verification: current workspace regression passed (436 passed, 12 existing ignored; server 388 passed, 8 ignored), including delayed expiry and reservation progress/cancellation regressions. Rebuilt binaries passed 24 real Windows Actions checks, 14 real host-capacity checks and all 12 cluster checkpoints, including four schedulers beyond TTL and queued command effects exactly once after cancellation. Evidence is retained in worktree data/maintenance/workflow-reservation-*, workflow-actions-*-after.log and workflow-cluster-executions-after.log; cluster raw data is cluster-acceptance-6hYlbd in the local temp directory. PostgreSQL/MySQL contention and graph recovery remain unverified. This decision is not a claim that the complete platform is implemented or accepted.

## Localized mixed execution output (2026-11-02, confirmed)
- Context: execution outputs mix product summaries with literal command logs. A single Message has a 256 KiB metadata limit, while existing logs allow 1 MiB of raw bytes.
- Decision: reuse i18n.rs with StoredText, an explicitly versioned `$system_text` collection of Raw and Message segments. Store message identity; render once for each reader. Keep raw logs, provider diagnostics, and user notes literal.
- Bounds: 1 MiB raw bytes, 256 KiB message metadata, 4096 segments, and 8 MiB encoded input. Truncate only raw UTF-8 prefixes; never encoded envelopes. Render a translated limit notice when bounded.
- Compatibility: log readers recognize only the new text envelope; legacy single-message-shaped log JSON remains literal. Existing error readers retain single Message compatibility. Escape reserved raw namespaces and decode exactly once. No schema or module added.
- Verification: 18 message-foundation tests passed, including large mixed logs, six reader locales, byte/metadata/segment limits, malformed envelopes, collisions, and SQLite reads without rewrites. Integration and release remain owned by the main agent.

## Post-alpha.34 integration and execution order (2026-11-02, user confirmed)
- Latest user clarification: finish ALL confirmed functionality and release first; structure/code-quality cleanup is a separate commit after that release. This supersedes starting module moves alongside features. Frontend never moved; initial backend entity extraction restored without losing preexisting dirty functionality.
- Approved Runner policy: global default automatic updates enabled; host inherit/on/off; exact server-version target, no downgrade, managed agents only; complete-task draining, signature/health/rollback preservation, bounded retry and visible state.
- Multilingual first release explicitly narrowed by user to en-US, zh-CN, zh-TW, ja-JP, ko-KR, ms-MY; remaining mainstream languages follow later. API uses standard Accept-Language, default English absent; app locale prefix/browser detection, validated browser-WebSocket locale query exception accepted. All selected locales require actual domain translations, not just selector infrastructure.
- Following user's certificate question, native upgrade test fixtures use process-scoped embedded test CA in throwaway source copies. Normal Runner installation needs no extra certificate. Do not mutate the user's trust store for subsequent fixtures; earlier temporary CurrentUser CA was removed and absence verified.
- Finish CI coverage for integrated insights, source themes and typed dictionaries/settings first; then UTC audit/fixes, Runner idle upgrade, actual cloudflared verification and multilingual support. Improve CI timing based on measurements. Present concrete designs before substantial behavior changes and perform unified acceptance before publication.
- Reuse current workflow jobs, existing three-database maintenance/transfer tests, production Vite preview and Chromium scripts. Run SQL contribution invariants on separate fresh databases; verify additive typed-data upgrade from released alpha.34 without replacing the existing historical alpha.1 upgrade gate.
- Released alpha.34 and local service upgrade are complete according to the user's release handoff; unfinished feature source and unrelated PRD edits are not part of that release.

## Typed dictionaries/configuration and source theme migration (2026-11-02, implemented and verified locally)
- Decision: use 7link-mall-web sys/dicts as the reference for both existing menus, sharing 12 value types, category, JSON values/schema and dynamic editors. Allow custom settings CRUD; preserve fixed operational keys/types/ranges, existing dictionary consumers, workspace isolation, audit and optimistic version checks.
- Reason: explicitly confirmed by user. Source repository remains read-only; source Next/i18n integration is adapted to Ferry shared resource components.
- Tradeoffs: additive migration retains old items/numeric projections; uploaded attachments are workspace/resource scoped in server data storage and require authenticated access. They remain retained when removed from a value to avoid invalidating other references.
- Scope: all six source themes including custom editing; ERP removed with saved-preference fallback. User also approved integrating existing code insights work into main; no publication implied.
- Verification: SQLite/MySQL8.4.10/PostgreSQL17.6 API, transfer and actual old-version upgrade suites passed; final production browser covered all12 value types, actual editors/uploads and responsive layout. Existing desktop-updater Windows named-pipe access test remains an unrelated environment failure; not modified or skipped.

## Unified list and form migration (2026-10-31, implemented and verified)
- Decision: user explicitly requested migrating all Ferry pages to the unified components in 7link-mall-prd/apps/7link-mall-web/ui/components/auto-table and auto-form. Port AutoTable contracts/state/hook/sticky helpers, and adapt the AutoForm configuration/renderer/field layout to Ferry's existing React/Radix controls. Keep the source repository read-only and retain Ferry's resource DSL as the API/permissions/business adapter.
- Reason: one renderer for resource lists, file tables and import previews; one form/field framework for resource forms, authentication, dialogs and specialized editors. Extend configuration/composition for rich flows instead of retaining page-specific table/form implementations.
- Tradeoffs: commerce-only widgets, use-intl/Next integration and unrelated rich-editor/map dependencies are not copied. Existing Monaco and asynchronous repository/Runner controls remain custom renderers inside the shared field frame. Retain URL state, secret semantics and delete confirmations.
- Verification: architecture audit rejects raw page form/table JSX outside the shared engines. Component interaction tests and real browser layout/business regression are required before delivery; no backend contract change or release is implied.

## Password minimum (2026-10-31, confirmed and delivered alpha24)
- User explicitly selected 8 characters instead of 10. Apply the same minimum to shared account validation, registration and platform setup/confirmation inputs. Existing password hashes and sign-in verification remain compatible. Delivered in alpha24 sourcea69b4c1 after135isolatedHTTP/158UI/156serverchecks and full21jobCI; publicartifacts/image verified, ordinarylocal4291 upgraded with identitiesandcredentials preserved.

## Repository monitoring mode selection (2026-10-31, delivered alpha24)
- Decision: user requested event push or active polling with minimum1minute. Keep legacy mode push by serde default. Poll through saved connection authorization, requiring no inbound Webhook secret; reuse notification history/delivery.
- Necessary persistence: encrypted SQL snapshot and last/next poll/error state, renewable shared database lease plus configuration/version fencing; no memory-only polling baseline or per-process schedule that duplicates work in cluster deployments. First complete poll creates baseline; errors never produce deletion events or replace the baseline.
- Limits: polling observes snapshot changes, not a complete audit stream. Missing repository means no longer visible to current credentials, possibly deletion or permission change. Show this distinction in UI and events. Codeup ListChangeRequests uses /oapi/v1/codeup/[organizations/{id}/]changeRequests with projectIds filter, not a repository-nested route.

## alpha23 combined delivery and native installer verification (2026-10-31)
- Decision/status: published v0.1.0-alpha.23 from d0ada482f70f70395c43725f70bd086d63d0c85e after successful exact-source full build and image; upgraded ordinary local4291 preserving identities and encrypted credentials. Latest accepted repository namespace List/Group switch ships in this release with monitoring notifications and previously approved cluster/monitoring work.
- Evidence: Windows upgrade diagnostics identified MainWindowHandle closing the single-instance plugin's transparent WS_VISIBLE helper rather than the UI. Gate now enumerates owned visible enabled Tauri Window class, matching existing native acceptance; original deadlines and exit requirement retained. Existing installer rerun and final full native checks both pass. No additional speculative desktop behavior change.
- Limits unchanged: monitoring history temporarily in service memory; provider Webhooks require manual platform setup; Codeup repository creation/deletion events unavailable. Verification and release identifiers are recorded in MEMORY.md.

长期产品与架构决策。历史条目的状态对应记录时点；后续明确决策优先，当前交付状态见 [MEMORY.md](MEMORY.md)。
## T2 流水线定义
- Decision：definition 在资源 API 和数据库中使用 JSON 字符串，纯函数校验后返回结构化 Definition。
- Reason：任务明确以 &str JSON 为输入，沿用已有资源字段风格；不引入执行机制。
- Alternatives Considered：直接接收 JSON 对象；本任务不额外增加双格式接口。
- Tradeoffs：客户端需序列化定义；workdir 必填，env 和步骤 timeout_seconds 可省略，未知字段拒绝，避免误拼静默失效。
- Scope：仅 pipelines 服务端。
- Status：已实施。
## T2 shell 与模板边界
- Decision：允许 run 中换行、分号、反引号和 $()；拒绝 NUL；模板只接受四个精确名称，不折叠空格或大小写。
- Reason：run 是运维编写的 shell 程序；定义校验不充当执行沙箱。
- Alternatives Considered：禁止 shell 元字符会阻止正常构建脚本。
- Tradeoffs：执行权限及模板替换安全属于后续执行引擎设计。
- Scope：T2 纯函数校验和对抗测试。
- Status：已实施并单测通过。

## T3a 数据层基础机制
- Decision：新 locks / worker_ids 用 SeaORM 实体与 schema builder，追加迁移；保留旧主键、旧迁移及 db::id 调用。
- Reason：用户明确执行 T3a，其禁止事项限定本次范围；§18.5 全量改造属于后续工作。
- Alternatives Considered：同步重写全部数据层；超出任务范围，未做。
- Tradeoffs：仓库暂时并存既有 raw SQL 与新 ORM 查询；新增机制没有 raw SQL。
- Scope：T3a；Status：实施。

## T3a 租约与分配原子性
- Decision：10 秒心跳 / 60 秒 TTL；本地使用 Instant，从数据库写入开始时刻计时；等待发号时也检查租约。分配在事务内取得锁，并通过旧 owner/heartbeat 条件更新槽位，避免回收期间覆盖刚恢复的心跳。
- Reason：墙钟回拨不能延长本地安全期限，慢响应不能额外续命；分配失败或取消必须回滚释放锁。
- Alternatives Considered：事务外持有 TTL 锁；操作超过 TTL 时无法确保串行分配。
- Tradeoffs：锁竞争/候选槽位并发续期时明确拒绝启动，不增加隐式重试。跨实例租约仍按设计依赖主机时钟同步；复制数据目录不能同时启动同一个 instance_id。
- Scope：worker 生命周期；Status：实施。

## T3a 优雅关闭与重启复用
- Decision：优雅关闭先停心跳/发号，再将本 owner 的 heartbeat_at 置 0，保留槽位映射。
- Reason：同时满足立即释放槽位和同 instance_id 重启优先复用；若槽位已被其他实例回收，则走最小空闲槽位分配。
- Alternatives Considered：删除行会丢失重启亲和性；不释放则占用到 TTL。
- Tradeoffs：保留最多 1024 行；Status：实施。

## T3b ID 存储与边界
- Decision：重写旧迁移的 41 个业务 ID 列为 BIGINT；保留原生 SQL。State::next_id 返回 Result<Id>，失败转 503；Id 绑定为 i64，JSON 序列化为字符串。移除 db::id。
- Reason：任务书与 §18.2/18.5；逐表核对实际为 41 列（14 表），不是预估约 39 列。
- Alternatives Considered：兼容迁移、SeaORM 实体改造；均不属于 T3b。
- Tradeoffs：本地旧测试库重建；可选引用空字符串输入解析为 0，输出为 "0"，校验允许 0 回传；必需引用仍由现有校验拒绝不存在的记录。请求体沿用字符串接收并显式解析，保持既有 400 JSON 错误响应。
- Scope：T3b；Status：实施，三库首轮各 98/98。

## T3c 强类型数据访问
- Decision：16 张表的实体集中在 entities.rs；业务读写使用 Entity/ActiveModel/Select，resource::list 泛型查询保留 Spec 白名单后解析实体 Column，Condition 绑定所有值。
- Reason：任务书和设计 §18.5；不增加继承式公共模型或运行时 SQL 转译层。
- Alternatives Considered：保留字符串 SQL 辅助层、取消列名白名单；均违背任务要求。
- Tradeoffs：host_tags 尚无业务调用者，但任务要求每张表均有镜像，针对该模块以有原因的 expect(dead_code) 标注，不伪造调用。实体不派生 Serialize，HTTP 显式映射 Id。凭据列表/详情通过 Summary 投影只读取公开字段及 has_secret 计算结果，保留原查询层不读取密文的防护；共享list支持类型化投影。
- Scope：只改数据访问；迁移 DDL/执行逻辑不动，migration.rs 仅 schema 测试移除已删除的 db helper 依赖；SQLite PRAGMA 元数据检查保留注释说明。
- Status：完成；三库基线及最终各 99/99，全部门槛退出0，两次变异正确失败并恢复验证通过。

## T3d Schema builder
- Decision：已有迁移全部改为 Table/Index/Alter builder，保留名称与顺序；索引和加列通过 SchemaManager 存在性检查重跑。MySQL 设置 utf8mb4_bin。
- Reason：任务书要求删除 DDL 字符串转换，同时保留 MySQL 可启动、大小写与幂等保护。
- Alternatives Considered：直接 .text() 在 MySQL 上会生成 TEXT 而非 LONGTEXT（SeaQuery 1.0.2 源码已确认）；只使用它会缩小原容量且违反无裸 TEXT 保护。
- Tradeoffs：长列以 .text() 定义，MySQL 专用 builder .custom("longtext") 保留容量；没有 SQL 字符串改写。SQLite 的 BigInteger 正常生成 INTEGER，元数据检查适配，并以实际 MySQL 生成 SQL 验证同一41列仍为 BIGINT，不能退化为32位整数。
- Scope：T3d 迁移与其保护测试；业务、JSON、错误文案和矩阵断言不变。
- Status：完成；全部门槛退出0，新建三库各99/99，两库last/all重放无业务数据或结构变化；变异退出101且恢复后通过。

## T4 执行引擎（进行中）
- Decision：三张独立表；工作空间行的计数器在事务内 UPDATE + 读取分配 number，唯一索引兜底。触发保存定义、仓库配置与目标主机快照；队列 CAS 领取，线性执行。
- Reason：工作空间内并发发号、运行历史可复现、单实例调度约束。
- Alternatives Considered：MAX(number)+1 先查后写有竞态；凭据放 exec/env argv 会泄露，因此拒绝。
- Tradeoffs：HostChannel 捕获输出后返回（沿用既有单步输出上限），本期按步分片。取消只停止等待，远端可能继续；清理临时目录后再结束执行器。取消状态可早于清理完成。
- Scope：T4 后端、真实验收脚本与 §17 文档；不改 UI、不加 exec HTTP、不 commit/push。
- Status：主机组按已告知的最小单目标假设，触发时选直接成员中 ID 最小的在线主机并固定 host_id；无在线成员拒绝，不做多机编排。触发可提供 branch/commit 模板上下文，缺省为仓库默认分支/空字符串。完成：三库原矩阵各99/99，真实执行各44/44，两次变异正确失败并恢复，全部门槛退出0。

## T5 构建界面
- Decision：仓库、流水线、历史仅配置现有 ResourceTable/ResourceForm；DSL 新增 code 与 resetOnChange，旧字段行为不变。唯一新增页面为 PipelineRunDetail，日志分片为其内部 memo 渲染单元。
- Reason：任务明确禁止另写资源表格/表单；JSON 需等宽多行，目标切换需清空旧引用。
- Alternatives Considered：手写资源组件、修改服务端以返回关联名称，均不在范围内。
- Tradeoffs：当前 API 只返回关联 ID，界面明确标注流水线/主机/触发人 ID。日志先读取状态、再按 next_seq 排空每页200片，终态排空后停止；隐藏abort并暂停，恢复可见立即读取未完成任务；稳定对象+seq+memo保留旧日志DOM，只有在底部才跟随。
- Scope：T5 前端及验收脚本；没有服务端、依赖、ResourceTable行为改动，无commit/push。
- Status：完成。23项前端测试通过；真实Chromium→服务端→runner→HTTP Git链路通过；终态继续轮询变异被三项测试捕获，恢复后通过。

## T6 CI / release preparation
- Decision：使用四个 e2e matrix job，每个连续三轮且任一轮失败立即退出；日志与 Summary 标注轮次/run id/SHA。release needs 包含整个 e2e matrix。
- Reason：失败按 harness 定位，连续运行覆盖清理竞态；不是失败重试，不能用后续成功掩盖失败。
- Alternatives Considered：合并单 job 会延长反馈并让前项失败遮住后项；触发三次完整打包成本更高。
- Tradeoffs：三轮共享同一 runner；只能提供已实际运行的证据，不能数学证明永无 flaky。
- Scope：T6；runner exec 复用 pipeline API，UI 用临时裸仓库 fixture branch 兼容 detached HEAD；无产品 API 变更。
- Status：本地四个harness均连续三轮通过；CI 三轮和 dispatch 发布产物收集必须用户提交推送后验证。不 commit/push/tag/release。手动触发即使选择 tag 也不发布，只有 push v tag 调用发布 action。
- T6 stability finding：本地UI第2轮实际复现状态/日志轮询竞态。用fixture文件屏障同步观察与后续执行，替换固定sleep；保留原断言并重新跑连续三轮。不是重试，不改产品轮询行为。

## T6 远端验收补记（2026-10-25）
- Decision：T6 验收通过；原“等待推送/CI待验证”状态已被远端证据取代。
- Reason：f1d7b0f 对应 runs 40110398175、40112246163、40113131064 全部 success，逐一核对四个 harness 每轮日志（每个 job 3/3）。
- Tradeoffs：三连绿满足门槛，不代表永无 flaky；发布演练验证收集/校验和生成/上传，不代表已正式发布。
- Scope：本次仅验收与状态同步。f1d7b0f 的临时 Git 仓库修复消除 UI fixture 对项目浅克隆历史的依赖。
- Status：两次 dispatch 均上传 10 个资产和 SHA256SUMS.txt；发布 action skipped。正式发版仍由用户决定。

## 首个预发布版本
- Decision：用户确认 v0.1.0-alpha.1；所有产品版本配置一致，GitHub Release 必须 prerelease=true。
- Reason：用户澄清当前不是正式版；保留0.1.0用于未来正式版。
- Scope：仅版本元数据和发布；误推v0.1.0已取消流程并删除标签，没有发布资产或版本镜像。
- Status：已发布。d607b63；main CI 40147891295、镜像40147891375及tag发布40148433585、镜像版本40148433485均success。Release prerelease=true；11资产齐全，校验和匹配；双架构镜像已发布。

## 发布门槛捕获 Git ASKPASS 用户名参数泄露
- Decision：使用 Git credential helper 标准管道协议，禁用 ASKPASS 回退，保留临时文件权限与清理机制。
- Reason：alpha.2 标签 CI 真实捕获 SSH 进程参数含凭据用户名；增加凭据进程采样窗口后，旧实现两种通道均稳定失败，新实现连续三轮各46项通过。
- Alternatives Considered：仅重跑测试、放宽用户名断言均不能解决问题，未采用。
- Tradeoffs：不再依赖 ASKPASS 提示文本；密码/用户名通过 helper 标准输入输出传递，不进入命令参数。
- Scope：发版必要修复及回归测试；Status：6e58782 已推送。

## 后续预发布使用 alpha.3
- Decision：保留已公开 alpha.2 Git/容器标签，后续发布 alpha.3。
- Reason：alpha.2 桌面 Release 因真实测试失败未发布，而版本容器已公开；不覆盖公开版本。用户已授权下一次预发布，“继续”“都要”覆盖后续发版。先前额外版本确认被明确撤回为过度谨慎，不将沉默当作授权。
- Alternatives Considered：覆盖 alpha.2 标签/镜像，需要改变已公开引用，未采用。
- Tradeoffs：版本序号跳过没有桌面发布的 alpha.2；alpha.1 用户手动安装 alpha.3 后启用后续应用内更新。
- Scope：版本与发版；Status：已发布 v0.1.0-alpha.3，abce34f；主分支及标签 CI/镜像全部成功，19项资产及签名/校验和通过最终验证。

## Desktop blank-address launch defect
- Decision: prioritize user's installed desktop blocker ahead of approved variables work. Empty address starts the existing bundled server, stores data under Tauri per-user app_local_data_dir, and waits for existing /api/status plus matching instance-id. Reject occupied port; never attach blindly. Hide launcher only after workspace creation; restore on workspace close. Stop owned child on app exit; suppress Windows console.
- Reason: released alpha.3 server.rs contained only URL navigation, no actual sidecar lifecycle or launcher hiding; installer tests did not exercise first launch.
- Alternatives Considered: asking user to manually run server or enter localhost would not implement documented blank-address behavior.
- Tradeoffs: native installed Windows UI still requires verification; backend lifecycle has real local process coverage and a new CI matrix gate. No server API/data migration or variable design changes.
- Scope / Status: local desktop hotfix implemented and tested; not yet published. Variables remain authorized and queued.

## alpha.4 桌面热修复交付
- Decision：发布 v0.1.0-alpha.4 测试版；用户“继续”覆盖 Windows 验收和发版，密钥变量模块保留为后续已授权工作。
- Reason：alpha.3 留空地址没有启动本地服务，连接窗口也未隐藏；本次补齐既定桌面服务生命周期。
- Alternatives Considered：手动启动后端不能满足安装后留空使用；不绕过 Windows 真实测试。
- Tradeoffs：Windows 单元测试首次暴露控件声明缺失（Tauri 只嵌入普通应用），CI 提取打包应用同一 manifest 给测试程序，全部原断言保留；非产品业务改动。
- Scope / Status：1495453 热修复、a0365af 测试声明修正已推送；main40197659996/tag40198613433及镜像全部成功。Windows 两种安装范围均验证真实页面、隐藏/恢复、重开和退出清理；19公开资产、18校验和、5更新签名及版本绑定全部通过。alpha.4 已发布，不是正式版。原密钥变量工作尚未实施，不视作取消。

## T8 独立资源实现 / 保留策略待答复
- Decision：继续用户已批准的密钥变量模块；独立实现加密资源、共享表单、作用域解析和测试，尚不决定旧密钥保留。
- Reason：回滚恢复当时值会加密保留旧密钥；回滚使用当前值会改变完整配置恢复语义。2026-10-25已发异步二选一问题，推荐历史加密快照，尚未收到答复。CLAUDE §2适用，不将继续模块等同于选择保留政策。
- Alternatives Considered：历史快照/当前值两方案已写docs/tasks/T8-variables.md。
- Tradeoffs：实际Compose交付尚未接入。pure render明确标记未绑定引用，执行入口拒绝标记，绝不静默漏掉环境项。
- Scope / Status：本地变量资源/共享UI/测试已实施；回滚与快照待选择。没有commit/push/release；不影响已发布alpha.4。

## Desktop automatic upgrade / tray (alpha.5 testing)
- Decision: signed download precedes private stdin EOF drain; stop dequeuing and reject new browser work while preserving runner transport for active steps/cleanup; await exit0 before installation. Timeout aborts install without killing work. Save one-use workspace resume marker; passive installer restarts automatically.
- Reason: Windows updater directly exits process and bypasses ordinary RunEvent cleanup; prior basicUi repeats installer choices, tray dependency alone never created a tray.
- Alternatives considered: force-kill active sidecar, public shutdown endpoint, asking users to manually end work do not satisfy requested graceful automatic upgrade.
- Tradeoffs: old binaries cannot acquire this lifecycle before first upgrade; old-client installer arguments are supported by minimal vendored CLI2.12.1 template modification, with first-install scope choice preserved. A one-time old-client transition limitation must be disclosed.
- Scope/status: user-authorized desktop fix, commits through6674792; release alpha.5 NOT YET published. Windows CurrentUser real checks passed; AllUsers overflow geometry verification pending final CI40208925861. No platform-superadmin role or first-run authentication policy changed. T8 snapshot retention choice still pending.

## alpha.5 发布验收
- Decision：发布 v0.1.0-alpha.5 测试版，commit6674792；桌面修复验收PASS，密钥变量仍保留本地未发布。
- Evidence：最终dispatch40208925861/tag40210357191全部成功；Windows两种范围实测通过托盘（含系统折叠区）、隐藏/恢复、跨版本无交互安装和自动重启；19公开资产、18校验和、5签名/版本绑定验证通过。两个镜像工作流成功，匿名registry验证实际ferry-server:v0.1.0-alpha.5包含amd64/arm64。
- Tradeoff：已发布说明明确旧客户端第一次升级仍需等待任务结束、退出再直接更新；新生命周期只能从新客户端发起后生效。主分支重复CI40210356975的checks仍在安装OS依赖，其余job已通过；不谎称该重复job已完成，同commit发布检查已全绿。
- Status：公开下载和应用内更新已可用。账号仍为注册/空间拥有者，无默认超管；首次初始化只提出建议，未擅自改变权限模型。T8旧密钥保留选择仍待答复。

## T9 — confirmed platform boundary and alpha.6 delivery
- Decision: user explicitly selected a real platform superadmin and platform management page; prior workspace-owner-only recommendation is superseded. Status: APPROVED and IMPLEMENTED in0bb0f51.
- Reason: multi-account/multi-workspace platform needs global administration distinct from tenant ownership. Global routes /_platform and /api/platform avoid collision with existing valid workspace slug platform.
- Security/compatibility: no default password, no automatic promotion of legacy users. One-time private operator proof plus human credentials; existing account claim requires its old password. Desktop owns proof handoff; server operator supplies it. Existing authenticated installs are prompted but may defer to continue workspace use. Parallel setup loser sees a login form.
- Transaction boundary: singleton platform lock serializes setup, account privilege/status mutations, registration gate/settings. Last active admin protected and actor rechecked; affected sessions revoked. Workspace creation and audit reused within transaction; shared form/list DSL extended with explicit global endpoint.
- Registration policy: enabled after initialization by default, configurable by superadmin. No password-reset or workspace-delete feature added.
- Integration: original T8 changes restored on main, migration order PlatformInitialization then Variables; T8 remains uncommitted and excluded from alpha.6. No decision on T8 secret-retention/rollback is implied.

## T8 历史变量回滚（已确认）
Decision：用户明确选择恢复历史变量值；revision变量快照以现有vault加密，AAD绑定workspace/revision，随历史删除。Reason：维持精确回滚。Alternatives：使用当前变量不保留旧密钥（未选择）。Tradeoffs：旧密钥可能已失效，UI提示。Scope：Compose变量引用，不扩展流水线。Status：2026-10-26用户已确认，实施中。

## T11 — creation-time app/pipeline templates
Decision: template CRUD plus shared form selection/parameter generation, no live binding. Reason: confirmed next scope reuses complete existing deployment/pipeline creation and execution. Typed .Values parameters only; app secret inputs are existing variable names and generate valueFrom references, never plaintext; pipeline secret explicitly rejected until engine supports variables. Tradeoffs: template updates require explicit reload; deletion leaves created resources intact; no service config file templates or marketplace. Status: implemented and locally accepted 2026-10-26; no release/version change.

## T12 full product completion — APPROVED 2026-10-26
User explicitly confirmed complete product scope including Docker/native/Swarm/K8s/Helm/artifacts/configfiles in addition to remaining navigation and import/export. External hosted Git platforms only (GitHub/GitLab/Gitee/Codeup), no internal Git hosting. Cron explicit timezone; missed schedule not caught up, overlapping skipped with record, retries default off but configurable. History retained until admin sets retention days. Alerts in-app and configuredWebhook. Implement in stages, real acceptance, no release requested.

## T12 alerts — APPROVED
Decision: user confirmed configurable thresholds, one unresolved event during continuous abnormality, record recovery, one Webhook attempt for trigger and recovery, failure recorded without retry. Reason: avoid notification storms. Scope: alerts stage3, no unsolicited external notification; tests use owned local receiver. Status: confirmed, implementation next.

## T12 external repository authorization — APPROVED
Decision: user explicitly says implement now; prefer browser/web authorization where supported, use API authorization where web flow is unsuitable/unavailable. Reason: user should not have to manually provision every integration token. Scope: GitHub/GitLab/Gitee/Codeup connections, authorization status/expiry/reauthorization; encrypted credentials reused. Tradeoff: deployment operator must configure registered OAuth app credentials/redirects; no actual private provider accounts were supplied, do not claim live private authorization acceptance. Status: user confirmed, implementation pending; not blocked waiting for testrepo question.

## 2026-10-27 release scope amendment
Decision: user defers K8s/Helm, requests every other remaining feature completed/tested and next alpha published for testing. Release now explicitly authorized, supersedes prior no-release scope. Keep Docker/native/Swarm/groups/artifacts/config/import-export plus desktop fixes. Do not silently treat next alpha as only existing changes.

## T12 final correctness fixes
- Shared creation transaction uses stable existing workspace row lock; actual MySQL concurrent import confirmations proved gap/insert deadlock in prior ephemeral lock table. Reuse existing lock_workspace, no retry/fallback policy added.
- Scope audit found host_tags unused; implemented original many-to-many intent via shared host multiselect/filter/CRUD/import. Omitted tag_ids preserves existing associations; explicit [] clears. Referenced tags and groups follow existing refusal-to-orphan policy. Parent groups validated for same workspace/cycles.
- Local acceptance complete for non-K8s/Helm scope, external live provider accounts remain unavailable as already disclosed. Bump next authorized alpha to0.1.0-alpha.7; remote CI/Windows/signed assets are delivery gates, no further user confirmation required.

## alpha.7 release acceptance — 2026-10-27
Decision: publish authorized non-K8s/Helm completion as prerelease v0.1.0-alpha.7 at8b4125c. Main40386314056, tag40394778911, images40386314066/40394779052 allSUCCESS.19public assets/18checksums/5signed versions verified. Windows both scopes andalpha4/6 upgrades, connection reopening/tray/lifecycle passed. CI systemd/tmp fixture race and positional pipeline-form selector fixed without relaxing assertions. Live third-party private accounts remain unverified; no default admin password; K8s/Helm deferred. Status: DELIVERED, release notes disclose limitations.

## Deployment editing UI — confirmed 2026-10-27
Decision: deployment configuration is a form; no drag/reorder or graph controls. User explicitly distinguishes build workflow visualization (React Flow/GitHub Actions style) from deployment settings. Scope: existing application manifest editor, preserve source editing and existing server contract; no new build graph implementation in this correction. YAML parser dependency used to preserve existing YAML/JSON editing safely. Status: deployment editor implemented, final browser acceptance pending.

## alpha.9 release acceptance — 2026-10-27
Decision: deliver authorized form/menu/repository/pipeline corrections as prerelease0.1.0-alpha.9 atfe4db1a; preserve publicalpha8tag/image, alpha8desktopneverpublished. Reason: alpha8 gates exposed draft reset and native rollback race; both fixed and regression verified. Main40443729070/tag40445832180 SUCCESS; main/tag images40443729071/40445832022SUCCESS. TagSwarmfixture cleanup race required unchanged full3-runjob rerun, final3x53PASS; no product assertions relaxed.19publicassets/18checksums/5signatures/versionbindings/anonymousOCIamd64+arm64PASS. Scope: deployment form, not build graph; K8s/Helm deferred, real private provider OAuth accounts still unverified. Status: DELIVERED; no further release action pending.

## Dynamic platform OAuth and unified repository creation — 2026-10-27
Decision: user requests dynamic platform app configuration and explicitly rejects repository/auth tabs. One repository create/edit form chooses GitHub/GitLab/Gitee/Gitea/Codeup/plainGit; provider web OAuth or validatedPAT then actual repositoryselection; plainGit inline password/SSH/token atomically saved withrepository. Reason: priorPATshortcut onlycreatedcredential and didnotconnectprovider. Alternatives: separateauthorizationtab/manualcredentialbinding rejectedbyuser. Scope: existingplatformsettings+sharedResourceForm+connection/vault reused, noGit hosting, noextraAPI-tokenmodule. Appsettingssuperadminonly/versioned/audited/encrypted; envfallbackuntilwebsave, storedsnapshotauthoritative thereafter; secretomitted/blankretains, disablekeepsconfig. Workspaces authorize separately; existingactor/providerconnection IDs preserved onreauthorization. PATtargetsboundtoconnectionindependentfromOAuthenable. CodeupAPIhasnoPATuseridentity, requiresGitusernameandorganization forcentralAPI; listverificationneverclaimedidentityverification. GiteaOAuth+PATincluded; marketGiteafileadapternotexpanded. BrowserCOOPisolationusesstate-boundrandomBroadcastChannel, nosecretstoredinbrowserstorage. Status: implemented/localacceptancePASS; realexternalprivateaccounts andinstalledWebViewuntested, no newreleaseyet.

## T14 multi-environment pipelines — APPROVED
Decision: user explicitly selected "同一流水线支持多个环境，运行时选择（推荐）" on2026-10-27. Implement multiple environment bindings per pipeline; choose one at trigger; freeze chosen environment's executor/variables into history. Existing apps/deployments and tags reused, no duplicate app registry. Status: approved, implementation starting. Earlier pending-environment statements superseded.

## T15 explicit user correction — 2026-10-28
Decision: code connection consists of platform + auth, existing credentials reusable; repository/branch chosen at pipeline build configuration. Supersedes previous decision requiring repository choice at connection creation. Reason: user explicitly rejects premature branch/repository binding and repeated credential input. Scope: connection management and pipeline source selection; preserve old source bindings/history. Status: APPROVED by explicit request; implementation pending.

## T16 Runner lifecycle required in next alpha — APPROVED
Decision: user explicitly says “agent runner 这个版本就要” after discussion of service installation, auto recovery and graceful automatic upgrade. Include Linux/macOS/Windows native services/autostart/crash restart and drain→verified upgrade→healthcheck→rollback in next alpha, not a later release. Existing network reconnect retained. Boundary: brief agent offline during replacement is disclosed; no zero-disconnect/automatic failed-job resume claim. Scope includes necessary signed artifacts and native platform acceptance. Status APPROVED, not implemented.

## T17 desktop unattended Windows updates — APPROVED 2026-10-28
Decision: user approved independent updater, ordinary helper for CurrentUser and protected initially-authorized native updater service for AllUsers. Preserve graceful task drain and automatic original-user restart; eliminate repeated installer wizard/UAC on already provisioned installs. Old installation migration may need one authorization; no OS privilege bypass. Scope: desktop Windows update lifecycle plus necessary signed broker/installer/native tests. AgentRunner supervisor remains separate. Implementation plan docs/tasks/T17-desktop-unattended-updates.md; not implemented/accepted yet.

## 根目录共享项目记录 — 2026-10-29
- Decision：将原 `.claude/decisions.md` 移至根目录 `ADR.md`，原 `.claude/task-state.md` 移至 `MEMORY.md`。ADR 保存长期决策，MEMORY 保存当前状态、限制、验证证据和交接记录。
- Reason：用户要求保留有价值的记录并移至根目录，供不同开发工具共同使用；采用标准拼写 MEMORY。
- Scope：项目文档及其引用；不改变运行时行为。原路径不再维护。
- Status：已实施。历史记录保留，当前状态摘要优先于历史检查点。

## 工作空间 ID 与统一资源路径 — 2026-10-30
- Decision：用户明确选择统一资源路径、去掉菜单分组。页面 `/workspaces/{workspaceId}/{resource}`，API `/api/workspaces/{workspaceId}/{resource}`；节点为 `nodes`、主机分组为 `node-groups`。平台页面 `/platform/{resource}`。菜单和面包屑保留业务层次。
- Identity：使用数据库已有雪花 ID，JSON 和浏览器全程保留十进制字符串。名称/slug 不再作为接口作用域索引；保留 metadata，不修改既有 ID、数据和权限资源名。
- Operations：HTTP 方法配合名词子资源（`runs`、`connection-tests`、`cancellations`、`renderings` 等）；排序用 PATCH `ordering`，批量删除用 DELETE 集合。共享资源用 `scope` 查询参数。
- Compatibility：旧页面书签在登录后通过身份中的可访问工作空间映射为 ID，保留详情和查询；API 拒绝 slug，不增加旧 API 别名。Runner 机器连接协议与安装分发端点维持兼容。
- Supersedes：旧的 slug/菜单分组页面路径和 `/actions/{verb}` 接口约定；不涉及数据库迁移。
- Status：用户已批准，当前实现以 MEMORY.md 验证记录为准。

## Windows terminal start and resource detail layout — 2026-10-30
- User requests a user directory as the default Windows terminal CWD. Prefer own user profile, then active console/RDP user profile when running as LocalSystem; no logged-in user falls back to PUBLIC/system drive. Do not change service identity or permissions.
- All schema-driven detail pages reuse ResourceForm with dedicated read-only section/field styling and available width. Existing editors, secrets, permissions, actions and routes retain their behavior.

## 节点报警上下文与应用菜单 — 2026-10-30
- 用户要求节点详情新增报警规则时保留层级与上下文。复用 ResourceForm 在详情弹窗内创建，固定关联当前节点，忽略外部查询中的复制和目标预填；保存后刷新当前规则列表，保存及取消均不跳转到其他模块。不增加重复 API 或页面资源路由。
- 一级菜单“部署”改为“应用”，共享导航定义同时更新菜单、无障碍名称和面包屑。现有资源 URL、部署操作名称及 API 权限语义不变。
- 已随 `v0.1.0-alpha.21` 交付，源码 `2d9d3b5`；完整 CI 和实际浏览器验收通过，4291 开发服务已更新。后续状态以 MEMORY.md 顶部为准。
# Codeup PAT contract hardening — 2026-10-30

Decision: extend the existing repository adapter and shared checkout UI, retaining named connection IDs, vault references, ownership/workspace permissions and native Git credentials. Verified against current Aliyun PAT API docs: central uses organizations/{organizationId}, Region omits it; repositoryId accepts the encoded FULL path including organization; lists are arrays with x-next-page/x-total-pages. Use header pagination with existing count fallback; decode URL path once before encoding an API segment, using the already locked percent-encoding crate as a direct dependency. Add paged branch suggestions and current-page retry without changing an entered ref. Distinguish upstream authorization/permission/rate/service errors without exposing response bodies or forwarding credentials through redirects. No AccessKey integration, Git hosting or execution-platform expansion. Live Aliyun account acceptance requires a locally configured connection; fixtures must never be represented as live cloud acceptance.
## Credential management module boundaries — 2026-10-31
User requires separate host, build and application authorization pages. Reuse one encrypted credential store with immutable module scope and scoped lists/pickers; retain existing credential permissions. Use existing REST resource `/workspaces/{id}/credentials?scope=host|build|deploy`, keeping menu/breadcrumb context. Upgrade preserves IDs, ciphertext and existing references; host-referenced legacy credentials belong to host, marketplace-referenced credentials to deploy, other legacy credentials retain build ownership. Existing historical references continue to resolve; this change does not redesign repository-connection ownership or runtime RBAC.

## Optional NATS JetStream server clustering - APPROVED 2026-10-31
Decision: user approves NATS JetStream for node connection ownership and cross-instance commands/terminal/files/monitor access. Preserve standalone operation without broker. Use expiring ownership with fencing, bounded streams and no mutating-command replay. Owner loss ends existing terminal; Runner reconnect allows a new session. Monitoring recent history remains owner memory (one hour); no fabricated backfill or long-term TSDB this round. Scope includes cross-instance resets and upgrade/task exclusion. Shared business database and vault master key required; separate data directories per server. Status: approved, implementation/testing in progress, not delivered.
# Repository connection events and list grouping (2026-10-31)
- Decision: reuse private named connection ownership, workspace repository permissions, encrypted vault, shared resource form/action dialog and shared resource table. A connection receives a monitoring tab; repository lists optionally group the current remote page by full namespace.
- Event ingress: platform-authenticated public callback, narrow browser-CSRF exemption, bounded body, provider-aware normalization, explicit all/specified repository filtering. Codeup repository hooks do not expose repository creation/deletion; disclose this and do not infer events.
- Cluster behavior: shared SQL transaction and unique authenticated-body digest admit each payload once across replicas before one bounded outbound HTTP attempt. No automatic delivery retry or durable job queue is introduced. An interruption can leave a pending notification; this limitation is displayed and documented.
- Secrets: encrypt complete settings using the existing connection/workspace-bound vault context; callback secrets and notification URLs are write-only. Deleting a connection removes its event settings and history.
- Verification: isolated API acceptance 120 checks, real Chromium grouping/settings/responsive acceptance, server 151 passed/4 existing ignored, new Codeup old/new payload tests. Actual cloud accounts/webhooks remain unverified.

## File browser and repository namespace navigation - 2026-10-31
- User requires a default-off hidden-file switch on all platforms and switching among all mounted disk partitions. Reuse host monitor disk enumeration and existing host command channel for Windows hidden/system attributes; no incompatible Runner frame or forced service upgrade.
- Repository grouping is a directory browser, not an expanded/collapsible flat list: direct child folders/repositories, parent/breadcrumb navigation and shareable folder query. Include every provider page in the directory index while list mode preserves upstream pagination. Implement through the shared table adapter.

## Complete Compose quickstart - 2026-10-31
- User requests one-command deployment including SQL, NATS and Redis. Provide common persistent/authenticated services with mutually exclusive PostgreSQL/MySQL override files; Python generates stable random credentials and validates/waits for Compose startup. Repeated startup must preserve master key and refuse accidental database switching.
- Redis is optional infrastructure, not an application dependency until actually integrated. This Compose is explicitly single-host; HA requires external database HA, a multi-node JetStream cluster and distinct application instance volumes. Default image is the published immutable version tag rather than latest.

## 2026-10-31 ? Terminal directory compatibility and workspace tool lifetime
- Use remote node platform metadata (readonly OS/agent version), with drive-path inference for missing legacy metadata, to select directory startup syntax. Never use the browser OS to decide remote shell syntax.
- Keep current Runner protocol/services unchanged for legacy support. CMD directory selection uses a literal encoded resolver and PUSHD; run UTF8 codepage selection on a separate line before FOR output decoding. Legacy service-profile home resolution changes cwd only, not user identity or PATH. Existing terminal permission remains sufficient; ConPTY cursor reports must flow during metadata loading.
- Workspace terminal/files tabs mount on first use and retain their component/session until the pane/tab closes. Reuse Radix Tabs and the shared form/table contracts for subscription header actions and full-width cards.

## Explicit Windows terminal runtime account ? 2026-10-31
- User confirms node-level terminal runtime account configuration to access user-installed tools and login configuration; cwd-only changes do not change the LocalSystem identity.
- Bind existing host-scoped password credentials through shared ResourceForm; secrets stay vault-encrypted and never appear in node JSON. Only newly opened Windows Runner terminals use the configured account. Files, monitoring, pipelines and existing sessions preserve their identity. SSH terminals retain SSH credentials.
- Use LogonUserW, loaded profile and CreateEnvironmentBlock, ConPTY/CreateProcessAsUserW with target-user home; terminate descendant processes through a job on session close. Failed logon is an error, never a silent SYSTEM fallback.
- Separate TerminalUserStart request and advertised capability prevent old Runner versions from silently ignoring configuration. Reuse existing opaque cluster session relay. No new middleware, service identity changes, or automatic interactive-user impersonation.
- Status: UI/API/protocol/failure-path integration and regression tests passed; local server updated. Privileged native success validation remains pending after OS UAC cancellation. Installed Runner remains unchanged; capability not yet released.

## Runner install scope selection (system / current user) — 2026-10-31
- Decision: Runner installation offers 系统服务 or 当前用户 on Windows, Linux and macOS (UI connection command + install scripts). Linux/macOS reuse the existing `service install --user` (systemd --user / LaunchAgent). Windows current-user mode registers a per-user logon Task Scheduler task (no administrator, no stored password), state under %LOCALAPPDATA%, running only while the user is logged on.
- Reason: lets terminals/commands use the user's own identity, PATH and tool logins (e.g. npm-global codex) without changing SYSTEM installs or storing Windows passwords.
- Alternatives rejected: SYSTEM-wide codex install (runs AI tooling as SYSTEM, separate login); Windows service under a user account (needs admin, password, logon right); interactive-user token impersonation from SYSTEM.
- Supersedes "Explicit Windows terminal runtime account" above: user chose to withdraw that uncommitted feature (not released). Full pre-withdrawal diff kept in ignored data/maintenance/pre-user-runner-full.diff plus windows_terminal.rs.bak/test-terminal-accounts.mjs.bak.
- Constraints: existing SYSTEM installations and default behaviour unchanged; no hidden window flashes; self-update/rollback supervisor reused unchanged; old Runner releases must be refused explicitly for Windows user mode.
- Status: IMPLEMENTED and released in v0.1.0-alpha.25 (a84b74e).
- Addendum (user request, 2026-10-31): wherever the Runner install command is shown, also provide a one-click uninstall command for the same platform and install scope. It removes the native registration and the default installation directory (program, connection credentials, host key, logs); system scope requests administrator/sudo, current-user scope does not. The CLI `service uninstall` keeps its data-preserving behaviour.

## Release alpha.25 scope additions — 2026-10-31
- Node detail tab "报警监控" renamed "报警" (global sidebar entry unchanged).
- Node monitor network charts show physical adapters by default, hiding loopback/container/hypervisor/VPN/OS-filter interfaces by a name heuristic, with a "显示虚拟网卡（n）" toggle; if every adapter looks virtual, all are shown. UI-only: the Runner still reports every interface; protocol unchanged. Reason: mainstream dashboards (Zabbix discovery filters, Grafana node_exporter device filters) exclude virtual interfaces by default.
- User-visible "授权认证" renamed "凭据" everywhere (menus, permission labels, forms, server messages, docs). API path /credentials, data and permission keys unchanged. Reason: the items are login/access secrets; "凭据" matches existing "登录凭据" wording and common usage.

## Chat robot webhook formats — 2026-11-01
- Problem: repository monitor and alert notifications always POST Ferry's generic JSON. WeCom/DingTalk/Feishu robots reject it but still answer HTTP 200 with an error code, so deliveries showed "已投递" while nothing arrived (user's WeCom group).
- Decision (user-confirmed): detect official robot endpoints by URL (qyapi.weixin.qq.com/cgi-bin/webhook/send, oapi.dingtalk.com/robot/send, open.feishu.cn|open.larksuite.com/open-apis/bot/v2/hook/) and send each platform's native readable message; judge success by the platform's errcode/code, reporting the platform error otherwise. Any other URL keeps the existing generic JSON and 2xx semantics. One shared helper serves both repository monitor and alert deliveries. Robots requiring signatures/keywords are not configured specially; their rejection is reported verbatim.
- Alternatives rejected for now: explicit channel field in forms (needs form + config migration).
- Release: alpha.27, separate from in-flight alpha.26.
- Addendum (user request "telegram WhatsApp 等能推送都做进来"): also recognise URL-only push channels — Telegram Bot API sendMessage (chat_id taken from the URL query), Slack incoming webhooks (hooks.slack.com/services/…), Discord webhooks (discord.com|discordapp.com/api/webhooks/…), Google Chat space webhooks (chat.googleapis.com/v1/spaces/…/messages) — each with its native body and success rule. WhatsApp is NOT included: the Cloud API needs a bearer token header, a recipient number and Meta-approved templates for business-initiated messages, which a URL cannot carry; it needs dedicated form fields and is deferred.

## Detailed repository events and retention — 2026-11-01
- Decision (user: "有需要的都要有，不能太简陋，要做全"): repository events carry structured detail — push commit range (SHA, full message, author, time, link; ≤20 with total), before/after, forced, compare link; branch/tag head commit and annotation/tagger; merge request number/title/full description/state/branches/author/merger/reviewers/merge commit/times/link; repository visibility/description/url. Stored as nullable JSON `repository_events.detail`; list shows a summary; a read-only detail page (shared form engine, new generic `records` field type) shows everything.
- This reverses alpha.24's minimal design (empty actors, no PR bodies in events). Kept: baselines still hold only names/SHAs/fingerprints (detail lives in event records, never the encrypted baseline); author e-mails are never stored.
- Operators: webhook mode uses the platform's pusher/merger/deleter. Polling cannot know who acted: pushes/new branches show the newest commit's author, new PRs the author, merges the merger — each labelled as inferred; deletions and other unknown operators stay empty with an explicit note recommending event push.
- Polling fetches branch commits (one page of 50) outside the workspace lock; failures add a note and never fail the poll.
- Retention (user suggestion): workspace setting `repository_event_retention_days` (0 = permanent, default 0, preserving the existing "history kept unless an administrator sets retention" policy); pruned every 10 minutes in batches with an audit record.
- Status: released in v0.1.0-alpha.27 (fe24f95).

## Compose quickstart as a one-command shell script — 2026-11-01
- Decision (user: should be `curl … | sh`, not Python): replace scripts/quickstart.py with POSIX `scripts/quickstart.sh`, run as `curl -fsSL https://raw.githubusercontent.com/7link-team/ferry/main/scripts/quickstart.sh | sh [-s -- options]` without cloning or Python. It creates/reuses a deployment directory (`./ferry`, `--dir`), downloads the Compose files of one release tag, pins `FERRY_IMAGE` to that same release, and keeps the previous semantics (random reusable secrets, database switch refused, health wait, setup token, `--init-only`, `--without-redis`).
- Version: latest published release from the releases Atom feed (includes pre-releases, not subject to the anonymous API rate limit, which was exhausted during verification), GitHub API as fallback, `--version` to pin. Fixes the stale alpha.24 pin; compose.production.yaml no longer defaults FERRY_IMAGE.
- Windows runs it in WSL or Git Bash. Tested by scripts/test-quickstart.sh with stub curl/docker (CI runs it under dash).

## Compose quickstart one-command upgrade — 2026-11-01
- Decision (user asked whether the quickstart supports one-click upgrade): `quickstart.sh --upgrade [--version vX] [--dir D]` upgrades an existing deployment in place. Other configuration options are rejected with --upgrade; addresses/secrets stay as they are in `.env.production`.
- Order: read current tag from FERRY_IMAGE (digest or non-release tags refused → manual) → resolve target (Atom/API or --version) → same version no-op → refuse downgrades by semver (pre-release < release, numeric identifiers compared numerically; migrations are forward-only) → database dump through the running DB container (pg_dump --clean --if-exists as postgres / mysqldump as root via in-container env; failure aborts before any change) → backup env + Compose files to `backups/<UTC>-<old>-to-<new>/` (umask 077) → stage all new release files in `.upgrade/` then swap → only FERRY_IMAGE tag changes (registry part kept) → config/pull failure restores files, services untouched → `up -d --wait`; unhealthy prints exact rollback commands, no automatic DB restore.
- Redis profile kept iff a redis container exists. Application/JetStream volumes are not part of the backup (documented).
- Alternatives rejected: automatic database restore on failure (risky against a partially migrated DB without operator review); recording redis choice in the env file (existing deployments lack it).
- Status: implemented, stub-tested (scripts/test-quickstart.sh, 30 PASS); no real-container upgrade run (no Docker locally).

## Compose HTTPS domains through a script-generated Caddy proxy — 2026-11-01
- Decision (user: no FERRY_PUBLIC_URL, several domains, commands show the domain in use, ship without a release): `quickstart.sh --domain NAME` (repeatable) records `FERRY_DOMAINS`, allows those hosts, sets secure cookies, and generates `compose.https.yaml` + `caddy/Caddyfile` in the deployment directory (Caddy 2.10, ports 80/443, egress network, `caddy-data` volume, automatic Let's Encrypt issue/renew, HTTP->HTTPS redirect). `--allow-host` (repeatable) replaces `--public-url` for plain-HTTP LAN access and cannot be combined with `--domain` (Secure cookies). Access options passed to an existing deployment replace the recorded ones; secrets never change. Removing the domains stops and removes Caddy.
- The proxy is generated by the script (served from main), not shipped in the release Compose files: it does not depend on the Ferry version, so existing deployments get it without a release.
- FERRY_PUBLIC_URL: never read by Ferry (commands come from the request Host, scheme from secure cookies). Removed from compose.production.yaml/compose.yaml; the script keeps it only while a deployment's compose file still requires it (releases up to v0.1.0-alpha.29), set to the first domain.
- Redis choice now recorded as FERRY_REDIS; deployments without it keep Redis iff a redis container exists. Reruns read the database from the env file.
- Verification: stub tests; official docker compose v2.39.4 config with the published alpha.29 files and the new files; real Caddy 2.10 + Ferry server for two *.localhost domains (HTTPS, redirect, per-domain Runner commands, Secure cookies, unknown host 403). Real Let's Encrypt issuance needs a public server (sslip.io on the user's server).

## Self-service profile: name, login email and password — 2026-11-01
- Decision (user chose "本人在「个人资料」里改"): the signed-in user edits their own name, login email and password at `/account` (GET/PATCH `/api/profile/1`, a single-record form like the platform settings, rendered by the shared ResourceForm). Reached from the workspace account menu and the platform management header.
- Rules: the name needs no password; changing the email or the password needs the current password (wrong/missing -> 400). Emails are normalised like registration (trim + lower-case) and must be unused (409). New passwords follow the registration length rule. Optimistic `version`. Email/password changes delete every other session of the user and keep the editing one. Audited as `update-profile` (changed fields; old/new email; never a password).
- Administrators cannot change another user's email or password (account takeover risk); the platform user form keeps email/name read-only. No email verification: the platform has no mail delivery.
- Verification: scripts/test-platform.py (8 new checks, 31 total) and scripts/test-platform-ui.mjs (profile flow in Chromium).

## Workspace lists show the user's own relation — 2026-11-01
- Decision (user: each workspace should show the role, so one can tell created from joined): `/api/platform/workspaces` items and `/api/me` workspaces carry `relation` = owner (created it; ownership never changes) | member (joined; `my_role`/`role_name` is the role) | none (a platform administrator who has not joined). `/me` no longer replaces a superadmin's real roles with a blanket "平台超级管理员" (kept only for unjoined workspaces); permissions stay full. Shown as the platform list column 「我的角色」 and in the workspace switcher: 拥有者（我创建的） / 角色（已加入） / 未加入（平台超级管理员）.
- The platform list's enter action uses an explicit LogIn icon instead of the label-derived fallback ellipsis.

## Referenced records are shown by name — 2026-11-01
- Decision (user: lists must not show raw IDs such as 流水线 ID/触发人 ID; APIs carry both ID and name; pages show the name, the ID is optional; check other pages too): `crates/server/src/names.rs` attaches `*_name` next to reference IDs in one query per referenced kind, scoped to the workspace (users global, display name only); unresolved references get "" and pages show "已删除（ID …）", absent ones "—".
- Applied to list/detail responses that are displayed: pipeline runs list + detail (pipeline_name, triggered_by_name, host_name), pipelines list (repository_name, target_name by target_kind), pipeline artifacts list (pipeline_name, run_number), deployments list (target_name), deployment revisions list (deployment_name, triggered_by_name). Single-record GETs that feed edit forms are unchanged so saving never round-trips name fields. Exports gain the name columns (export files are not import templates).
- Filters that asked for typed IDs now choose by name (artifact pipeline, revision application, alert rule, cron job, marketplace source when readable). The manifest editor's artifact source chooses the pipeline and an optional successful build run by name when it knows the workspace; the stored manifest still holds IDs, and choosing another pipeline clears a pinned run.
- Not changed: variables' scope_ref still typed (depends on scope); Actions jobs on Windows hosts remain unsupported (act worker is Linux/macOS).

## Pipeline runs snapshot the names they started with — 2026-11-01
- Decision (user chose "存快照，显示执行时的名称"): `pipeline_runs` gains `pipeline_name`, `host_name`, `triggered_by_name` (VARCHAR(255) NOT NULL DEFAULT '', migration `pipeline_run_names_v1`), written when a run is created, like the existing `environment_name`. History shows the names as executed after a rename or delete. `names::attach` keeps a name the response already carries, so runs recorded before the migration (empty snapshots) are still named live from their IDs.

## GitHub Actions pipelines on Windows hosts — 2026-11-01
- Decision (user: a YAML with only `run` steps must not be refused just because act cannot run on Windows; chose to make the existing act engine support Windows rather than add a second engine): Windows runner hosts execute Actions jobs in host mode (`image: host`). Container images remain Linux-only and are refused on Windows with a clear reason.
- Server (`windows_shell.rs`): the server's POSIX job scripts run on Windows through Git for Windows' `sh.exe` (located via `git --exec-path`; Git is required there to check out code). Each command is written to a script file, so it never depends on cmd.exe quoting. Paths are native with forward slashes (`C:/…`); the private job directory lives in the account's %TEMP% and is limited to that account and SYSTEM with icacls. System tools are called by full path (`%SystemRoot%\System32\whoami.exe`, `icacls.exe`) because Git's optional Unix tools on PATH shadow `whoami`.
- Runner: `cmd /S /C "<command>"` via `raw_arg`. Before this, `Command::arg` escaped inner quotes as `\"`, so every quoted command on a Windows runner failed (proved with a minimal program and a runner test). The server probes the fix (`echo "ferry"`) and asks to upgrade older runners instead of misreporting missing Git.
- Worker (done by Codex CLI in a separate worktree, reviewed and re-verified): builds windows/amd64 and windows/arm64 (`.exe`); every host process is created hidden (CREATE_NO_WINDOW/HideWindow, via the fail-closed actpatch); unspecified shells prefer `pwsh`, else Windows PowerShell 5.1, invoked with UTF-8 output and a BOM'd script; command files written as UTF-16 by PowerShell 5.1 are decoded before parsing and masking. Two real masking defects were found and fixed this way (code-page output corrupting Unicode secrets; UTF-16 GITHUB_OUTPUT).
- Limitation: Windows cancellation is taskkill (forced), so post steps may not run.
- Verification: worker `go test` on Windows 19 PASS/2 SKIP, linux+darwin vet; `scripts/test-actions-windows.py` real server + current-user runner + worker on this Windows machine 9 PASS; CI runner-services (Windows) runs both and gates releases.

## Pipeline v2 decisions D1–D17 — confirmed as recommended (2026-11-02)
- Decision: user accepted all recommendations in docs/ferry-流水线全功能设计.md §27 (D1–D13) and docs/ferry-流水线对标功能清单.md §3.1 (D14–D17): worker evaluate subcommand for server-side expressions; S3-compatible artifact store for clusters; YAML on.schedule rejected in favour of cron jobs; trigger_run_as rebinding to last definition editor; legacy JSON frozen with converter; Windows native shell jobs in P3; robot-webhook notifications without email; phase order P0→P1→P2; reruns create new runs with copied artifacts; concurrency in envelope settings with queue_latest default for event triggers; fork MRs never auto-run; environment protection gates; trigger_environments mapping; pipeline-level permissions in P2; admin bypass off by default with per-environment switch; draft/publish where event and schedule triggers use the published revision (P1); generic secret webhook trigger (P1).
- Scope: design only; development not started (user waiting on another session). Checklist §3.2 items still need folding into the design before T18 task breakdown.
- Status: CONFIRMED.

## Product scope expansion to a three-plane workflow platform — confirmed (2026-11-02)
- Decision: user requires that everything Aliyun Flow, n8n and ComfyUI can do is available in Ferry. Ferry expands from an ops platform to "研发交付 + 通用自动化 + AI 内容生产". Three execution planes share one shell (triggers, credentials, queue, approvals, versions/publish, runs, artifacts, canvas): (1) host job plane = pipeline v2 (Actions YAML, Flow parity); (2) dataflow node plane = self-built n8n-style engine (item dataflow, control nodes, sandboxed code, HTTP/webhook response, LLM/agent, integrations added in batches); (3) AI engine plane = **self-built** ComfyUI-style capability (typed node graph, input-signature caching, queue/progress/preview, model management, generation nodes); full ComfyUI ecosystem compatibility NOT required, functional coverage is.
- Constraints: n8n source must not be embedded (Sustainable Use License forbids commercial redistribution) — behaviour reference only. ComfyUI is GPL-3.0 — no code copying; self-implementation only. Supersedes general spec §1 positioning and the v2 "single definition format" principle (dataflow graphs need a second graph format); v3 overall design must state both.
- Order: v2 P0 (leases/concurrency/revisions, shared by all planes) first, then v3 shared shell and data model, then v2 P1 and new planes in parallel.
- Status: CONFIRMED; v3 overall design not yet written.

## v3 decisions V13–V14 confirmed; PRD before development (2026-11-02)
- V13: JS code/expressions run in a separate low-privilege sandbox process (ferry-sandbox), no vault key or DB access; S1 starts with a technical spike.
- V14: Not built: hosted build clusters, package-protocol artifact repository, Go-template YAML pre-rendering. K8s batch/blue-green/canary ships with the application module's K8s plan. Other Flow gaps handled per v3 §18.2.
- V15 CONFIRMED 2026-11-02 after explanation: public pages (form trigger, chat trigger, external review, read-only run share) recommend a separate FERRY_PUBLIC_ORIGIN; without it fall back to same origin with strict CSP and no session access; public access per workspace stays default-off (V9). V1–V12 recommended, not yet explicitly confirmed.
- User direction: no development task breakdown now; produce the capability inventory (v3 §18.5) and a complete PRD first.

## D5/D6 superseded by implemented decisions (2026-11-02)
- D6 (Windows native PowerShell executor in P3, recommended 2026-10-31) is superseded by the 2026-11-01 user decision to make the existing act engine support Windows host mode, which is implemented and verified ("GitHub Actions pipelines on Windows hosts — 2026-11-01"). The blanket D1–D17 confirmation on 2026-11-02 approved a stale recommendation; the specific, earlier, implemented user choice prevails.
- D5 (freeze legacy JSON + converter) is superseded by commits c32c4ee/02df78a, which removed the legacy pipeline format (convert on write and by migration) so all pipelines run as Actions workflows.
- v2 design header, PRD (FR-PIPE-01, NFR-COMP-02, §12, Q02) and capability inventory are annotated accordingly. Found by the PRD drafting review (Q02).

## v3 V1–V12 and PRD open items Q03–Q15 confirmed as recommended (2026-11-02)
- V1–V12 confirmed as recommended in docs/ferry-工作流平台v3总体设计.md §16 (V6/V7 concrete model families and providers still chosen before S4/S3).
- Q03 permissions: workspace permission is the ceiling; object roles (owner/all/run/view) granted within it; group grants inherited by members and unioned with object grants; no explicit deny; ownership transferable to members with edit permission.
- Q04 skip failed job: requires the object's "all" permission; only failed/timed-out host jobs; approvals, deploy tasks and environment gates cannot be skipped; downstream continues; audited.
- Q05 merged MR runs check out the exact merge commit from the event, never the target branch head.
- Q06 problem severities: block save (syntax errors, expression interpolation in host commands); block publish (missing credentials, host bindings, models); block execution (budget enabled and price missing; model not ready on target host); everything else warns.
- Q07 publish version name required, 1–64 chars, unique per definition; change note optional ≤200 chars.
- Q08 non-approval human tasks: assignees follow approval rules; default timeout 7 days → fail; external review links valid 72 h and revocable; edit tasks autosave drafts every 30 s.
- Q09 public form defaults: 20 MB per file, max 5 files, 10 submissions per IP per minute; captcha off by default, switchable.
- Q10 queue view shows cluster-wide aggregate with concrete reasons (concurrency group/host/GPU); members see their workspace only.
- Q11 statistics: success rate = succeeded/(succeeded+failed); duration excludes queueing, waiting shown separately; costs summed per credential currency, no FX conversion; unpriced shows "—".
- Q12 archive auto-unpublishes (triggers and public entries stop), history kept, restorable as draft; only archived definitions can be deleted.
- Q13 routes/REST follow general spec §6 (no DELETE bodies; noun sub-resources); new pages /workspaces/{id}/workflows, /executions, /assets, /models, /human-tasks.
- Q14 pipeline execution identity binds at publish (aligned with workflows and D16); saving drafts does not rebind. This amends D4's save-time rebinding.
- Q15 inventory rows with evidence gaps stay "未开始" until V6/V7 selection; no early commitment.
- Precedence: where these conflict with v2/v3 technical docs, the PRD and this entry prevail; technical docs are aligned at task breakdown.
- User also approved committing and pushing the finalized documents after the PRD review.

## PRD review follow-up defaults (2026-11-02, under the user's "按推荐" approval)
- Skip failed job (Q04) ships in P2 together with D14 object permissions; no interim substitute for the "all" permission. Skipping creates no new attempt: the job becomes skipped with skipped_by/reason, the failed attempt's logs stay in history.
- Human "return for changes" (Q08): reviewer returns with a required comment (≤500 chars); the task ends with decision=returned and the graph decides what follows (e.g. a redo loop). Same for external review.
- Public uploads (Q09): allowed types come from each form field's declaration; configurable upper bounds 200 MB per file and 20 files; chat: 20 messages per minute per session, uploads follow form rules.
- Statistics (Q11/M1): time buckets use the workspace timezone (default server timezone); waiting time is excluded from duration and shown separately; zero denominator shows "—"; recovery time = time from a failed run to the next succeeded run of the same definition, averaged over the window; per-job failure rate is included.

## Controlled state presentation (2026-11-04)
- Operational values belong to backend contracts, never editable workspace vocabulary: host, pipeline, deployment, cron, alert, repository and insight state sets cannot be replaced, extended or removed through dictionary CRUD or import.
- Dictionaries configure presentation only: labels, per-locale overrides, colors and order. Legacy system labels are recognized in all six supported languages and projected into multilingual metadata at read time; custom literal names remain literal. Generated default translations are removed before persistence so later translation corrections remain effective.
- Frontend filters, badges, forms and exports resolve the active language from the same option metadata and always submit the fixed value. Environment and application-category dictionaries remain business vocabulary rather than operational enums.

## Conversation files, deletion and explicit group participation (2026-11-08)
- Decision: extend existing conversations with explicit human memberships and frozen per-Agent execution bindings. Private conversations keep owner-only access; group visibility requires explicit membership plus workspace permissions. Only explicitly selected @Agent targets execute; human messages without targets are persisted without model calls. Each reply retains its triggering actor; tool decisions and cancellation require that actor.
- Decision: reuse encrypted content_assets/ArtifactStore and asset reference GC for binary uploads; store per-upload filenames and conversation ownership, serve authenticated previews/downloads with byte ranges. Do not expose active HTML/SVG as same-origin documents. Existing UTF-8 attachment requests remain compatible. Model media comprehension is a separate adapter capability, not implied by upload.
- Decision: owner can delete an idle conversation with confirmation; reject while queued/running/stopping, transactionally release file refs and delete messages/events/turns/memberships. Never delete the user-supplied execution directory or native CLI history. Execution directory cleanup remains the managed-directory lifecycle work.
- Reuse/scope: Rust Server scheduler/leases, Scope, resource forms, themes, six languages; direct execution, no subagents, no new dependencies, no global CLI changes or production deployment.
- Validation: owner/member/nonmember/admin privacy, target-only dispatch, independent Agent native state, forbidden cross-conversation file references, range/download byte equality, deletion during execution, record cleanup; browser upload/paste/drop/audio and desktop/mobile flows. Status: implementation in progress.

## Conditional execution and branch projection - implementation design (2026-11-04)
- Scope: existing core.filter/if/switch contracts and core.merge append only; no new product defaults, dependencies, migration or UI. Remaining merge collision/join rules remain unresolved, not silently chosen.
- Reuse: graph parameter resolution, bounded QuickJS subprocess, immutable attempt snapshots and Journal atomic multi-port output. Optional internal capture_presence envelopes distinguish top-level undefined from null only for conditional operands; ordinary expressions remain strict JSON and nested undefined/nonfinite/functions remain errors. Literal operands are enveloped by the resolver too, preventing user object values from impersonating internal metadata.
- Routing: persist every declared output port, including explicit skipped branches. Upstream lookup flattens ports in manifest order; paired indices include preceding-port offsets. Merge waits for every predecessor terminal projection, then consumes inputs in index order; any_items excludes skipped/empty inputs and skips only when none has items, all_inputs skips on a skipped input while executed-empty inputs participate.
- Verification: sandbox presence/cache/syntax and bounded regex tests; conditional truth tables/type errors/missing/null; real HTTP branch routing, downstream paired lookup, merge empty/skipped, immutable restart recovery on isolated SQLite/MySQL. Multi-port pin/bypass semantics are not invented; unsupported execution combinations must reject explicitly until defined.
- Status: technically necessary implementation design within existing authorized workflow scope; not approval of unresolved PDR business decisions.

## Failed execution terminal bookkeeping - implementation design (2026-11-04)
- Problem: an executor error can terminate its parent execution while its activated node row still says running. That stale row must not look like ongoing work or become replayable.
- Design: extend the existing lease-fenced workflow finish transaction. Acquire the parent row in the same order as Journal, mark its running node attempts failed with only the bounded public end-reason code, then recheck lease/version/expiry in the final parent terminal update. Node and execution timestamps/versions commit together; succeeded and waiting historical rows and encrypted snapshots are preserved. No raw exception, expression or stop details enter public error fields.
- Reuse: existing execution lease/CAS, workflow_node_runs fields and transaction; reuse the Journal database fixture for real production-schema tests. No new table, dependency, product default, retry or continue/error-output semantics.
- Acceptance: runtime executor failures produce failed attempts with completion timestamps; completed attempts remain byte-for-byte unchanged; failed history survives hard restart; expired/stale owners and expiry injected before final write roll back all node/parent changes. Verify isolated SQLite and MySQL; fault-trigger injection remains SQLite-only and explicitly excluded from MySQL.
- Scope boundary: this fixes existing stop-on-error terminal bookkeeping. Outputless Stop nodes, retry, cancellation state codes, other error policies and undefined product defaults remain separate existing work; no claim those features are implemented.

## PDR-01 through PDR-05 confirmed as documented (2026-11-04)
- Decision: user explicitly selected the documented recommendations in both asynchronous confirmation questions: PDR-01 through PDR-03, and PDR-04 through PDR-05, in PRD section 15.10. The complete corresponding table rows are the approved rules, including their limits and edge cases; this record does not replace those details with an abbreviated interpretation.
- Scope: implement shared form/API validation and resource limits, human operation/protection defaults, trigger/session/notification rules, safe legacy production-authorization migration and the documented currency/month/reservation/rate-limit accounting rules.
- Constraints: implementation and tests remain in the isolated workflow worktree. Confirmation of migration behavior does not authorize applying that migration to running user deployments, publication, commit, push, merge or updating port 4291.
- Status: CONFIRMED. PDR-06 provider/model/hardware choices and the separate execution-timeout question remain unresolved; they are not implied by this approval. Continue all independently authorized work without requesting these five approvals again.

## Outputless Stop node - implementation design (2026-11-04)
- Scope: implement the existing core.stop contract on the current stop-on-error execution policy. Its resolved message/details remain in the encrypted attempt snapshot; public termination uses the existing bounded node_failed reason and a localized generic stopped message. It uses the same terminal transaction as other executor failures.
- Outputless history: freeze a default-false outputless flag in new attempt snapshots from the manifest's declared outputs. Empty output-port packets are permitted only for these authenticated snapshots, so mute/upstream-skipped Stop nodes can finish skipped without fabricating a main output. Old snapshots retain the prior nonempty-output rule. Ordinary nodes still reject an empty output-port packet; an executed empty main port remains valid and distinct.
- Boundaries: flat pins and bypass have no defined outputless routing contract and remain explicitly rejected for this node; this does not silently add ports to its manifest. Active Stop always fails, so always_output cannot turn it into success. No new state codes, retry rules, credential or external I/O.
- Verification: old-snapshot compatibility and output shape guards, node/parent failure atomicity and six locale strings; real active Stop/mute/upstream-skipped behavior, encrypted private details, immutable failure/skip history across restart on isolated SQLite and MySQL.

## Workflow ownership transfer - implementation design (2026-11-04)
- Implement FR-AUTH-02/05 and AC-AUTH-06 with POST /workspaces/{workspace}/workflows/{id}/owner-transfers, accepting only owner_id and version. Ordinary draft writes continue ignoring ownership fields. Only the current owner can transfer, including when the caller is a platform administrator.
- Reuse the workspace mutation lock, live caller reauthorization, object Edit checks and Scope::for_actor_in. Require explicit active workspace membership even for a superadmin recipient, then require the recipient's existing workspace and object editing permissions. No implicit grants or old-owner grant are added.
- Compare-and-swap owner/version and write old/new owner identifiers in the same activity transaction. Do not change revisions, production identity, publication pointer or in-flight runs. No migration or new dependencies.
- Validate with real isolated SQLite/MySQL HTTP: target membership/activity, workspace ceiling, object edit grants, owner-only authority, stale/concurrent requests, audit and revision preservation, scope/body injection, and restart persistence. UI confirmation follows the approved PDR-02 ordinary confirmation rule when the shared UI is implemented.

## Workflow publication ledger and identity - implementation design (2026-11-04)
- Implement the approved definition_publications table with append-only metadata, exact case-sensitive trimmed 1-64 character names, optional 200-character change notes and optional bounded request keys. Use the existing schema builder's explicit MySQL binary collation and unique subject/name and subject/request indexes. Do not backfill publications or run production migrations.
- Workflow publications use existing workspace update and run ceilings plus object Edit/Run. The publisher becomes run_as; a different previous publisher requires an explicit matching previous_run_as confirmation. Lock the workspace, reread live authority, validate the selected current draft revision and runtime, then write the immutable ledger, pointer, identity and audit atomically with version CAS. Ordinary draft saves keep production pointer and identity untouched.
- Reuse graph checked validation, plan compilation and the current explicit runtime support gate. Publication ignores pin_data, and unsupported runtime/credential/host nodes remain refused until their actual executors and authority adapters exist. Do not promote advisory topology warnings into product errors. No tenant expressions are executed by publishing.
- GET/POST workflow publications and POST unpublication use separate endpoints and existing list envelopes. An identical request-key retry returns its original publication without republishing; a changed payload/actor conflicts. Unpublication clears only the active pointer, retaining ledger and prior identity for later identity-change confirmation. Archived workflows cannot publish.
- Verify isolated SQLite/MySQL names and limits, request races/retries, publisher switching/confirmation/revocation, draft-versus-published manual runs, archive/unpublish, immutable in-flight snapshots and hard restart. This step does not claim automatic trigger delivery, pipeline publication or UI completion; those remain required subsequent work.

## Administrator-installed declarative workflow packages (2026-11-05)
- Decision: extend the existing HTTP declarations with startup-loaded JSON packages under the server data directory; expose the combined catalog through an authenticated workspace endpoint for the existing editor and credential forms. Installation/enabling/disabling requires restart and browser refresh, not recompilation. User authorized continuing extension work; this is the first data-only increment, not a JS/WASM plugin implementation.
- Reuse: graph schema compilation, expression restrictions, existing encrypted profile-bound credentials, HTTP request builder, object/run authorization and durable request ledger. No tenant code loading or new runtime/dependency.
- Identity: package id/version namespace both node types and credential profiles. Canonical package SHA256 is recorded in a durable version lock; changing a registered version fails startup. New versions coexist with previous packages. A disabled package is not silently substituted; dependent definitions become unavailable until restored.
- Trust/scope: packages are installed by the deployment administrator, never via a tenant upload endpoint. First version restricts parameter forms to supported flat scalar controls, fixed HTTPS HTTP execution and single-server deployments. Cluster mode with enabled packages fails explicitly until shared catalog identity is implemented. JS/TS and WASM are recommended future code extension targets; Lua is not added by this increment.
- Tradeoffs: avoids a compilation requirement for new HTTP integrations and preserves old versions without mutable global reloads. No hot installation, package manager, sandbox for arbitrary third-party code, npm/custom-node compatibility or extension translation bundle is claimed. Removing a package can prevent pending workflows from running; the operator must finish running work before disabling and back up the lock with workflow data.

## Foundation priority and embedded extension engines (2026-11-05)
- Confirmed: stabilize Actions/Codeup visual and YAML editing and n8n-style node editing/execution before expanding the integration catalogue. JSON plus embedded JS/QuickJS, WASM and Lua are in scope; Python and externally installed interpreters are deferred. Reuse isolated workers, not the business server process, for code execution. This supersedes the earlier suggestion to defer Lua; engine/plugin support is not yet implemented.
- Current implementation unit: keep Actions YAML as the sole editable truth and reuse its AST, existing inspector, map rows and save/binding contracts. Add workflow/job environment and command defaults, job conditions, safe structured runner editing and reusable-workflow form fields. Patch mapping entries in place so comments, aliases, scalar types and unknown siblings survive unrelated edits. Do not infer Flow dialect compatibility from the form UI or introduce automatic conversion.
- Validation: regression tests for comments/types/aliases/unknown fields and actual browser visual/YAML round trips, save/reopen, real runner execution, read-only and narrow viewport. Retain invalid YAML for correction; mutations must not silently rewrite unsupported mapping forms. Only isolated 4292 may be updated after backup/verification; no commit, push, merge or production release.
## AI模型直接配置与响应式表单（2026-11-09）
- 范围：模型页直接配置连接，一次事务保存；复用既有供应商、凭据、额度和价格校验，无新增连接管理模块或数据库迁移，历史 provider_id API 兼容。
- 布局：Desktop/Pad 短字段两列，地址和模型清单整行，价格卡片两列；Phone 单列，保留标签、帮助、错误及统一底部按钮；禁止整页横溢。
- 状态：创建默认现有 OpenAI 参数，编辑回填；版本冲突保留输入；非法配置回滚；无凭据链接现有凭据页；加载和错误明确；只读显示文本。CLI 无连接及历史自定义连接保留。
- 权限/审计：保留工作空间 Scope 与既有模型/连接独立权限，提交审计，查询不增加业务审计；共享连接的编辑影响沿用既有行为，不复制。无删除、保留、公式、索引、灰度、第三方变更；配置来源仍为用户配置。
- 文案/时区：复用六语及建议值必选、检测提示。验收覆盖真实创建/编辑/查看、权限、跨空间、冲突和回滚，1440/900/390/320 布局截图。Figma 暂缺、虚拟键盘和原生 App 实机未覆盖，不冒充全站认证。

## 仓库连接与洞察按工作空间共享（2026-11-05，用户确认）
- Decision：命名仓库连接及代码洞察按空间角色权限可见、使用和维护，不再以创建者作为访问边界；列表、详情、事件、仓库解析、洞察聚合和定时采集采用相同范围。
- Reason：用户进入demo空间后管理员看不到原demo账号创建的连接/洞察，明确要求进入空间就应按权限可见。
- Reuse：继续使用Scope工作空间提取/权限检查、既有连接查询及资源列表，不改数据归属，不新增共享开关或第二套权限。
- Boundaries：跨空间继续隔离；读权限不赋予修改/删除/采集权限；未命名OAuth授权准备会话、回调state及密钥保持个人隔离/不回显；洞察编辑保留原创建者及独立身份映射，后台授权身份不随查看者改变。
- 用户补充：平台超管进入任意存在的空间即拥有最高权限，无需加入、不创建成员关系；查询不增加空间活动/业务操作记录，提交修改/删除/触发等操作按真实超管身份审计。平台内部请求监控/错误日志保留，普通空间用户不可访问。
- Alternatives：仅移除列表过滤会导致详情404/采集需求丢失，未采用；更改所有者或复制记录会丢失语义并重复数据，未采用。
- Status：已实施并备份更新4291，角色/跨空间/未加入超管/查询无业务留痕及提交审计测试通过；具体证据与用户验收状态以MEMORY.md为准。


## 预览穿透 Preview Holes（2026-11-09，用户确认）

- Decision：复用Runner会话通道新增`SessionKind::Forward`，公开路由`/holes/{token}/...`按路径反代到Runner本机`127.0.0.1:{port}`；同源下强制`CSP sandbox`（不含allow-same-origin），剥离Ferry Cookie/Set-Cookie/Service-Worker-Allowed。
- Reason：不分配额外域名；Rathole为L4按端口转发、与现有通道重复；同源不加沙箱时预览页JS可用查看者会话调用全部Ferry API。
- Alternatives Considered：Rathole；子域名/独立端口origin；仅凭token权限（不安全，风险来自查看者登录态）；HTML改写（不可靠）。
- Tradeoffs：被预览应用不能用Cookie/localStorage/IndexedDB；Web预览需应用配置base或相对路径。
- Scope：详见docs/preview-holes-design.md；转发长连接不持有升级租约，Runner停止/升级时主动关闭。
- Status：已实现并验证，提交于feat/preview-holes@403a5b6，待合并main（2026-11-09）。实现补充：导航放host分组；url返回根相对路径由浏览器拼接；错误页为带沙箱CSP的纯文本；转发会话不持有Runner升级租约。

## 中断私聊显式恢复（2026-11-09，用户确认）

- 事实/范围：用户拒绝中断后永久锁会话，要求可重试和继续输入。原会话Codex thread/resume返回no rollout found；不伪造原生历史已恢复。本期修复私聊中断恢复，不修改群聊成员/智能体生命周期。
- 决策/复用：沿用会话PATCH、版本冲突、所有者及工作空间校验；中断可显式改为active，要求update+run权限、无排队/执行中回合，清空原生会话ID，保留主机/账户/目录/权限绑定。界面允许输入，发送时先恢复；重试为新client_id的新回合，不覆盖旧记录，不自动重放。
- 上下文：仅从同会话已完成回合取最近20条未撤回聊天文本，序列化并限制16KiB及总请求64KiB预算；历史明确标记为上下文，不再次执行。失败/不确定回合、其他会话、附件及工具请求不自动重放。重试保留未发送草稿，权限和附件检查沿用原发送接口。
- 数据/兼容：无新表、字段、状态或迁移；历史interrupted会话可直接使用新入口。原生恢复仍要求身份绑定一致，绑定变化不绕过。重试前在界面提示核对先前操作结果；失败不是撤销已执行操作。
- 状态/响应式：默认active沿用；interrupted显示说明与重试，允许输入；archived只读；无权限仍只读，版本冲突/离线/发送失败保留草稿并刷新版本。按钮和说明在窄屏换行；复用Composer/按钮/错误反馈及六语。Figma与原生App暂缺，不冒称覆盖。
- 验证：后端测试所有者/跨空间/权限/版本/忙碌/旧回合保全/新原生ID/上下文边界；前端验证可输入、显式PATCH后POST、草稿保留；真实Runner及本地4291验收结果见MEMORY.md。

## 克隆配置中的多仓库和分组（2026-11-09）

- 事实/范围：用户反馈实际克隆步骤仍是单选。流水线复用已有批量checkout生成器，入口补到克隆步骤内；替换当前步骤并保持原位置，首个步骤保留原ID，条件、环境变量、checkout版本与非来源参数随步骤保留。后续命令不被移动到克隆之前，输出引用不因首个ID改变失效。
- 节点工作流：克隆节点可保存同一连接下多个仓库；分组包含子组并展开为当前完整分页清单，去重保存快照，不在运行时静默追加新仓库。每仓库使用根目录下独立编号子目录，结果包含仓库URL和实际目录。复用原Git传输、凭据解析、租户授权、逐项发送账本、主机调度/取消；无新服务、凭据种类或运行器协议。
- 边界：单个节点批量克隆上限500，对齐图的500节点容量边界；空清单、重复URL、非法路径/URL、匿名混用、展开后同目录和超限在发送前拒绝。中途失败保留已完成的克隆及发送账本，不自动回放。流水线仍生成标准Actions多步骤，不虚构单个actions/checkout支持多仓库。单节点批量模式只绑定一个连接；跨连接使用多个节点，流水线批量选择保留跨连接能力。
- 状态/界面：默认单仓库不变；可切批量、搜索、多选仓库/组、取消选择；加载期间禁用选择，错误可刷新原连接；失效的已选记录保留名称，不静默删除。Desktop/Pad用两列列表，Phone一列；长路径换行，仓库/组列表自身滚动，操作可达。六语，Figma/原生App无交付来源，不冒称覆盖。
- 数据/权限/兼容：仅扩展已有定义JSON和节点schema的可选repositories字段，旧定义无需修改；无数据库变更、DDL、索引、金额计算、新审计字段或删除。查询权限不变，保存/执行沿用原工作空间/连接/凭据与主机权限，非查询操作沿用现有审计。灰度无；需服务端与前端一并更新，旧服务不识别新可选字段，不支持仅回滚服务读取新定义。
- 验证：前端真实组件覆盖Codeup分页分组去重、清空联动、步骤插入位置/原ID/条件/环境/YAML锚点；后端覆盖展开路径、边界与既有连接隔离。实际HTTP Git+Runner、保存回读及四宽度截图见MEMORY.md，外部商业Codeup写入不在本次测试范围。

## 已知主机的连接命令默认值（2026-11-09）

- 事实/范围：用户要求已知主机按OS及系统/当前用户服务默认展示命令。复用hosts.os、Runner Hello、在线注册和集群RunnerInfo，不靠浏览器OS/用户名/管理员权限推断。
- 设计：Hello可选installation_scope仅接受system/user/foreground；官方service worker从现有service.json.user_service报告system/user，直接CLI报告foreground。旧Runner缺字段为未知；在线集群复制同一可选元数据；不新增数据库列。connection-credentials在原hosts.enroll和workspace范围检查后返回os/installation_scope，不新增权限豁免。
- UI/状态：已知OS自动选择Windows/macOS/Linux；已知system/user选择对应安装范围；foreground显示直接启动说明，旧版/离线未知要求手选范围再生成/卸载命令。不执行安装/重装/重置，不改已有服务。当前页手动选择不被状态轮询覆盖；切主机清旧令牌和默认值。元数据仅展示，不作授权依据。
- 数据/兼容：无新表/字段/时间审计/索引/DDL或up/down迁移；旧Hello和集群记录默认None；新字段可被旧端忽略；离线范围不持久化，不伪装已知。无删除/金额/计算/新增依赖；命令生成/重置审计沿用现有规则，查询不新增操作记录。
- 校验/全状态：限定OS枚举和三种上报值，未知值丢弃；空/请求错误/离线/旧版保留手选且不生成错范围命令；长文案换行，令牌不写日志。六语；Desktop/Pad/Phone沿用连接卡片与通用Tabs/表单、纵滚/安全区/键盘；截图测试四宽度，Figma和原生触控未提供。
- 验收：RunnerConnection单测Windows用户服务默认与foreground不冒认系统服务；协议旧Hello兼容、server Runner transport/集群回归；隔离真实Runner握手和用户指定本地主机页面验证。system/user语义来自官方worker配置，未经真实OS安装流程不宣称系统服务安装验收。

## 账号外观与语言云端偏好（2026-11-10）

- 决策/依据：用户要求主题和语言存入个人云端配置。复用现有主题/自定义变量白名单、六种语言及Account认证；不新增设置页面、不改工作空间配置，不新增依赖。直接执行；读写同一认证账号，不接收user_id，未知字段拒绝。
- 接口：GET/PATCH /api/me/preferences。theme包含palette、mode、customLight、customDark；locale为现有六语枚举。PATCH只更新传入的非null字段；theme整体覆盖，locale独立更新。并发同字段最后到达数据库的写入生效，不引入版本冲突弹窗。每标签页串行发送，登出/切账号中止旧请求与未发送队列；迟到GET不能覆盖本地新选择。
- 模型：account_preferences.user_id BIGINT NOT NULL主键/外键users.id，删除账号CASCADE；theme TEXT NULL（上限由两组各40键、键40字节/值200字节约束），locale VARCHAR(16) NULL（六语）；updated_at BIGINT NOT NULL毫秒UTC，updated_by BIGINT NOT NULL DEFAULT 0（当前账号，0=system）。无金额/计算/灰度/额外索引；主键覆盖唯一及全部查询。theme枚举复用现有六主题/三明暗模式；自定义变量在显示前继续使用前端白名单过滤。
- DDL（迁移SeaQuery对三库生成）：CREATE TABLE account_preferences (user_id BIGINT NOT NULL PRIMARY KEY, theme TEXT NULL, locale VARCHAR(16) NULL, updated_at BIGINT NOT NULL, updated_by BIGINT NOT NULL DEFAULT 0, FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE); 无条件唯一索引/方言差异。up新增表不修改历史账号；down删除该表及偏好，不影响账号/业务数据；需备份后部署，旧二进制降级必须同步匹配迁移状态。
- 状态：匿名只存本地；登录GET读取云端，尚无主题使用modern/system，语言未设保留当前URL语言；切换自动PATCH，读取/写入失败显示现有多语言错误且保留本地显示，不谎报已同步，不后台自动重放。仅本地缓存支持首屏；云端值到达后覆盖缓存，已有账号本机偏好不自动上传，避免多人共用浏览器串号。查询不新增审计；偏好更新以updated_by/at追溯，非安全权限变更，不计账号资料version。
- UI/全状态：复用现有顶部切换器，无新布局或新弹窗；Desktop≥1200/Pad768–1199/Phone320–767均保持原菜单、换行及安全区行为，切语言保留路径其余部分、query/hash/history.state与草稿。空值回默认；错误不阻断业务；最大自定义键数和非法枚举测试覆盖。Figma无，本轮不宣称新增三端截图交付或原生App验证。
- 验收：后端隔离数据库测试持久化/账号隔离/部分更新与校验，前端测试云端恢复/匿名/编辑顺序/加载竞态/账号切换/失败及现有主题语言和App回归；另跑隔离真实HTTP+浏览器跨上下文验证。部署、提交推送、发布分别记录，不由本决策授权。

## 中立访问地址与域名预览（2026-11-10，用户确认）

- Decision：主要公开URL和可选预览根域名统一在平台设置的访问地址分区；控制台接受网络可达合法Host，保留认证、同源和写入请求标记。预览按token.根域名分流且不回退控制台，未配置域名时禁用。
- Reason：用户拒绝路径前缀开关及env/部署脚本重复维护业务域名。原256位token无损Base32转换成52字符标签，复用原授权、撤销、Runner和沙箱。
- Alternatives：旧有效路径308迁移，旧域名和未知token404。运维DNS/TLS/代理设置仍属于部署层，数据库配置不能替代。
- Scope/Tradeoffs：新增access_config可空TEXT迁移，保留旧加密URL和历史strip_prefix字段。详细模型、up/down、全状态与验收见docs/preview-holes-design.md第17节。旧配置base路径的应用需调整为根路径。
- Status：已实现，真实Runner/Actions/工作流、CI测试DNS与发布证据以MEMORY.md为准，不宣称公网DNS/TLS验收。

## 平台级模板市场来源与索引 v2（2026-11-11，用户确认）

- Decision：超级管理员在平台管理“模板来源”添加多个官方/社区公开仓库，所有工作空间应用市场开箱可见可导入；空间来源（公有/私有）保持不变。平台来源与空间来源同表（`workspace_id=0`），但 CRUD/同步为独立实现：不加工作空间锁、不绑定仓库与凭据、请求不带 token、只允许 HTTPS（debug 构建可用 `FERRY_TEST_MARKETPLACE_HTTP=1` 放行回环 http 供验收脚本）。索引新增 `schemaVersion 2`（每包多版本、附带配置文件、README、元数据、digest），v1 继续兼容。
- Reason：用户要求类似 `helm repo add` 的平台级多源、模板以 Git 管理、平台级先只做公开仓库；参考 Helm/Nomad Pack/1Panel 的目录+版本结构，同步只读 CI 生成的单一 bundle 以避开匿名 API 限额。
- Alternatives：平台来源独立新表（重复空间来源的查询与条目存储，否决）；同步时逐文件请求目录（匿名 60 次/小时不足，否决）；平台级私有仓库与平台凭据（用户确认本期不做）。
- Scope/Tradeoffs：同步以 `sync_owner/sync_lease_until` 租约保证多实例只执行一次，后台每 60 秒调度到期来源，失败保留旧目录；`marketplace::recover` 不再处理平台来源。导入在一个事务里创建应用模板与附带配置模板，配置模板重名追加 `-2…-20`，应用模板 `fromTemplate` 改写为配置模板 ID，并记录 `origin_*`。未知 kind / `requires.ferry` 不满足 / 集群蓝图（P2 前）入库但不可导入。官方模板先放本仓库 `marketplace/`，协议稳定后再拆独立仓库。迁移内置一条默认官方来源（id=1，`7link-team/ferry` main `marketplace/index.json`，用户确认），可停用/删除且不再重建。迁移 `marketplace_platform` 仅新增列与索引，down 删除新增列。
- Status：P1 已在分支 `feature/marketplace-p1` 实现并本地验收（证据 `data/maintenance/marketplace-p1/`）；集群蓝图、拓扑变更、六个 HA 模板按 `docs/应用市场与集群模板设计.md` 后续分期。未合并 main、未发布。
