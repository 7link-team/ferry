# Multi-Agent Collaboration — Phase 1 (sponsors, relay, parallel turns) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let several developers each bring their own Agents into one room, let Agents address each other directly, and let different Agents run in parallel on their own hosts, safely bounded by sponsor permissions, per-Agent approval modes and a hop limit.

**Architecture:** Extends the existing room model (`ai_chat_agents`, `ai_chat_turns`, `ai_turns`) in `crates/server/src/conversations/`. An Agent sends a relay message through the existing `ferry_request` tool to a new conversation endpoint; the server handles it inside the turn (no approval, no Runner protocol change) and queues turns for the targets. Turn ownership moves to the target room Agent's sponsor. Phase 2 (threads, pipeline/workflow links, run results, deletion rules, thread UI) gets its own plan after this phase lands.

**Tech Stack:** Rust (axum, SeaORM; SQLite/Postgres/MySQL), React + TypeScript (Vite, Vitest), Playwright browser scripts.

**Spec:** `docs/superpowers/specs/2026-11-24-multi-agent-collaboration-design.md` (sections 1, 1a, 2, 3, 4, 8 room parts, 9 relay parts).

## Global Constraints

- Migrations idempotent on SQLite, Postgres and MySQL; copy the `migration/workspace_avatar.rs` pattern (`has_column` check, `down` returns error, inline `#[tokio::test]`); register in `migration.rs` Migrator list after `account_email_verification`.
- UTC storage; no new dependencies; LF endings; `cargo fmt`; explicit staging only.
- Every room-Agent turn runs as the target room Agent's sponsor (`owner_id = added_by`); private conversations unchanged.
- Approval semantics unchanged except that the mode is read per room Agent; only its sponsor can change it; the room mode is only the default for newly added Agents.
- Defaults: `agent_relay_max_hops` = 10; relay targets per message max 8 (same as humans); relay text limit = human message limit (63 KiB budget as in `send_message`).
- All new UI text in all 6 locales (zh-CN, zh-TW, en-US, ja-JP, ko-KR, ms-MY).
- Builder deviation from spec, recorded: spec section 2 names a new tool `ferry_message`. This plan exposes the same contract as an endpoint reached through `ferry_request` (`POST /conversations/{id}/relay`), handled specially in `run.rs` without approval. Reason: no per-CLI tool-list changes in Claude/Codex/ACP/pi runtimes. Catalogue and tool description call it "relay message".

## Review Focus

1. Human addresses another developer's Agent: turn must run as that Agent's sponsor and stay on the sponsor's host binding (native session not lost). Test in Task 3.
2. Room owner sets the room to `yolo`: another sponsor's Agent must keep its own (stricter) mode. Test in Task 3.
3. Two Agents ping-pong: the 11th hop posts the message but queues nothing; Continue queues held targets with hop 0. Test in Task 6.
4. Sponsor removed from room while their Agent has queued turns: Agent removed, queued turns end `cancelled` with `conversations.sponsorUnavailable`. Test in Task 4.
5. Same Agent definition added by two developers: human message with legacy `targets=[agent_id]` is ambiguous and must be rejected, not routed to an arbitrary copy. Test in Task 2.

---

### Task 1: Schema for sponsors, per-Agent approval, hops and held deliveries

**Files:**
- Create: `crates/server/src/migration/room_agent_sponsors.rs`
- Modify: `crates/server/src/migration.rs` (module decl + Migrator entry), `crates/server/src/entities.rs` (`ai_chat_agents`, `ai_chat_turns`, `ai_chat_messages`)

**Interfaces:**
- Produces columns:
  - `ai_chat_agents.added_by: i64` (backfill = conversation `owner_id`), `ai_chat_agents.approval_mode: String` (backfill = current room mode via `approval::mode(conversation.config)`, default `"request"`).
  - `ai_chat_turns.hop: i64` (default 0), `ai_chat_turns.caller_chat_agent_id: i64` (default 0 = human).
  - No message column for held deliveries (an older migration reads `ai_chat_messages` through the entity, so new columns there break fresh installs). A held relay delivery is an `ai_turns` row with `state = "held"` plus its link; Continue switches it to `queued`.
  - Index `ai_chat_agents (conversation_id, agent_id, added_by)` replacing any uniqueness on `(conversation_id, agent_id)` if one exists.

- [ ] **Step 1: Write the failing migration test** `room_agent_sponsors_backfills_owner_and_mode` in the new module: create a SQLite DB at the previous schema with one room (owner 7, config `{"approval_mode":"assisted"}`) and one `ai_chat_agents` row; run the migration twice; assert `added_by` = owner, `approval_mode == "assisted"`, `hop` column exists, same sponsor cannot add the same Agent twice, another sponsor can.
- [ ] **Step 2: Run** `cargo test -p ferry-server room_agent_sponsors` — expect FAIL (module missing).
- [ ] **Step 3: Implement** `pub struct Migration` with idempotent column adds and backfill `UPDATE`s; update entity structs.
- [ ] **Step 4: Run** the test and `cargo test -p ferry-server migration` — expect PASS.
- [ ] **Step 5: Commit** `feat: add room Agent sponsor and relay columns`.

### Task 2: Developers add their own Agents; address room Agents by row id

**Files:**
- Modify: `crates/server/src/conversations/groups.rs` (`prepare`, `insert`, `MessageBody`, `send_message`), `crates/server/src/conversations/interactions.rs` (`members`)
- Test: `crates/server/src/conversations/tests.rs`

**Interfaces:**
- Produces routes: `POST /{workspace}/conversations/{id}/agents` body `{agent_id: Id}` → `201 {id: room_agent_id, agent_id, added_by, approval_mode, state}`; `DELETE /{workspace}/conversations/{id}/agents/{room_agent_id}` → 204. Any human room member with `conversations.run` may add an Agent they can use (`available(...)`); delete allowed for its sponsor or the room owner.
- `MessageBody` gains `#[serde(default)] agents: Vec<Id>` (room Agent row ids). Legacy `targets: Vec<Id>` (agent definition ids) still accepted when exactly one active row matches; otherwise `409 conversations.ambiguousTarget`.
- Room detail (`detail`) returns per Agent: `id` (room row), `agent_id`, `name`, `added_by`, `sponsor_name`, `approval_mode`, `state`, `binding host name`.
- Owner `members` edit keeps working; it only adds/removes the owner's own sponsored rows and never removes rows sponsored by others.

- [ ] **Step 1: Failing tests**: `member_adds_own_agent_as_sponsor` (non-owner member adds agent X → row `added_by` = member); `same_agent_added_by_two_sponsors` (two rows, both active); `legacy_target_ambiguous_rejected` (`targets=[X]` → 409 `conversations.ambiguousTarget`); `agents_field_targets_exact_row`; `non_member_cannot_add_agent` (403); `only_sponsor_or_owner_removes_agent`.
- [ ] **Step 2: Run** `cargo test -p ferry-server conversations::tests::` filters — FAIL.
- [ ] **Step 3: Implement** routes in `groups::routes`, target resolution `async fn resolve_targets(tx, conversation_id, body) -> Result<Vec<agents::Model>>` in `groups.rs`, new i18n key `conversations.ambiguousTarget` in the 6 server locale files.
- [ ] **Step 4: Run** tests — PASS; run full `cargo test -p ferry-server conversations` — PASS.
- [ ] **Step 5: Commit** `feat: let room members bring their own Agents`.

### Task 3: Turns run as the sponsor with the Agent's own approval mode

**Files:**
- Modify: `crates/server/src/conversations/groups.rs` (`send_message` turn insert, `execution`), `crates/server/src/conversations/approval.rs` (`decide_automatically`, `update`), `crates/server/src/conversations/run.rs` (`actor`)
- Test: `crates/server/src/conversations/tests.rs`

**Interfaces:**
- `send_message` sets `ai_turns.owner_id = room_agent.added_by` for each room-Agent turn; the human sender stays `ai_chat_messages.owner_id`.
- `groups::execution` writes `config["approval_mode"] = room_agent.approval_mode` instead of the room mode.
- `decide_automatically` compares against the room Agent's mode when the turn has a link.
- New route `PATCH /{workspace}/conversations/{id}/agents/{room_agent_id}` body `{approval_mode, version}`; only `added_by` may call (403 otherwise); 409 while that Agent has an active turn. Room-level `approval::update` now only sets the default for future Agents.

- [ ] **Step 1: Failing tests**: `human_message_to_other_sponsor_agent_runs_as_sponsor` (turn.owner_id == sponsor); `placement_uses_sponsor_scope` (bound host usable only by sponsor stays bound); `room_yolo_does_not_loosen_other_sponsor_agent` (owner sets room yolo; Agent B with `request` still produces a pending approval); `only_sponsor_changes_agent_approval_mode`.
- [ ] **Step 2: Run** — FAIL.
- [ ] **Step 3: Implement** as above.
- [ ] **Step 4: Run** tests + `cargo test -p ferry-server conversations` — PASS.
- [ ] **Step 5: Commit** `feat: run room Agents as their sponsor with their own approval mode`.

### Task 4: Sponsor eligibility

**Files:**
- Modify: `crates/server/src/conversations/interactions.rs` (`members`), `crates/server/src/conversations/run.rs` (`claim`)
- Test: `crates/server/src/conversations/tests.rs`

**Interfaces:**
- Removing a human from the room sets that user's sponsored room Agents `state = "removed"` and cancels their queued turns with `conversations.sponsorUnavailable`.
- `claim`: if the sponsor is not an active workspace member, lacks `conversations.run`, or can no longer use the Agent, set room Agent `state = "paused"`, turn `cancelled` with `conversations.sponsorUnavailable`. New i18n key in 6 locales.

- [ ] **Step 1: Failing tests**: `removing_member_removes_sponsored_agents_and_cancels_queue`; `claim_pauses_agent_when_sponsor_left_workspace`.
- [ ] **Step 2: Run** — FAIL. **Step 3: Implement.** **Step 4: Run** — PASS.
- [ ] **Step 5: Commit** `feat: pause room Agents whose sponsor is unavailable`.

### Task 5: Parallel turns per room Agent; Agents see relay messages

**Files:**
- Modify: `crates/server/src/conversations/run.rs` (`claim` serialization block, comment "Serialize room replies"), `crates/server/src/conversations/groups.rs` (`context_in`)
- Test: `crates/server/src/conversations/tests.rs`

**Interfaces:**
- Room turns wait only for running/stopping or earlier queued turns whose `ai_chat_turns.chat_agent_id` equals theirs. Private conversations keep the per-conversation rule. Global `conversation_concurrent_runs` unchanged.
- `context_in` includes messages with role `"user"` and `"relay"`; relay lines formatted `Agent {name} ({sponsor}) -> {target names}: {text}`.

- [ ] **Step 1: Failing tests**: `two_room_agents_claim_concurrently`; `same_room_agent_turns_stay_serialized`; `private_conversation_still_serialized`; `context_includes_relay_messages`.
- [ ] **Step 2: Run** — FAIL. **Step 3: Implement.** **Step 4: Run** — PASS.
- [ ] **Step 5: Commit** `feat: run different room Agents in parallel`.

### Task 6: Relay endpoint, hop limit, relay switch, held deliveries

**Files:**
- Create: `crates/server/src/conversations/relay.rs`
- Modify: `crates/server/src/conversations/run.rs` (`platform_call_with_slots`: intercept before approval), `crates/server/src/conversations/groups.rs` (routes), `crates/server/src/conversations.rs` (mod), `crates/server/src/accounts/conversation_policy.rs` (`agent_relay_max_hops`, default 10)
- Test: `crates/server/src/conversations/relay.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- `pub(super) async fn relay(s: &AppState, turn: &turns::Model, body: RelayBody) -> Result<Value>`; `RelayBody { agents: Vec<Id>, text: String }`. Called when a running room-Agent turn issues `ferry_request POST /conversations/{this id}/relay`; no approval; any other conversation id → 403.
- Inserts `ai_chat_messages` role `"relay"`, `agent_id` = caller's agent, `targets` = room Agent ids; for each target (active, not the caller) queues a turn as in Task 3 with `hop = caller hop + 1`, `caller_chat_agent_id = caller room Agent id`.
- If `hop > agent_relay_max_hops` or room config `relay_enabled == false`: turns are inserted with `state = "held"` (never claimed); returns `{queued: [], held: [...]}` so the caller knows. Otherwise `{queued: [ids], held: []}`.
- `POST /{workspace}/conversations/{id}/relay/continue` (any human room member): switches this room's `held` turns to `queued` with hop 0. `PATCH` room config `relay_enabled` via existing conversation config update (any developer member may set).
- Human `send_message` turns have `hop = 0`.
- Sponsor of each target gets the existing mention/unread signal (add sponsor id to the message `mentions`).

- [ ] **Step 1: Failing tests**: `relay_queues_turn_for_target_as_its_sponsor`; `relay_needs_no_approval`; `relay_rejects_self_and_inactive_targets`; `relay_other_conversation_forbidden`; `hop_limit_holds_delivery` (hop 11 → held); `relay_disabled_holds_delivery`; `continue_queues_held_with_hop_zero`; `relay_mentions_target_sponsor`.
- [ ] **Step 2: Run** `cargo test -p ferry-server relay` — FAIL. **Step 3: Implement.** **Step 4: Run** — PASS, plus `cargo test -p ferry-server conversations`.
- [ ] **Step 5: Commit** `feat: let room Agents relay messages to each other`.

### Task 7: Agent-facing guidance and room roster

**Files:**
- Modify: `crates/server/src/platform_tools_catalog.json` (relay endpoint entry), `crates/server/src/platform_tools.rs` (error hints for `conversations.ambiguousTarget`, `sponsorUnavailable`, held relay), `crates/server/src/conversations/run.rs` (`turn_prompt` adds room roster)
- Test: `crates/server/src/conversations/run/platform_tests.rs`

**Interfaces:**
- Room turns' prompt includes a roster: for each active room Agent `id, name, sponsor, host, state`, plus one paragraph: use `ferry_request POST /conversations/{id}/relay {agents, text}` to hand work to another Agent; results come back as new turns; do not poll.

- [ ] **Step 1: Failing tests**: `room_turn_prompt_lists_roster_and_relay_usage`; `catalogue_contains_relay_endpoint`.
- [ ] **Step 2–4:** run FAIL, implement, run PASS.
- [ ] **Step 5: Commit** `docs: describe Agent relay to room Agents`.

### Task 8: Allow CLI-native subagents inside a turn

**Files:**
- Modify: `crates/runner/src/agent/mod.rs` (`claude_tools`), check `crates/runner/src/agent/provider.rs` (Codex subagent settings)
- Test: `crates/runner/src/agent/tests.rs`

**Interfaces:**
- Wherever `claude_tools` grants tools (write_workdir or non-read-only), add the installed Claude CLI's subagent tool name (verify with `claude --help` / docs of the CLI version the Runner installs; `Task` or `Agent`) to both `--tools` and `--allowedTools`. Read-only requests unchanged. Codex: confirm no flag disables subagents; if one exists, enable it only under the same conditions.

- [ ] **Step 1: Failing test** `claude_tools_include_subagent_when_tools_allowed` and `claude_read_only_has_no_subagent`.
- [ ] **Step 2–4:** FAIL, implement, `cargo test -p ferry-runner agent` PASS.
- [ ] **Step 5: Commit** `fix: allow CLI subagents within permitted Agent turns`.

### Task 9: Room UI — sponsors, adding Agents, per-Agent approval, relay display and controls

**Files:**
- Modify: `src/features/conversations/ConversationMembers.tsx` (add own Agent, sponsor label, per-Agent approval mode for own Agents, remove), `src/features/conversations/Composer.tsx` (target picker uses room Agent ids, labels `Name (Sponsor)`), `src/features/conversations/Conversation.tsx` (render role `relay` as `A -> B` card; banner + Continue when held; relay switch in the More/actions panel), `src/shared/i18n/locales/*` conversation domain files (6 locales)
- Test: matching `*.test.tsx` next to each file; browser script `scripts/test-conversation-relay-ui.mjs` (pattern of existing conversation browser scripts)

**Interfaces:**
- Consumes Task 2/3/6 routes and detail fields.

- [ ] **Step 1: Failing unit tests**: member adds own Agent (POST called, list shows `Name (Sponsor)`); approval mode select only enabled for own Agents; composer sends `agents` ids; relay message renders sender, arrow, targets; held banner shows Continue and calls `/relay/continue`.
- [ ] **Step 2–4:** FAIL, implement, `npx vitest run src/features/conversations` PASS, `npm run build` PASS.
- [ ] **Step 5: Browser acceptance** with stateful fixtures at 1440/900/390/320, zh/en/ms, dark, 200% text: two sponsors, two copies of one Agent, relay card, held banner and Continue, per-Agent approval control; no horizontal overflow. Run `node scripts/test-conversation-relay-ui.mjs` — PASS.
- [ ] **Step 6: Commit** `feat: show room Agent sponsors and relay messages`.

### Task 10: Phase gate

- [ ] Full gates: `cargo fmt --check`, `cargo test --workspace`, `npm run build`, `npx vitest run --maxWorkers=2`, browser script; `git diff --check`.
- [ ] Update `MEMORY.md` T6 checkpoint and `ADR.md` (sponsor ownership, per-Agent approval, relay via endpoint, hop limit).
- [ ] Write the Phase 2 plan (spec sections 5–7, thread UI, run follow-up turns) from the landed interfaces.
