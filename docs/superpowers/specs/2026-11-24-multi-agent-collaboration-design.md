# Multi-developer, multi-Agent collaboration in one conversation

Date: 2026-11-24. Status: approved by the user 2026-11-24.

## Intent

Development is increasingly done by AI Agents, and each developer will run
several of them. When two developers work together (for example one builds
an API on Windows, the other an iOS app on a Mac), today every contract
change, failing call or "is it deployed yet?" has to be relayed by a human.
That multiplies communication cost and slows integration.

Goal: in one group conversation, developers and their Agents (each Agent on
its own Runner host) work in parallel, Agents talk to each other directly,
and pipelines/workflows created or run from the conversation are visible
there. Humans watch, step in at any time and decide only what is risky.

## Goals

1. An Agent can address one or more other Agents in the room. That queues a
   turn for each target, which resumes the target's own native session on its
   own host. Humans see every exchange.
2. Different Agents in the same room run in parallel (different hosts at the
   same time). One Agent still runs one turn at a time.
3. Pipelines and workflows created from the conversation are linked to it.
   Each linked resource, and each resource run from the conversation, has a
   thread inside the conversation: a card in the main feed, a side panel with
   run results and discussion.
4. When a run started from the conversation finishes, its result is posted to
   the thread and the Agent that started it gets a follow-up turn, so it can
   continue without a human relaying the result.
5. Deleting the conversation never deletes pipelines/workflows; the link is
   hidden.

## Non-goals

- Injecting messages into an Agent's running turn (Runner protocol `Input`
  only carries PlatformResult/Cancel/Decision). Messages wait for the next turn.
- Network connectivity between developer hosts (LAN, VPN, tunnels). Test
  environments are deployed by pipelines to reachable hosts; Ferry does not
  provide tunnels in this project.
- Cross-workspace rooms; real-time push (UI keeps HTTP polling).
- Changing pipeline/workflow execution engines.

## Anti-goals (failure even if it works)

- An Agent gaining permissions its sponsoring developer does not have, or
  bypassing approvals for running, publishing, deploying or deleting.
- Agents ping-ponging indefinitely and burning tokens with nobody noticing.
- The main conversation flooded by run logs.
- Deleting a conversation destroying CI/CD resources or their history.

## Decisions made with the user

- Thread inside the original conversation (not a separate sub-conversation,
  not inline in the main feed).
- Only pipelines/workflows created from the conversation are linked. Reusing
  an existing pipeline from the conversation puts the runs started from the
  conversation into a thread, but the pipeline itself is not linked.
- Conversation deletion keeps pipelines and hides the link.
- Agents may trigger another developer's Agent directly, without that
  developer accepting first. The developer is notified and can stop or take
  over. High-risk actions keep the existing approval rules.
- Technical approach A (turn relay), below.

## Approaches considered

- **A. Turn relay (chosen).** An Agent's message to another Agent queues a
  turn for the target, resuming its native session on its host. Reuses rooms,
  per-Agent bindings, native session resume, approvals and the scheduler.
  Cost: a message waits until the target's current turn ends; integration
  work is naturally turn-shaped (change, run, read result).
- **B. Live injection.** Extend the Runner protocol to push text into a running
  Agent. Most immediate, but CLI Agents (Claude Code, Codex) do not support
  mid-turn input; fragile per-CLI adaptation. Possible later for specific
  Agents on top of A.
- **C. Shared mailbox.** Agents call a blocking "wait for message" tool inside a
  turn. Holds sessions open, burns tokens, hits run-time limits.

## Current state (facts from the code)

- Rooms: a conversation with `ai_chat_members` rows. `ai_chat_agents` has one
  row per Agent with its own `binding` and `native_session_id`.
- Room turns come only from a human message's explicit `targets`
  (`conversations/groups.rs` send_message). Assistant messages have empty
  targets/mentions. `mentions` only feed unread counters.
- Room context sent to an Agent contains only prior **user** messages
  (`groups.rs` ~557). Agents do not see other Agents' replies.
- `run.rs::claim` serializes turns **per conversation**: two Agents in one room
  can never run at the same time. Global limit
  `policy.conversation_concurrent_runs` also applies.
- Agent platform calls run as the turn owner (`ai_turns.owner_id`, the sending
  human) narrowed by the Agent role. Every non-GET needs approval unless the
  conversation's approval mode is `assisted` (auto POST/PATCH of pipelines,
  workflows, groups, templates; runs/publish/delete stay manual) or `yolo`.
- Agents can already create/update/run pipelines and workflows via
  `ferry_request` (`/pipelines`, `/pipelines/{id}/runs`, `/workflows`,
  `/workflows/{id}/executions`, ...).
- Conversation delete is a hard delete of the conversation and its chat data
  (`conversations.rs` remove); it does not touch pipelines/workflows.
- Pipelines/workflows have no origin column. Runs/executions have
  `trigger_kind` / `triggered_by` (`workflow_executions` also `trigger_ref`).
  No generic finish hook; `pipelines/runs.rs` and `workflows/run.rs` `finish`
  only update state. UI progress is polling.
- Frontend: right-side panel pattern exists (`ConversationMembers.tsx`, Radix
  Dialog); special cards in `TurnActivity.tsx`; mobile drawer in
  `ConversationsWorkspace.tsx`.

## Design

### 1. Sponsors and permissions

- Add `added_by` (user id) to `ai_chat_agents`: the developer who brought the
  Agent into the room. Existing rows: set to the conversation owner.
- Every room-Agent turn, whether a human or an Agent addressed it, runs as
  the **target Agent's sponsor** (`owner_id = added_by`), narrowed by the
  target Agent's role. So the API Agent always works with the API developer's
  permissions and stays on the hosts that developer can use (placement runs
  under the turn owner's scope; mixing owners would rebind hosts and lose
  native sessions). Private conversations are unchanged. *(Builder's-check
  call.)*
- Approval mode becomes per room Agent, set only by its sponsor; the room
  setting is only the default for newly added Agents and never loosens
  another sponsor's Agent. *(Builder's-check call: a room-wide yolo set by
  the owner would otherwise approve writes under someone else's
  permissions.)*
- Sponsor loses eligibility (removed from the room, leaves the workspace, or
  can no longer run conversations/use the Agent): their Agents are paused;
  queued turns of those Agents end with a clear "sponsor unavailable" state;
  the room shows it. Removing a member from the room removes the Agents they
  sponsor. *(Builder's-check call.)*
- Approvals created in that turn go to the sponsor, as today for the turn
  owner. The room approval mode applies unchanged.
- The sponsor receives a notification (existing mention/unread mechanism)
  for Agent-triggered turns of their Agents, and can cancel the turn or pause
  their Agent (existing `ai_chat_agents.state`).

### 1a. Who adds Agents (decided with the user)

- Any developer member of the room can add their own Agents; they become
  that Agent's sponsor. The same Agent definition may appear several times,
  once per sponsor, shown as "Name (Sponsor)". The room
  Agent row id, not `agent_id`, identifies an Agent for addressing,
  approvals and turns; uniqueness moves to (conversation, agent_id, sponsor).
- Sub-Agents (decided with the user) are internal to one Agent's turn: the
  Agent's own CLI may spawn helpers (e.g. Claude Code / Codex subagents)
  while executing. They never join the room, are not addressable and need no
  Ferry membership. Requirements: the Runner must not block the CLI's native
  subagent capability, their work stays within the parent turn's sponsor,
  workdir and approvals, and their activity is shown inside the parent
  turn's execution timeline when the CLI reports it.
  Fact: the Runner starts Claude with an explicit `--tools` list
  (`crates/runner/src/agent/mod.rs` `claude_tools`) that omits the
  subagent tool, so Claude subagents are blocked today. Add the CLI's subagent
  tool (name per installed CLI version, Task/Agent) wherever the parent may
  run tools, keeping the same allow-list and workdir limits; Codex already
  has subagent model settings (`provider.rs`), verify they are not disabled.
  New Ferry tools (`ferry_message`) must also be added to that list.
- Removing an Agent from the room cancels its queued turns.

### 2. Agent-to-Agent messages

- New platform tool `ferry_message` (alongside ferry_discover / ferry_request /
  ferry_ask_user): `{ to: [room Agent member ids], text, thread_id? }`.
  It is a conversation action, not a workspace write, so it needs no approval.
- Server: inside the calling turn, append an assistant message with
  `targets = to` (and `thread_id`), then queue one `ai_turns` row per target
  as `groups.rs` does for humans, with owner from section 1. The tool result
  tells the caller the turns were queued; it does not wait for replies.
- Validation: targets must be active Agents of this room, not the caller;
  max 8 targets (same as humans); text size limit as for human messages.
- `ferry_discover` / tool description lists the room's Agents (member id,
  name, sponsor name, short role) so Agents can address each other.
- Room context given to an Agent includes the other Agents' replies and
  `ferry_message` texts (with sender name), not only user messages. Long
  histories are truncated by the existing context budget, newest first.

### 3. Parallel execution

- Change `claim` serialization in rooms from per-conversation to per room
  Agent (`ai_chat_turns.chat_agent_id`): a turn waits only for running or
  earlier queued turns of the same room Agent. Private conversations keep
  their current rule. The global `conversation_concurrent_runs` limit stays.
- Each turn reads room context at its start; messages arriving meanwhile are
  seen in that Agent's next turn.

### 4. Loop and cost guard

- Each Agent-triggered turn carries a hop count: 0 for a human-triggered
  turn, parent + 1 for `ferry_message` or run follow-up turns.
- Workspace/room limit `agent_relay_max_hops` (default 10). When exceeded the
  message is still posted, no turn is queued, and the room shows "Agents are
  waiting for a human" with a button that continues (resets the count).
- A room-level switch "Agent relay" (on by default for rooms) that any
  developer member can turn off; when off, `ferry_message` posts messages but
  queues no turns.
- Held-back deliveries are stored per message. "Continue" or turning relay
  back on queues them (hop count reset). A human message addressed to an
  Agent also starts a new chain at hop 0. *(Builder's-check call.)*

### 5. Threads and links

- New table `ai_chat_threads`: `id, workspace_id, conversation_id, kind
  (pipeline|workflow), resource_id, linked (bool: created from this
  conversation), created_by, created_at`. Unique on
  (conversation_id, kind, resource_id).
- Messages get a nullable `thread_id`. Main feed shows messages without a
  thread plus one card per thread (name, latest run state, who started it,
  unread count). Thread messages appear in the thread panel.
- New table `ai_chat_run_links`: `conversation_id, thread_id, kind
  (pipeline_run|workflow_execution), run_id, turn_id (nullable),
  chat_agent_id (nullable), notified_at`.
- Creating a link: when a platform call made from a conversation turn
  (or a conversation UI action) creates a pipeline/workflow
  (`POST /pipelines`, `POST /workflows`), record a thread with
  `linked = true`. When it starts a run (`POST /pipelines/{id}/runs`,
  `POST /workflows/{id}/executions`), ensure a thread exists (`linked =
  false` if the resource was not created here) and record a run link.
  The link is recorded by the conversation layer from the platform call's
  successful response, so pipeline/workflow modules need no conversation
  knowledge.
- Visibility (decided with the user): every room member sees thread cards
  and run summaries (state, duration, failing job/node), because Agents
  discuss results in the room anyway. Full logs, run details and the
  resource itself still require the viewer's own pipeline/workflow
  permissions.
- Resource pages: a linked pipeline/workflow shows "Created from conversation
  X" (link opens the thread) when the viewer can see that conversation.

### 6. Run results back into the conversation

- Add a small finish notifier called from pipeline run and workflow
  execution `finish` (after commit): if `ai_chat_run_links` has the run and
  `notified_at` is null, post a system card in the thread (state, duration,
  failing job/node, link to details) and set `notified_at`.
- If the run was started by an Agent turn (`chat_agent_id` set), queue a
  follow-up turn for that Agent (hop + 1, owner = sponsor) whose prompt is the
  result summary with how to fetch logs via `ferry_request`.
- Runs that end while the server is down: the notifier also runs from a
  periodic sweep of unnotified links whose runs are finished.

### 7. Deletion

- Conversation delete keeps its current semantics for chat data, and deletes
  `ai_chat_threads` / `ai_chat_run_links` rows of that conversation. Pipelines,
  workflows, runs and their history are untouched. Because the link lives
  only on the conversation side, the "Created from conversation" label
  disappears (hidden). Activity log keeps the audit trail.
- Deleting a pipeline/workflow leaves the thread and its messages; the card
  shows "deleted resource".

### 8. UI

- Main feed: Agent-to-Agent messages show "Agent A -> Agent B" with sponsor
  names; thread cards as in section 5; banner when relay is paused or hop
  limit hit.
- Thread panel: right side panel on desktop (reuse ConversationMembers panel
  pattern, non-blocking so the feed stays visible where width allows), full
  screen sheet on phones (reuse mobile drawer pattern). Shows run cards and
  thread messages, with a composer that posts into the thread (humans can
  address Agents there too).
- Members panel: each Agent shows host, sponsor, state, current turn; the
  sponsor can stop/pause it.
- All new text in all 6 locales.

### 9. Platform tool documentation

- Catalog entries and descriptions for `ferry_message`, threads, run links
  and the follow-up turn behaviour, so Agents know: address other Agents with
  `ferry_message`; start pipelines from the conversation; results come back as
  a new turn; no polling loops needed.

## Constraints

- SQLite, Postgres and MySQL migrations, idempotent (`migration/*.rs` pattern,
  e.g. `workspace_avatar.rs`). UTC storage.
- No new dependencies. Approval semantics, permission checks and immutable
  release rules unchanged.
- Mobile/H5 layout rules from the compact chat work stay valid.

## Left to the builder

- Exact table/column names and indexes; hop limit default plumbing into
  existing resource policy.
- Prompt wording of follow-up turns and tool descriptions.
- Whether the thread panel is a Dialog or a split pane on wide screens, as
  long as the feed stays usable.
- Test strategy (server unit/integration tests per section, browser tests at
  1440/900/390/320).
