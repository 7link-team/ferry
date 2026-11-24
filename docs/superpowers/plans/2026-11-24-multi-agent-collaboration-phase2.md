# Multi-Agent Collaboration — Phase 2 (threads, run results, links) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Pipelines and workflows created or run from a room appear as threads in that room; run results come back to the thread and wake the Agent that started the run; deleting the room never deletes the resources.

**Architecture:** Conversation-side tables only (`ai_chat_threads`, `ai_chat_run_links`, `ai_chat_thread_items`). Links are recorded by the conversation layer from successful platform calls in `conversations/run.rs`; pipeline/workflow modules stay unaware of conversations. A periodic sweep posts finished runs into threads and queues a follow-up turn.

**Tech Stack:** Rust (axum, SeaORM; SQLite/Postgres/MySQL), React + TypeScript, Playwright.

**Spec:** `docs/superpowers/specs/2026-11-24-multi-agent-collaboration-design.md` sections 5–9. Phase 1 plan and ADR entry of 2026-11-24 give the landed interfaces (`room_agents`, `relay`, `groups::queue_turn`, sponsors, `held`).

## Global Constraints

- Same as phase 1 (idempotent migrations on 3 databases, no new dependencies, 6 locales, LF, explicit staging).
- Never add columns to `ai_chat_messages`: migration `chat_interactions` reads it through the entity. Thread membership lives in `ai_chat_thread_items`.
- Builder deviation from spec 6, recorded: no hook in each run `finish` path (pipelines finish through several paths, e.g. cancellation). A sweep every 5 s over unnotified run links is the single notifier.
- Visibility (user decision): thread cards and run summaries are visible to all room members; logs, run detail and the resource itself keep their own permissions.
- Only resources created from the room are `linked`; resources merely run from the room get a thread with `linked = false`.
- Deleting a room deletes its thread/link rows only; pipelines, workflows and runs are untouched.

## Review Focus

1. Agent runs the same pipeline twice from the room: one thread, two run cards, two follow-ups. Test in Task 3.
2. Run finishes while the server is down: the sweep posts it once after restart, never twice. Test in Task 3.
3. Sponsor of the triggering Agent became unavailable before the run finished: card is posted, follow-up turn is cancelled with `sponsorUnavailable` (phase 1 rule). Test in Task 3.
4. Member without pipeline read permission opens the thread: sees the card summary, gets 403/404 on details, no crash. Test in Task 6 browser script.
5. Room deleted, pipeline still exists: pipeline page shows no origin label; thread rows gone; run history intact. Test in Task 4.

---

### Task 1: Schema

**Files:** Create `crates/server/src/migration/room_threads.rs`; modify `migration.rs`, `entities.rs`. Test in `conversations/tests.rs`.

**Interfaces (produces):**
- `ai_chat_threads(id PK, workspace_id, conversation_id, kind TEXT 'pipeline'|'workflow', resource_id, linked SMALLINT, created_by, created_at)`; unique `(conversation_id, kind, resource_id)`; index `(workspace_id, kind, resource_id)`.
- `ai_chat_run_links(id PK = run id, workspace_id, conversation_id, thread_id, kind 'pipeline_run'|'workflow_execution', turn_id, chat_agent_id, notified_at NULL)`; index `(notified_at)`.
- `ai_chat_thread_items(message_id PK, thread_id)`.

- [ ] Test `room_threads_migration_is_idempotent_and_unique_per_resource`; run FAIL; implement; run PASS; commit `feat: add room thread and run link tables`.

### Task 2: Record threads and run links from room platform calls

**Files:** Create `crates/server/src/conversations/threads.rs`; modify `conversations/run.rs` (after `dispatch_as_agent` returns), `conversations.rs`.

**Interfaces:**
- `pub(super) async fn record(s, turn: &turns::Model, call: &Call, result: &Value) -> Result<()>`; only for room turns, only when `result.ok`:
  - `POST /pipelines` → thread(kind pipeline, resource = data.id, linked = true); `POST /workflows` likewise.
  - `POST /pipelines/{id}/runs` → thread (linked false unless exists) + run link (kind pipeline_run, run = data.id); `POST /workflows/{id}/executions` likewise (workflow_execution).
  - Each recording posts a `notice`-role message into the thread (`ai_chat_thread_items`) such as "Pipeline run #N started by Agent X".
- Failures to record never fail the platform call (log and continue).

- [ ] Tests: `creating_a_pipeline_from_a_room_links_a_thread`, `running_an_existing_pipeline_records_a_run_but_not_a_link`, `private_conversations_record_nothing`, `failed_calls_record_nothing`; FAIL → implement → PASS; commit `feat: link room-created pipelines and workflows as threads`.

### Task 3: Run results back to the room

**Files:** `conversations/threads.rs` (`sweep`), `conversations/run.rs` scheduler spawn (same pattern as `scheduler`), i18n 6 locales.

**Interfaces:**
- `pub async fn sweep(s: &AppState) -> Result<usize>`: for links with `notified_at` NULL whose run/execution is terminal (`succeeded|failed|cancelled|timed_out` per each module's states): set `notified_at` first (single winner), post a thread card message (role `notice`, stored message with state, duration, failing job/node name) and, when `chat_agent_id` != 0, queue a follow-up turn for that room Agent via `groups::queue_turn` (hop = original link hop + 1, held past the limit, prompt = summary + how to read logs with `ferry_request GET /pipeline-runs/{id}/logs` or `/workflow-executions/{id}/node-runs`).
- Spawned every 5 s alongside the conversation scheduler.

- [ ] Tests: `finished_run_posts_once_and_wakes_its_agent`, `sweep_after_restart_posts_exactly_once`, `followup_respects_hop_limit`, `followup_for_unavailable_sponsor_is_cancelled_at_claim`; FAIL → implement → PASS; commit `feat: post run results to room threads and wake the starting Agent`.

### Task 4: Thread API, deletion, resource origin

**Files:** `conversations/threads.rs`, `conversations/groups.rs` (`detail`, `MessageBody.thread_id`), `conversations.rs` (`remove`), `pipelines.rs`/`workflows.rs` detail untouched.

**Interfaces:**
- Room detail adds `threads: [{id, kind, resource_id, name, linked, latest_run: {id, state, number?} | null, unread}]` and per message `thread_id` (from items).
- `send_message` accepts optional `thread_id` (must belong to the room) and records the item.
- `GET /{workspace}/resource-conversations/{kind}/{id}` → rooms the viewer can see that link this resource (`linked = true`), `[{conversation_id, title}]`.
- Room delete removes its threads, run links and items in the same transaction.

- [ ] Tests: `thread_messages_stay_in_their_thread`, `resource_origin_lists_only_visible_rooms`, `deleting_a_room_keeps_pipelines_and_hides_the_link`; commit `feat: expose room threads and hide links when rooms are deleted`.

### Task 5: Agent guidance

**Files:** `conversations/groups.rs` (`roster` text), `platform_tools_catalog.json` (regenerate).

- Roster paragraph adds: pipelines/workflows created or run here are tracked as threads; results arrive as a new message; do not poll run status in a loop.
- [ ] Test extends `agents_relay_to_each_other…` context assertion; regenerate catalog with `python scripts/generate-platform-tools.py` and `--check`; commit `docs: tell room Agents how run results return`.

### Task 6: UI

**Files:** Create `src/features/conversations/RoomThreads.tsx` (+ test); modify `Conversation.tsx` (thread cards in feed, open panel), `room-agents.ts` (thread fields), `conversations.json` (6 locales); pipeline/workflow detail pages show "Created from conversation" via the origin endpoint (`src/app/pages/PipelinePages.tsx`, workflow editor header). Browser script `scripts/test-conversation-threads-ui.mjs`.

- Feed: one card per thread (resource name, latest run state, unread); messages with `thread_id` leave the main feed.
- Panel: right side panel ≥ 900 px (reuse ConversationMembers dialog pattern), full screen on phones; run cards, thread messages, composer posting with `thread_id`; detail links open the run page (permission errors shown inline).
- [ ] Unit tests for card/panel; browser acceptance 1440/900/390/320, en/zh/ms, dark, 200 % text, member without pipeline permission; commit `feat: show pipeline and workflow threads in rooms`.

### Task 7: Gates and release preparation

- [ ] `cargo test --workspace`, full vitest, `npm run build`, both room browser scripts, `generate-platform-tools.py --check`, `git diff --check`.
- [ ] ADR entry; MEMORY checklist.
- [ ] Push; green CI on the exact commit; then user acceptance; then version bump to 0.11.0-alpha (rename `docs/releases/v0.10.15-alpha.md` → `v0.11.0-alpha.md` and extend), tag after green CI.
