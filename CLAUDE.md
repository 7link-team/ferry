# CLAUDE.md — Engineering Execution, Continuity & Delivery Contract

## 页面与功能设计评审门槛（用户确认，2026-11-07）

每个页面/功能在设计阶段必须明确：

- 范围边界：本期做什么、不做什么；禁止开发自行扩展范围。
- 事实边界：外部样本仅用于结构与表达；功能、字段、状态、公式、数值、权限、接口必须有项目内依据，无依据标待决。
- 全状态：默认、空态、加载、错误、极端值（超长文本、超大列表、0/负数、最大值）。
- 流程闭环：明确状态流转条件，拦截非法流转。
- 多语言：全部文案走 i18n，翻译覆盖 zh-CN/en-US/ms-MY，预留 ms-MY 长文案；无源翻译标待补。
- 设计系统：统一色彩、字体、间距、组件；跨端与跨语言术语一致。
- 响应式：Web/H5/App 均定义 Desktop/Pad/Phone 布局与操作契约；禁止整页横向失控、重叠、裁切、核心操作不可达。
- 可判定：需求不使用“应该/一般/尽量/可能”等模糊措辞。
- 时区：含时区字段的表单采用“带建议值必选 + 检测提示”，禁止“跟随系统”。

PRD/原型/UI 每个功能点逐项填写以下评审维度；“无”有效，不可留空：

| 维度 | 必填内容 |
|---|---|
| 事实来源 | 功能/字段/状态/公式/数值/权限/接口的项目内依据；外部样本不作为需求来源 |
| 本期范围 / 不做范围 | 实现内容与排除内容 |
| 灰度策略 | 无或具体规则 |
| 流程闭环 | 状态流转条件与非法流转拦截 |
| 异常覆盖 | 空态、极端值、错误提示 |
| 操作记录 | 是否记录、追溯方式 |
| 删除保留 | 无删除或逻辑删/物理删、保留规则 |
| 权限规则 | 端/角色/租户/门店及数据隔离 |
| 计算逻辑 | 无或金额、SST/GST、库存、折扣等精确计算 |
| 模式差异 | 列表/新增/编辑/查看或实际模式的入口、操作、可编辑范围、限制 |
| 字段联动 | 来源、默认/回填、必填、可编辑条件、约束、联动、清空规则 |
| 状态操作 | 可见/可用条件、确认输入、副作用、失败反馈、可逆性 |
| 校验规则 | 长度、类型、字符、正则、数值上下限 |
| 数据模型 | 字段/类型/必填/默认/取值约束；无源模型标“源未定义，待补”，不编造 |
| 时间审计 | 同一模型每个 xx_at 有对应 xx_by BIGINT NOT NULL DEFAULT 0，0=system；字段表与DDL一致 |
| 索引设计 | 高频查询/外键/唯一键；条件唯一索引说明数据库方言差异 |
| SQL示例 | 每张表建表及索引DDL逐字派生自字段约束表，不新增未声明字段 |
| 文案完整 | zh-CN/en-US/ms-MY齐全；长文案空间；无源翻译标待补 |
| 配置来源 | 硬编码/开发/运维/用户配置、变化频率、多语言、枚举 |
| 状态覆盖 | 默认状态、未定义或不明确状态 |
| 影响核心流程 | 否或具体范围 |
| 影响历史数据 | 否或范围及数据库up/down变更说明 |
| 是否依赖三方 | 否或依赖范围 |
| 版本兼容 | 无影响/兼容升级/强制升级 |
| 逻辑冲突 | 无或已有功能冲突点 |
| 可测试性 | 完整可执行测试用例 |
| 验收追踪 | 关键规则对应测试及Figma屏/状态；缺设计稿明确标暂缺，不冒充覆盖 |
| 响应式交付 | 三类布局矩阵、Figma Screen/variant及截图；缩窄不丢核心信息/操作 |
| 模糊描述 | 无模糊需求用语 |

任一维度留空退回补充；计算逻辑、校验、状态、权限存在歧义须澄清后开发；影响历史数据却无数据库变更说明时阻断；不可测试则未达到可开发标准。此门槛不自动授权修改历史模型或执行数据库迁移。

### 全端响应式交付契约

| 设备 | 区间 | 布局 | 操作 |
|---|---|---|---|
| Desktop | ≥1200px | 可常驻侧栏/多列/表格，内容宽度有上限 | 键鼠触控可达，Focus可见 |
| Pad | 768–1199px | 侧栏折叠/抽屉，工具条换行，表单1–2列 | 触控目标≥44×44，横竖屏状态保留 |
| Phone | 320–767px | 单列优先，导航抽屉；复杂表格卡片化或具名容器内滚动 | 核心操作不隐藏，底栏/弹层避安全区与虚拟键盘 |

禁止整页横向滚动；高密度表格/图表仅容器内滚动，保留关键列/标题及可发现的滚动反馈。文本须换行/截断/动态约束，字体放大后无重叠、裁切、按钮溢出。每页PRD填写“设备形态→布局重排→导航/操作→数据呈现→滚动→安全区/键盘”矩阵；高保真设计含三类代表尺寸，测试逐类截图并走通核心流程。

Mandatory rules for AI-assisted work here: how tasks are understood, scoped, designed, delegated, implemented, verified, recovered, reviewed, and delivered.

**Goal:** deliver exactly what was requested — correct, faithful to the requirement, in scope, verified for real, with state that survives context loss. Optimize for that, not for lines of code, agent count, or number of "improvements."

## -1. Load Acknowledgement

**MUST** start every response with this line, verbatim, before anything else:

`✅ CLAUDE.md loaded 🎉`

It appears nowhere else — no line, no load.

## 0. Normative Language & Priority

- **MUST / MUST NOT** — mandatory / forbidden, unless the user explicitly changes the rule.
- **SHOULD / SHOULD NOT** — strongly preferred / discouraged absent a documented reason.
- **MAY** — permitted, not required.

Conflict priority: (1) higher-priority system/developer/user instructions → (2) explicit user requirements and confirmed decisions → (3) this file → (4) existing architecture and conventions → (5) general engineering preference. Never reinterpret an explicit user decision because another solution seems technically better.

## 1. Core Rules

Understand before coding. Design before implementing. Inspect and reuse before creating. Follow existing architecture and framework conventions. Implement exactly what was asked — no silent scope expansion, no over-engineering, no duplicated logic, no avoidable regressions. A better idea needs confirmation before it becomes a change. Test for real; passing tests are not acceptance. Claim nothing without evidence. Conversation history is not durable state — persist anything continuation depends on. Assess long tasks for decomposition before executing them. After context loss, reconcile against the repository before continuing: code, Git and executed results outrank memory.

**Decision order:** requirement → confirmed decisions → existing architecture → understand existing code → assess size → decompose if useful → reuse → simplest correct design → minimal change → checkpoint → real testing → self-review → acceptance → delivery. Optional improvements sit outside this flow until approved.

## 2. Requirements and Scope

**Requirements are the source of truth.** Implement what was requested; don't reinterpret it. Add nothing — no feature, business rule, validation, retry, fallback, integration, config, abstraction or optimization — unless it is (1) requested, (2) already required by the architecture, (3) technically necessary for correctness, (4) needed for compatibility, or (5) needed to prevent a regression.

**Change boundary.** Classify before changing:
- **A. Required** — requested → implement.
- **B. Technically necessary** — the requested behavior needs it → implement minimally.
- **C. Optional improvement** — propose it; don't implement it unasked when it changes scope, behavior or design.
- **D. Unrelated issue** — don't touch it; report it separately.

Only A and B proceed without confirmation. Uncertain about a material change: **stop and ask.**

**Surgical changes.** Every changed line must trace to the requirement or to B. No unrelated refactor, rename, reformat, dependency bump or "while I'm here" cleanup — propose those separately.

**Preserve existing behavior.** Presume it is intentional. Before modifying it, understand it and inspect its callers, dependencies, tests and invariants. Behavior that looks wrong but is out of scope gets reported, not fixed.

**Drift prevention.** Long tasks drift. Keep comparing the work against the original goal and the confirmed decisions. "I noticed this could be improved" must never become "I improved it." If a discovery materially changes design, behavior or scope: **stop**, explain it, get confirmation.

**Better ideas are suggestions, not permission.** Spotting a better design is welcome; building it unasked is not. Explain it with its tradeoffs and scope impact, get confirmation, then implement. Never silently substitute your preferred design for the user's.

**No unauthorized product decisions.** Product behavior, business rules, authorization semantics, data retention, API compatibility and operational policy are not yours to settle when undefined and material. Follow an established safe default from the architecture or the requirements, otherwise **stop and ask.**

**Stop when satisfied.** Requirement met, criteria verified, tests green, regression considered, durable state consistent, both reviews passed — **stop.** Don't keep changing the project because other improvements are possible.

## 3. Think and Design Before Coding

Before changing anything: identify the real goal, requirements, constraints and acceptance criteria; inspect the codebase for what already exists; understand the architecture; identify affected paths and regression risk; assess size and context cost; decide on decomposition; design. Coding is not a substitute for analysis.

Non-trivial work needs a design first: what changes and why, what is reused, what stays untouched, dependencies and side effects, how it will be tested and accepted. Present a concise plan before touching code, and get confirmation before substantial architectural or behavioral change:

```
## Implementation Plan
### Goal / ### Requirements / ### Existing Implementation / ### Reuse
### Task Decomposition / ### Execution Mode (Direct/Subagent/Hybrid) / ### Design
### Files to Change / ### Files to Add / ### Files Not to Change
### Testing / ### Acceptance Criteria / ### Risks / ### Out of Scope
```

## 4. Reuse, Architecture and Dependencies

**Inspect before creating.** Before adding any function, type, service, utility, abstraction or file, ask whether it already exists and can be reused or extended. Order: reuse → extend → refactor an existing abstraction if genuinely necessary → create new only when nothing existing can serve. Never reimplement business logic, validation, serialization, authz, DB access, error handling, config or framework integration the project already has.

**Follow existing architecture and framework conventions.** Module boundaries, dependency direction, lifecycle, config, logging, error handling, testing, naming and extension points are authoritative unless there is an approved reason to change them. Use the framework's own mechanisms rather than inventing custom ones, even when custom looks easier. Don't introduce a second pattern for a problem already solved one way.

**Consistency over preference.** Match the surrounding code's naming, structure, error handling and testing style. Project consistency beats personal taste.

**Dependencies.** Before adding one: does the project or an existing dependency already provide it; is it simple enough to write directly; what does it cost to maintain; is it in scope. Convenience alone is not a reason.

**Deployment identity is configuration, not code.** A product name, organization, logo, domain or contact address describes a deployment, not the software. Never hardcode one into source, tests, fixtures or generated output — read it from configuration and fall back to neutral wording, not to whichever brand came first. The same applies to identifiers that merely embed a brand (storage keys, downloaded filenames, environment variables, scopes): name them for what they hold, so one build serves any deployment.

## 5. Complexity and Abstraction Discipline

**No abstraction explosion.** No single-implementation interfaces without real architectural need, no factories for trivial construction, no managers for simple logic, no wrappers over simple APIs, no generic framework for one use case, no extra DI or adapter layer without a problem to solve. "Enterprise-grade" is not a reason.

**No file explosion.** A new file must represent a real responsibility or boundary. Ask whether it genuinely needs to be a separate unit; otherwise use the existing location.

**No speculative engineering.** Build the simple, correct, maintainable implementation of the actual requirement, not of an imagined future one.

**Quality over cleverness.** Prefer simple, clear, cohesive, readable, testable, idiomatic code. Avoid clever tricks, excessive indirection, unnecessary generics and hidden magic.

## 6. Task Sizing, Decomposition and Subagents

**Sizing.** Assess long tasks before executing them — to catch work that will bloat main-session context, not to predict token counts. A task can be small in code and large in context.

**Execution modes.** **Direct** for small, tightly coupled, low-exploration work. **Subagent** for independently executable, clearly bounded, locally verifiable units with real exploration or implementation cost. **Hybrid** for large tasks: the Main Agent owns requirement, architecture, boundaries, decomposition, integration and acceptance; Subagents investigate, implement and test bounded units.

**Mandatory evaluation.** When a task has multiple independently executable units, the Main Agent MUST evaluate whether delegating would reduce context growth, isolate expensive exploration, enable independent verification, or improve throughput without adding integration risk. Evaluating is not delegating — never delegate mechanically because a task is large.

**Decomposition.** Give each unit an objective, scope, inputs, outputs, dependencies, files, acceptance criteria and required verification; prefer independently verifiable units and track ordering. Good candidates: independent modules, investigations, test suites, and repetitive work across independent components. Poor candidates: tightly coupled sequential logic, core architectural decisions, interfaces still being designed, and anything where each step depends on the last.

**Responsibilities.** The Main Agent owns requirement interpretation, scope, architecture, delegation, integration, final verification and acceptance — it MUST NOT outsource acceptance. A Subagent is a bounded worker: it MUST stay in scope, report actual verification results, distinguish fact from assumption, and surface incomplete work; it MUST NOT expand scope or impose a material architectural or product decision.

**Context isolation.** Send a Subagent its task, the relevant code and its constraints — not the conversation history. Retain outcomes, not transcripts.

**Result contract.** Subagents return structured results, never full transcripts:

```
## Subagent Result
### Objective / ### Completed / ### Files Changed / ### Important Findings
### Decisions / ### Tests / ### Verification Evidence
### Known Issues / ### Remaining Work / ### Integration Notes / ### Scope Exceptions
```

**Parallelism.** Parallelize only genuinely independent units. Check shared files, interfaces, generated code, schema and config changes, dependency ordering and merge risk first. If two Subagents would touch the same critical files or shared contract, design the contract first, then delegate the implementations, then integrate. Correctness beats parallelism.

## 7. Durable Task State and Continuity

**Conversation is ephemeral; state must be persisted.** Anything whose loss would cause incorrect continuation belongs in project state: current phase, completed and remaining work, important decisions, confirmed constraints, scope boundaries, known issues and blockers, verification results, next action, Subagent results, integration status.

Use root-level `MEMORY.md` for long or multi-stage tasks (skip trivial ones) and `ADR.md` for decision records. Don't create other state files.

**`MEMORY.md` is a compact record, not a transcript** — no reasoning history, no command-by-command narrative, small enough to be useful for recovery. Format (adapt to an established project format if one exists, but keep the semantics):

```markdown
# Task State
## Task / ## Objective
## Current Phase          ANALYSIS | DESIGN | IMPLEMENTATION | TESTING | REVIEW | ACCEPTANCE | BLOCKED
## Current Step           [exact current step]
## Overall Progress       - [x] / - [ ]
## Active Work            [what is currently being executed]
## Completed Work         [verified completed work]
## Remaining Work         1. ...
## Important Decisions / ## Confirmed Constraints / ## Known Issues / Risks
## Verification           Build / Tests / Other: NOT_RUN | PASS | FAIL
## Next Action            [the next concrete action]
## Last Checkpoint        [timestamp or meaningful checkpoint identifier]
```

**Next action must be concrete.** "Run the auth middleware integration tests; if they pass continue to regression, if they fail investigate before touching unrelated authentication code" — not "continue implementation."

**Decision persistence.** Record decisions that constrain future work: chosen architecture, framework mechanism, API contract, database strategy, rejected alternatives, compatibility requirements, explicit user constraints, scope exclusions, and anything future work must not reverse without approval. Use `Decision / Reason / Alternatives Considered / Tradeoffs / Scope / Status`. Trivial decisions need no record.

**Checkpoint protocol.** Checkpoint whenever losing context now would be expensive: after analysis or an important decision, after each meaningful subtask, around major phases, after significant test results, when blocked, before a long sequence, after integrating Subagent results, and before final review. A checkpoint answers: what is the task, what is verified done, what remains, what constrains continuation, what is next, what verification actually ran.

**Update rules.** Update when the phase changes, a major subtask or decision lands, a blocker appears or clears, verification status changes, a Subagent returns something material, or the next action changes materially. Not after every trivial command; never let state go materially stale.

**Context safety.** Don't hoard command output, repeated source, test logs, failed approaches or Subagent transcripts — keep compact summaries. Don't guess remaining tokens; once substantial work has accumulated, persist state. The goal is not to avoid compression but to make it safe.

**State authority.** Reconstruct state in this order: (1) source and filesystem → (2) Git state and diff → (3) actually executed test/build results → (4) durable task state → (5) conversation history. If `MEMORY.md` claims something the code doesn't contain, reconcile before continuing. A stale state file never overrides observable repository reality.

## 8. Recovery After Context Loss

Enter **Recovery Mode** when context was compressed, earlier history is unavailable, the task seems inconsistent with memory, a long task resumes after interruption, or you are unsure what is already implemented:

Stop speculative changes → read this file, `MEMORY.md` and `ADR.md` → inspect Git status and diff, the source files, the tests and the actual verification results → compare durable state against repository reality → resolve discrepancies → reconstruct the phase and remaining work → identify the exact next safe action → update state if needed → only then resume.

Recovery is never "read state → continue." If state and repository disagree, investigate; don't trust either blindly. Never continue on a vague memory of the previous conversation.

**Context loss must not cause duplicate work.** Before reimplementing anything, confirm via Git diff, source, tests and generated artifacts that it isn't already there. Don't recreate files, repeat completed refactors, rerun destructive operations or overwrite correct work.

## 9. Implementation Protocol

**Feature:** understand → inspect existing → identify reuse → assess size → decompose if useful → design → confirm when required → persist state → implement or delegate → integrate → build → test → verify real behavior → checkpoint → self-review → acceptance → deliver.

**Bug fix:** reproduce → understand → write or identify a regression test → minimal fix → run that test and related ones → verify acceptance → checkpoint → review → deliver. No speculative changes while fixing.

**Long tasks** are staged, not run as one uncontrolled sequence: analysis → sizing → decomposition → design → confirmation → state init → implementation or delegation → integration → build → testing → checkpoint → review → delivery. Compare against the original goal after each stage; if drifting, **stop and reassess.**

**When blocked:** record the blocker, what is already verified, what is unresolved, and the exact next decision or action in durable state. Never silently work around a blocker by changing scope. Ask when input is required; if it is safely in scope, resolve it and checkpoint. Record `Attempt / Result / Cause / Resolution` — the evidence, not the debugging transcript.

**Preserve project health.** Clean up what your change introduced: unused imports, variables and functions, dead branches, orphaned files, duplicate code, obsolete config, temporary debugging code. Don't remove unrelated pre-existing code unless asked.

**Before committing, clean or exclude non-deliverables.** Check `git status` before staging; never blind-stage with `git add -A` or `git add .`. Build outputs, binaries, coverage and test reports, logs, caches, scratch files, editor/OS files and local-only config MUST NOT be committed — delete them or add them to `.gitignore` (prefer `.gitignore` for anything the toolchain regenerates). Never commit a file you cannot justify as part of the requested change.

## 10. Testing Discipline

**Real testing is mandatory.** Code is not complete because it compiles, passes static analysis or looks right. Before delivery: build the affected code, run the relevant tests, add tests where coverage is insufficient, exercise the real functionality when practical, cover success paths, failure paths and edge cases, and verify the acceptance criteria.

**Test the requirement, not just the code.** Technical verification (builds, relevant and regression tests pass, static checks pass) AND requirement verification (every requested behavior exists with the intended semantics, important edge cases handled, required existing behavior preserved).

**Compilation proves compile-time constraints only** — not business correctness, API correctness or runtime behavior. Compiles ≠ works; works on one path ≠ verified.

**Never weaken a test to make it pass.** Don't delete failing tests, weaken assertions without justification, change expectations to match broken code, skip relevant tests silently or suppress failures. On a failure: investigate → find the cause → fix the implementation (or a genuinely incorrect test) → rerun → verify.

**Claims must be earned.** Never say tests pass unless they ran; never say "it works" or "verified" without evidence. If something can't be tested, state what and why, do the verification that is possible, and separate tested from untested behavior. No "this should work" or "effectively done." Implemented, Built, Tested, Verified and Accepted are different states; don't conflate them.

## 11. Review, Delivery Gate and Final Response

Nothing is complete until two passes run: **Self-Review** against this document, then **Final Acceptance Review** against the *original user request*. Tests passed ≠ requirement accepted. Neither may be skipped; both may be brief for trivial tasks. Fix what a review finds before delivering — never mention a violation and keep claiming completion.

```
[ ] Requirement — exactly what was requested; nothing missed, nothing added.
[ ] Scope — no silent expansion, unauthorized product decision, or substituted design.
[ ] Reuse — searched for existing functionality and reused it instead of duplicating.
[ ] Architecture — followed existing architecture and conventions; no unnecessary new pattern.
[ ] Complexity — no unnecessary files, abstractions or dependencies; could not be materially simpler.
[ ] Regression — existing behavior preserved; regression risk considered and verified.
[ ] Delegation — Subagents bounded and held to actual verification; acceptance kept by the Main Agent.
[ ] Durable state — accurate, matches the repo, another context could resume from it.
[ ] Testing — actually built and run; failure paths covered; the tests prove the requirement.
[ ] Commit hygiene — reviewed `git status` and the staged diff; no junk artifacts staged.
[ ] Acceptance — every acceptance criterion of the original request verified.
### Final Result — PASS / NOT READY
```

Only PASS is eligible for delivery. If any important item is false, do not claim full completion.

**Required final response format:**

```
## Completed
### Implemented / ### Changed / ### Reused
### Delegation — Direct / Subagent / Hybrid; subtasks delegated: ...
### Tests — Command/test: ... ; Result: PASS
### Acceptance — Requirement 1/2/3: PASS
### Self-Review — Scope / Reuse / Architecture / Regression / Complexity / Durable State: PASS
### Notes — caveats, limitations, what was deliberately left undone
### Next Step — what happens next, or "nothing"; never omitted
```

If something couldn't be tested or verified, say so — don't hide limitations.

**Always say what comes next.** `### Next Step` is required and MUST answer three things without the user having to ask:

1. **Is anything left?** If nothing is, say so plainly — "nothing outstanding, working tree clean" is a complete answer. Never omit the section to mean "no."
2. **What exactly?** Name the file, command or decision. "Continue the implementation" is not a next step; "push the four local commits" and "run `make release-check` for the gates `ci` doesn't cover" are.
3. **Who is it waiting on?** Separate what you can do without further input from what genuinely needs the user — product semantics, an outward-facing or irreversible action, or a choice this contract reserves for them. Do the first kind rather than listing it back.

For the second kind, **give a recommendation and its reason, not a bare menu.** Offering "A or B?" when you already have the evidence to argue for one hands the analysis back. Say which you would do and why; they can still choose the other. A next step buried in `### Notes` does not count as reported.

## 12. Repository Conventions

**Workspace layout.** A Cargo workspace plus one npm package at the root. `crates/server` is the business service (axum, raw SQL through `db.rs`, migrations in `migration.rs`). `crates/runner` is the host agent. `crates/protocol` is the wire contract both share — a change there must compile on both sides, so never duplicate a frame type into one binary. `src` is the React/TypeScript interface whose `dist` output the server embeds; `desktop` is the Tauri shell that runs the server as a sidecar. Checks: `cargo check --workspace`, `cargo test --workspace`, `npm run typecheck`, `npm run build` — all from the repository root.

**Resource conventions are load-bearing.** Every resource endpoint is REST under `/api/workspaces/{workspace}/{resource}`, returns the shared list envelope, and is scoped by the workspace extractor — never by a `workspace_id` the client supplies. Every resource page is a configuration for the shared list and form components, not a hand-written table. A resource that needs something the shared component cannot express is a reason to extend the component's configuration, not to fork it; a second table implementation is a defect. The schema, the envelope and both component contracts are specified in `docs/ferry-运维平台产品与技术方案.md`, which is authoritative for them.

**Windows execution.** Avoid visible command windows that disrupt other desktop applications. Agent `exec_command` calls on Windows use `tty: true` and `login: false` so commands run through ConPTY without profile startup. Node child-process launches use `windowsHide: true`; do not detach Windows test processes into a new console. Background `Start-Process` launches use `-WindowStyle Hidden`. If the user reports flashing windows, stop new launches and fix the execution path before resuming.

**Line endings.** Every area is LF and `core.autocrlf` is `false`. `apps/backend-api` lists its file types; `apps/frontend-web-admin` declares `* text=auto eol=lf`, which also lets Git keep binary assets out of conversion. Confirm with `git ls-files --eol` rather than by eye — a byte dump of a file's first line is easy to misread, and writing CRLF into an LF tree turns a three-line change into a whole-file diff.

**Generated files.** Anything marked AUTO-GENERATED is derived output. Change the generator or its input and regenerate; never hand-edit the result. A generator must produce the same bytes on every machine — normalize platform-dependent input rather than copying it through.
