import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { Conversation, type ConversationSnapshot } from "./Conversation";
import { describeOperation, EndedDecisionCard, ExecutionTimeline, PendingDecisionCard, QuestionCard } from "./TurnActivity";
import { LONG_TEXT_LIMIT } from "./LazyDetails";
import * as apiModule from "../../shared/api/api";
import { observeSession, clearSession } from "../../shared/api/session";
import { commonText } from "../../shared/i18n/i18n";
import { conversationText } from "./resource-config";

const workflowBody = {
  name: "Weekly changelog",
  description: "Summarise commits",
  definition: JSON.stringify({ engine: "ferry-graph", schema: 1, nodes: [{ id: "a", type: "core.trigger.manual" }, { id: "b", type: "ai.agent.run" }], connections: {} }),
};
const createWorkflow = { tool: "ferry_request", input: { method: "POST", path: "/workflows", body: workflowBody }, decision: null, dispatching: false, sent: false };

function turn(id: string, extra: Record<string, unknown> = {}) {
  return { id, owner_id: "2", state: "running", prompt: "make a workflow", attachments: [], error: "", usage: null, decisions: {}, ...extra };
}
function chat(id: string, role: string, turnId: string | null, owner = "2") {
  return { id, role, reply_to: null, turn_id: turnId, agent_id: "1", sender_name: role === "user" ? "Ann" : "Agent", owner_id: owner, prompt: role === "user" ? "please" : "", attachments: [], targets: [], created_at: 1 };
}
function conversation(turns: unknown[], messages: unknown[], extra: Record<string, unknown> = {}) {
  return {
    id: "2", title: "Approvals", kind: "private", state: "active", viewer_id: "2", owner_id: "2", can_manage: true,
    members: [{ id: "2", name: "Ann" }, { id: "3", name: "Bob" }], agents: [], messages, turns, version: 1, revision_id: "1", members_version: "1",
    config: { name: "Agent", runtime: "codex", model: "", cwd: "/tmp" }, binding: { execution_user: "user", canonical_directory: "/tmp" }, ...extra,
  };
}

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  localStorage.clear();
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  clearSession();
});
const permissions = new Set(["conversations.read", "conversations.run"]);
const buttons = (text: string) => [...host.querySelectorAll("button")].filter((button) => button.textContent?.trim() === text);
it("opens compact actions without losing global tools and transfers focus into message search", async () => {
  const row=conversation([],[]);
  vi.spyOn(apiModule,"api").mockImplementation(async path => String(path).includes('/events?') ? {items:[],next:0,has_more:false} as never : structuredClone(row) as never);
  await act(async()=>root.render(<Conversation workspace="1" id="2" permissions={permissions} appActions={<button>Workspace tools</button>}/>));
  await act(async()=>host.querySelector<HTMLButtonElement>('[data-conversation-more]')!.click());
  const panel=document.querySelector('[data-conversation-actions]')!;
  expect(panel.textContent).toContain('Workspace tools');
  const search=[...panel.querySelectorAll<HTMLButtonElement>('button')].find(button=>button.getAttribute('aria-label')===conversationText('searchMessages'))!;
  await act(async()=>search.click());
  const field=[...host.querySelectorAll<HTMLInputElement>('input')].find(input=>input.getAttribute('aria-label')===conversationText('searchMessages'))!;
  expect(field).toBeDefined();
  // The dialog returns focus after it closes, which is asynchronous under load.
  await vi.waitFor(()=>expect(document.activeElement).toBe(field));
  expect(document.querySelector('[data-conversation-actions]')).toBeNull();
});

it("restores conversation content while its background refresh waits and clears it on access denial", async () => {
  const row = conversation([], [chat("1", "user", null)]);
  const snapshot: ConversationSnapshot = { id:"2", data:row as unknown as ConversationSnapshot["data"], events:[], cursor:0 };
  let reject: ((reason: Error) => void) | undefined;
  vi.spyOn(apiModule, "api").mockImplementation(() => new Promise((_resolve, fail) => { reject = fail; }));
  const changed = vi.fn();
  await act(async () => root.render(<Conversation workspace="1" id="2" permissions={permissions} initialSnapshot={snapshot} onSnapshotChange={changed} />));
  expect(host.textContent).toContain("Approvals");
  expect(host.textContent).toContain("please");
  expect(host.querySelector('[data-slot="loading-skeleton"]')).toBeNull();
  await act(async () => reject!(new apiModule.ApiError("Access denied", 403)));
  expect(host.textContent).not.toContain("please");
  expect(changed).toHaveBeenLastCalledWith(undefined);
});

it.each(['visibilitychange','focus','pageshow','online','session:renewed'])("recovers a suspended read on %s and ignores its late result without replacing the draft", async event => {
  const row = conversation([], []);
  let release!: (value: never) => void;
  let firstSignal: AbortSignal | null | undefined;
  let reads = 0;
  vi.spyOn(apiModule, "api").mockImplementation(async (path, options) => {
    if (String(path).includes('/events?')) return {items:[],next:0,has_more:false} as never;
    if (++reads === 1) { firstSignal = options?.signal; return new Promise<never>(resolve => {release=resolve;}); }
    return {...row,title:'Recovered conversation'} as never;
  });
  await act(async () => root.render(<Conversation workspace="1" id="2" permissions={permissions} initialDraft={{prompt:'Keep my draft',attachments:[]}} initialSnapshot={{id:'2',data:row as unknown as ConversationSnapshot['data'],events:[],cursor:0}} />));
  await act(async () => (event === 'visibilitychange' ? document : window).dispatchEvent(new Event(event)));
  expect(firstSignal?.aborted).toBe(true);
  expect(host.textContent).toContain('Recovered conversation');
  await act(async () => release({...row,title:'Stale response'} as never));
  expect(host.textContent).not.toContain('Stale response');
  expect(host.querySelector('textarea')?.value).toBe('Keep my draft');
});

it("retains messages during access renewal and recovers on the renewed event", async () => {
  const now=Date.now();
  observeSession({server_time_ms:now,access_expires_at_ms:now-1,refresh_expires_at_ms:now+60000,auto_renew:true,refreshable:true});
  const row=conversation([], [chat('1','user',null)]);
  let expired=true;
  vi.spyOn(apiModule,'api').mockImplementation(async path=>{
    if(String(path).includes('/events?')) return {items:[],next:0,has_more:false} as never;
    if(expired) throw new apiModule.ApiError('Renewing session',401);
    return {...row,title:'Renewed conversation'} as never;
  });
  await act(async()=>root.render(<Conversation workspace="1" id="2" permissions={permissions} initialSnapshot={{id:'2',data:row as unknown as ConversationSnapshot['data'],events:[],cursor:0}}/>));
  expect(host.textContent).toContain('please');
  expect(host.querySelector('textarea')).not.toBeNull();
  expired=false;
  await act(async()=>window.dispatchEvent(new Event('session:renewed')));
  expect(host.textContent).toContain('Renewed conversation');
  expect(host.textContent).not.toContain('Renewing session');
});

it("counts six actionable requests rather than turns or historical platform errors", async () => {
  const row = conversation([
    turn("10", { state: "completed", decisions: { a: createWorkflow, b: createWorkflow, c: createWorkflow, d: createWorkflow } }),
    turn("20", { decisions: { e: createWorkflow, f: createWorkflow } }),
    turn("30", { state: "failed" }),
  ], []);
  vi.spyOn(apiModule, "api").mockImplementation(async path => (String(path).includes("/events?")
    ? { items: [{ sequence: 1, turn_id: "30", event: { type: "platform_result", result: { ok: false, status: 413 } } }], next: 1, has_more: false }
    : structuredClone(row)) as never);
  await act(async () => root.render(<Conversation workspace="1" id="2" permissions={permissions} />));
  const tab = buttons(`${conversationText("pendingFilter")} · 6`)[0];
  expect(tab).toBeDefined();
  expect(host.querySelector("[data-pending-banner]")).toBeNull();
  await act(async () => tab!.click());
  expect([...host.querySelectorAll("article[data-turn-id]")].map(item => item.getAttribute("data-turn-id"))).toEqual(["10", "20"]);
});

it("dismisses an ended operation while another reply runs without sending a follow-up or losing a draft", async () => {
  let row = conversation([
    turn("10", { state: "completed", decisions: { old: createWorkflow } }), turn("20"),
  ], []);
  const posts: [string, unknown][] = [];
  vi.spyOn(apiModule, "api").mockImplementation(async (path, options) => {
    if (String(path).includes("/events?")) return { items: [], next: 0, has_more: false } as never;
    if (options?.method === "POST") {
      posts.push([String(path), JSON.parse(String(options.body))]);
      row = conversation([turn("10", { state: "completed", decisions: { old: { ...createWorkflow, resolved: "dismissed" } } }), turn("20")], []);
      return { ok: true } as never;
    }
    return structuredClone(row) as never;
  });
  const render = () => root.render(<Conversation workspace="1" id="2" permissions={permissions} initialDraft={{ prompt: "My draft", attachments: [] }} />);
  await act(async () => render());
  const dismiss = buttons(conversationText("dismissOperation"))[0];
  expect(dismiss).toBeDefined();
  expect(dismiss!.disabled).toBe(false);
  await act(async () => dismiss!.click());
  expect(posts).toEqual([[expect.stringMatching(/\/turns\/10\/decision-resolutions$/), { request_id: "old", action: "dismissed" }]]);
  expect(host.querySelector("[data-ended-decision]")).toBeNull();
  expect(buttons(conversationText("pendingFilter"))).toHaveLength(1);
  expect(host.querySelector<HTMLTextAreaElement>("#conversation-message")?.value).toBe("My draft");
  await act(async () => root.unmount());
  root = createRoot(host);
  await act(async () => render());
  expect(host.querySelector("[data-ended-decision]")).toBeNull();
});

it("keeps a failed dismissal actionable and shows the error in its own card", async () => {
  const row = conversation([turn("10", { state: "completed", decisions: { old: createWorkflow } })], []);
  vi.spyOn(apiModule, "api").mockImplementation(async (path, options) => {
    if (String(path).includes("/events?")) return { items: [], next: 0, has_more: false } as never;
    if (options?.method === "POST") throw Error("Offline");
    return structuredClone(row) as never;
  });
  await act(async () => root.render(<Conversation workspace="1" id="2" permissions={permissions} />));
  const dismiss = buttons(conversationText("dismissOperation"))[0];
  expect(dismiss).toBeDefined();
  await act(async () => dismiss!.click());
  expect(host.querySelector('[data-ended-decision="old"] [role="alert"]')?.textContent).toContain("Offline");
  expect(dismiss!.disabled).toBe(false);
});

it.each([true, false])("publishes the pending count after allow=%s and ignores an older in-flight poll", async allow => {
  vi.useFakeTimers();
  const row = conversation([turn("10", { decisions: { a: createWorkflow, b: createWorkflow } })], []);
  let releasePoll: ((value: never) => void) | undefined;
  let releaseRefresh: ((value: never) => void) | undefined;
  let reads = 0;
  vi.spyOn(apiModule, "api").mockImplementation(async (path, options) => {
    if (String(path).includes("/events?")) return { items: [], next: 0, has_more: false } as never;
    if (options?.method === "POST") {
      (row.turns[0] as ReturnType<typeof turn>).decisions = { a: { ...createWorkflow, decision: allow }, b: createWorkflow };
      return { ok: true } as never;
    }
    if (++reads === 2) return new Promise<never>(resolve => { releasePoll = resolve; });
    if (reads === 3) return new Promise<never>(resolve => { releaseRefresh = resolve; });
    return structuredClone(row) as never;
  });
  const changed = vi.fn();
  try {
    await act(async () => root.render(<Conversation workspace="1" id="2" permissions={permissions} onPendingChange={changed} />));
    const before = structuredClone(row);
    await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
    expect(releasePoll).toBeDefined();
    await act(async () => buttons(conversationText(allow ? "allow" : "deny"))[0]!.click());
    expect(changed).toHaveBeenLastCalledWith("2", 1);
    expect(buttons(`${conversationText("pendingFilter")} · 1`)).toHaveLength(1);
    expect(releaseRefresh).toBeDefined();
    await act(async () => releaseRefresh!(structuredClone(row) as never));
    await act(async () => releasePoll!(before as never));
    expect(host.querySelectorAll("[data-pending-decision]")).toHaveLength(1);
    expect(changed).toHaveBeenLastCalledWith("2", 1);
  } finally { vi.useRealTimers(); }
});

it("keeps approval summaries and raw controls in the same header row", async () => {
  for (const ended of [false, true]) {
    await act(async () => root.render(ended
      ? <EndedDecisionCard request="r" decision={createWorkflow} canAct busy={false} onContinue={() => {}} />
      : <PendingDecisionCard request="r" decision={createWorkflow} canDecide ownerName="Ann" busy={false} onDecide={() => {}} />));
    const raw = host.querySelector("[data-raw-toggle]")!;
    expect(raw.parentElement!.querySelector("[data-operation-title]")).not.toBeNull();
  }
});

it("shows a user question in the single pending filter without a duplicate banner", async () => {
  const question = { tool: "ferry_ask_user", input: { title: "Need input", question: "What next?" }, decision: null, dispatching: false, sent: false };
  const row = conversation([turn("10", { state: "succeeded", decisions: { "question-1": question } })], [chat("9", "user", "10")]);
  vi.spyOn(apiModule, "api").mockImplementation(async (path) => String(path).includes("/events?")
    ? { items: [], next: 0, has_more: false } as never : structuredClone(row) as never);
  await act(async () => root.render(<Conversation workspace="1" id="2" permissions={permissions} />));
  const card = host.querySelector('[data-user-question="question-1"]')!;
  expect(card).not.toBeNull();
  expect(host.querySelector("[data-pending-banner]")).toBeNull();
  await act(async () => buttons(`${conversationText("pendingFilter")} · 1`)[0]!.click());
  expect(host.querySelector('[data-user-question="question-1"]')).not.toBeNull();
});

it("accepts a custom question response without any fixed choice and preserves it on failure", async () => {
  const answer = vi.fn().mockRejectedValueOnce(new Error("Offline")).mockResolvedValue(undefined);
  const decision = { tool: "ferry_ask_user", input: { title: "Repository unavailable", question: "How should I proceed?", options: [{ id: "repair", label: "Repair connection" }], recommended_option_id: "repair" }, decision: null, dispatching: false, sent: false };
  await act(async () => root.render(<QuestionCard request="q" decision={decision} canAnswer busy={false} executing={false} onAnswer={answer} />));
  const suggested = host.querySelector<HTMLButtonElement>('[aria-pressed="false"]')!;
  await act(async () => suggested.click());
  expect(suggested.getAttribute("aria-pressed")).toBe("true");
  await act(async () => suggested.click());
  expect(suggested.getAttribute("aria-pressed")).toBe("false");
  const input = host.querySelector("textarea")!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(input, "Use my own alternative, not a fixed suggestion.");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  const submit = () => act(async () => { host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })); });
  await submit();
  expect(host.textContent).toContain("Offline");
  expect(input.value).toBe("Use my own alternative, not a fixed suggestion.");
  await submit();
  expect(answer).toHaveBeenLastCalledWith({ action: "continue", answer: input.value });
});

it("lets users pause or abandon without selecting anything or typing an answer", async () => {
  const answer = vi.fn().mockResolvedValue(undefined);
  const decision = { tool: "ferry_ask_user", input: { title: "Need input", question: "What next?" }, decision: null, dispatching: false, sent: false };
  await act(async () => root.render(<QuestionCard request="q" decision={decision} canAnswer busy={false} executing={false} onAnswer={answer} />));
  await act(async () => buttons(conversationText("questionPause"))[0]!.click());
  expect(answer).toHaveBeenLastCalledWith({ action: "pause", answer: "" });
  await act(async () => buttons(conversationText("questionAbandon"))[0]!.click());
  expect(answer).toHaveBeenLastCalledWith({ action: "abandon", answer: "" });
  await act(async () => root.render(<QuestionCard request="q" decision={decision} canAnswer={false} busy={false} executing={false} onAnswer={answer} />));
  expect(host.querySelector("textarea")).toBeNull();
  expect(host.textContent).toContain(conversationText("questionOwnerOnly"));
});

it("shows a waiting approval inline with a readable summary even before any reply text", async () => {
  const row = conversation([turn("10", { decisions: { "req-1": createWorkflow } })], [chat("9", "user", "10")]);
  const decided: unknown[] = [];
  vi.spyOn(apiModule, "api").mockImplementation(async (path, options) => {
    if (String(path).includes("/events?")) return { items: [], next: 0, has_more: false } as never;
    if (options?.method === "POST") decided.push([path, JSON.parse(String(options.body))]);
    return structuredClone(row) as never;
  });
  await act(async () => root.render(<Conversation workspace="1" id="2" permissions={permissions} />));
  const card = host.querySelector('[data-pending-decision="req-1"]')!;
  expect(card).not.toBeNull();
  expect(card.closest("details")).toBeNull();
  const title = card.querySelector("[data-operation-title]")!.textContent!;
  expect(title).toContain("/workflows");
  expect(title).toContain(conversationText("resourceWorkflows"));
  expect(card.textContent).toContain("Weekly changelog");
  expect(card.textContent).toContain("2 · core.trigger.manual, ai.agent.run");
  // The JSON stays one click away.
  expect(card.querySelector("pre")).toBeNull();
  await act(async () => (card.querySelector("[data-raw-toggle]") as HTMLButtonElement).click());
  expect(card.querySelector("pre")?.textContent).toContain('"path": "/workflows"');
  // The pending tab is the single navigation entry, without a duplicate composer banner.
  expect(host.querySelector("[data-pending-banner]")).toBeNull();
  await act(async () => buttons(`${conversationText("pendingFilter")} · 1`)[0]!.click());
  expect(host.querySelector('[data-pending-decision="req-1"]')).not.toBeNull();
  await act(async () => buttons(conversationText("allow"))[0]!.click());
  expect(decided).toEqual([[expect.stringMatching(/\/conversations\/2\/turns\/10\/decisions$/), { request_id: "req-1", allow: true }]]);
});

it("tells other members whose approval is awaited instead of offering buttons", async () => {
  const row = conversation([turn("10", { decisions: { "req-1": createWorkflow } })], [chat("9", "user", "10")], { kind: "group", viewer_id: "3" });
  vi.spyOn(apiModule, "api").mockImplementation(async (path) =>
    (String(path).includes("/events?") ? { items: [], next: 0, has_more: false } : structuredClone(row)) as never);
  await act(async () => root.render(<Conversation workspace="1" id="2" permissions={permissions} />));
  const card = host.querySelector('[data-pending-decision="req-1"]')!;
  expect(card.textContent).toContain(commonText("conversations:waitingFor", { name: "Ann" }));
  expect(buttons(conversationText("allow"))).toHaveLength(0);
  expect(host.querySelector("[data-pending-banner]")).toBeNull();
  await act(async () => buttons(`${conversationText("pendingFilter")} · 1`)[0]!.click());
  expect(host.querySelector('[data-pending-decision="req-1"]')).not.toBeNull();
  expect(host.querySelector("[data-pending-banner]")).toBeNull();
});

it("filters to turns needing action and continues an approval left when the reply ended", async () => {
  const ended = { ...createWorkflow, ended_undecided: true };
  let row = conversation(
    [turn("10", { state: "completed", decisions: { "req-1": ended } }), turn("20", { state: "completed" })],
    [chat("9", "user", "10"), chat("11", "assistant", "10"), chat("19", "user", "20"), chat("21", "assistant", "20")],
  );
  const posts: [string, Record<string, unknown>][] = [];
  vi.spyOn(apiModule, "api").mockImplementation(async (path, options) => {
    if (String(path).includes("/events?")) return { items: [], next: 0, has_more: false } as never;
    if (options?.method === "POST") {
      const body = JSON.parse(String(options.body));
      posts.push([String(path), body]);
      if (String(path).endsWith("/decision-resolutions"))
        row = conversation([turn("10", { state: "completed", decisions: { "req-1": { ...ended, resolved: "resumed" } } })], row.messages);
      return { id: "30" } as never;
    }
    return structuredClone(row) as never;
  });
  await act(async () => root.render(<Conversation workspace="1" id="2" permissions={permissions} />));
  const tab = buttons(`${conversationText("pendingFilter")} · 1`)[0]!;
  expect(tab).toBeDefined();
  await act(async () => tab.click());
  expect(host.querySelectorAll("article[data-turn-id]")).toHaveLength(1);
  expect(host.querySelector("article[data-turn-id]")?.getAttribute("data-turn-id")).toBe("10");
  const card = host.querySelector('[data-ended-decision="req-1"]')!;
  expect(card.textContent).toContain(conversationText("endedUndecided"));
  await act(async () => buttons(conversationText("continueTask"))[0]!.click());
  expect(posts[0]![0]).toMatch(/\/conversations\/2\/turns$/);
  expect(String(posts[0]![1].prompt)).toContain("/workflows");
  expect(String(posts[0]![1].prompt)).toContain("Weekly changelog");
  expect(String(posts[0]![1].prompt)).toContain("core.trigger.manual");
  expect(posts[1]).toEqual([expect.stringMatching(/\/turns\/10\/decision-resolutions$/), { request_id: "req-1", action: "resumed" }]);
  expect(host.querySelector("[data-ended-decision]")).toBeNull();
  // The composer draft was not touched by the follow-up.
  expect(host.querySelector<HTMLTextAreaElement>("#conversation-message")?.value).toBe("");
});

it("summarises tool calls, platform requests and decisions readably, with raw data per item and a remembered view", async () => {
  const events = [
    { type: "tool", item_id: "t1", name: "commandExecution", state: "running", input: { command: "git status --short" } },
    { type: "tool", item_id: "t1", name: "commandExecution", state: "finished" },
    { type: "approval", request_id: "p1" },
    { type: "platform_result", request_id: "p1", call: { tool: "ferry_request", method: "POST", path: "/workflows" }, result: { status: 201, ok: true, data: { id: "7", name: "Weekly changelog" } } },
    { type: "platform_result", request_id: "p2", call: { tool: "ferry_request", method: "POST", path: "/workflows" }, result: { status: 400, ok: false, data: { message: "Invalid workflow graph: /nodes/0 (missing type)" }, guidance: "Fix the named field and send the request again." } },
  ];
  const decisions = { p1: { ...createWorkflow, decision: true, sent: true } };
  const render = (viewerId = "2") => act(async () => root.render(<ExecutionTimeline key={viewerId} viewerId={viewerId} events={events} decisions={decisions} turnState="completed" usage={{ input_tokens: 5, output_tokens: 2 }} stateLabel="done" deciderName="Ann" />));
  await render();
  const text = host.textContent!;
  expect(text).toContain("commandExecution");
  expect(text).toContain("git status --short");
  expect(text).toContain("POST /workflows");
  expect(text).toContain(commonText("conversations:usageLine", { input: 5, output: 2 }));
  expect(text).toContain(`201 · ${conversationText("platformOk")}`);
  expect(text).toContain(commonText("conversations:platformCreated", { name: "Weekly changelog #7" }));
  expect(text).toContain(`400 · ${conversationText("platformFailed")}`);
  expect(text).toContain("/nodes/0 (missing type)");
  expect(text).toContain("Fix the named field");
  expect(text).toContain(conversationText("decisionAllowed"));
  expect(text).toContain(commonText("conversations:decisionBy", { name: "Ann" }));
  expect(host.querySelectorAll("[data-platform-entry=failed]")).toHaveLength(1);
  expect(host.querySelectorAll("[data-raw-toggle]")).toHaveLength(5);
  expect(host.querySelector("pre")).toBeNull();
  await act(async () => (host.querySelectorAll<HTMLButtonElement>("[data-raw-toggle]")[4]!).click());
  expect(host.querySelector("pre")?.textContent).toContain('"input_tokens": 5');
  await act(async () => buttons(conversationText("raw"))[0]!.click());
  expect(host.querySelector("[data-execution-timeline]")?.getAttribute("data-execution-timeline")).toBe("raw");
  expect(localStorage.getItem("conversation.details-view:2")).toBe("raw");
  expect(host.textContent).toContain('"output_tokens": 2');
  await act(async () => root.unmount());
  root = createRoot(host);
  await render();
  expect(host.querySelector("[data-execution-timeline]")?.getAttribute("data-execution-timeline")).toBe("raw");
  await render("3");
  expect(host.querySelector("[data-execution-timeline=readable]")).not.toBeNull();
});

it("still renders when storage throws", async () => {
  vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => { throw Error("blocked"); });
  vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw Error("blocked"); });
  await act(async () => root.render(<ExecutionTimeline viewerId="2" events={[]} decisions={{}} turnState="completed" usage={null} stateLabel="done" />));
  expect(host.querySelector("[data-execution-timeline=readable]")).not.toBeNull();
  await act(async () => buttons(conversationText("raw"))[0]!.click());
  expect(host.querySelector("[data-execution-timeline=raw]")).not.toBeNull();
});

it("describes operations in words and falls back to the tool name", () => {
  expect(describeOperation("ferry_request", { method: "DELETE", path: "/cron-jobs/4" }).title).toBe(
    `${commonText("conversations:operationDelete", { resource: conversationText("resourceCronJobs") })} · DELETE /cron-jobs/4`);
  expect(describeOperation("ferry_request", { method: "PATCH", path: "/workflows/3", body: { name: "x", version: 2 } }).fields.at(-1)).toEqual([conversationText("fieldChanges"), "name"]);
  expect(describeOperation("ferry_request", { method: "POST", path: "/unknown/thing" }).title).toBe("POST /unknown/thing");
  expect(describeOperation("commandExecution", { command: "ls -la" }).title).toBe("commandExecution: ls -la");
  expect(describeOperation("ferry_request", { method: "POST", path: "/cron-jobs", body: { action: "workflow", target_ref: "7", timezone: "Asia/Kuala_Lumpur", enabled: false, retries: 0, subject: "x".repeat(300), nested: { hidden: true } } }).fields).toEqual([
    ["action", "workflow"], ["target_ref", "7"], ["timezone", "Asia/Kuala_Lumpur"], ["enabled", "false"], ["retries", "0"], ["subject", "x".repeat(240)],
  ]);
});

it.each(["private", "group"])("counts pending requests and keeps historical failures in all messages in %s conversations", async (kind) => {
  const row = conversation([
    turn("10", { decisions: { a: createWorkflow, b: createWorkflow } }),
    turn("20", { state: "completed", decisions: { old: createWorkflow } }),
    turn("30", { state: "completed" }),
    turn("40", { state: "completed" }),
  ], [], { kind });
  vi.spyOn(apiModule, "api").mockImplementation(async (path) =>
    (String(path).includes("/events?") ? { items: [{ sequence: 1, turn_id: "30", event: { type: "platform_result", request_id: "failed", call: { method: "PATCH", path: "/workflows/7" }, result: { ok: false, status: 409, error: "Version conflict" } } }], next: 1, has_more: false } : structuredClone(row)) as never);
  await act(async () => root.render(<Conversation workspace="1" id="2" permissions={permissions} />));
  expect(host.querySelectorAll("[data-pending-decision]")).toHaveLength(2);
  expect(host.querySelectorAll("[data-ended-decision]")).toHaveLength(1);
  await act(async () => buttons(`${conversationText("pendingFilter")} · 3`)[0]!.click());
  expect([...host.querySelectorAll("article[data-turn-id]")].map((item) => item.getAttribute("data-turn-id"))).toEqual(["10", "20"]);
  await act(async () => buttons(conversationText("allMessages"))[0]!.click());
  const trigger = host.querySelector('article[data-turn-id="30"] button[data-execution-details]') as HTMLButtonElement;
  expect(trigger).not.toBeNull();
  await act(async () => trigger.click());
  const details = document.querySelector("[data-execution-panel]")!;
  expect(details.textContent).toContain("PATCH /workflows/7");
  expect(details.textContent).toContain("409");
  expect(details.textContent).toContain("Version conflict");
});

it("keeps an ended approval unresolved when its follow-up fails", async () => {
  const row = conversation([turn("10", { state: "completed", decisions: { a: createWorkflow } })], []);
  const posts: string[] = [];
  vi.spyOn(apiModule, "api").mockImplementation(async (path, options) => {
    if (String(path).includes("/events?")) return { items: [], next: 0, has_more: false } as never;
    if (options?.method === "POST") { posts.push(String(path)); throw Error("Offline"); }
    return structuredClone(row) as never;
  });
  await act(async () => root.render(<Conversation workspace="1" id="2" permissions={permissions} initialDraft={{ prompt: "My unsent draft", attachments: [] }} />));
  await act(async () => buttons(conversationText("continueTask"))[0]!.click());
  expect(posts).toHaveLength(1);
  expect(posts[0]).toMatch(/\/turns$/);
  expect(host.querySelector("[data-ended-decision]")).not.toBeNull();
  expect(host.textContent).toContain("Offline");
  expect(host.querySelector<HTMLTextAreaElement>("#conversation-message")?.value).toBe("My unsent draft");
});

it("shows updated identity and preserves denial history", async () => {
  await act(async () => root.render(<ExecutionTimeline viewerId="2" events={[
    { type: "platform_result", request_id: "p", call: { method: "PATCH", path: "/workflows/7" }, result: { status: 200, ok: true, data: { id: "7", name: "Updated flow" } } },
  ]} decisions={{ a: { ...createWorkflow, decision: false } }} turnState="completed" usage={null} stateLabel="done" deciderName="Ann" />));
  expect(host.textContent).toContain(commonText("conversations:platformUpdated", { name: "Updated flow #7" }));
  expect(host.textContent).toContain(conversationText("decisionDenied"));
  expect(host.textContent).toContain(commonText("conversations:decisionBy", { name: "Ann" }));
});

it("bounds the original request included in a Continue follow-up", async () => {
  const input = { method: "POST", path: "/workflows", body: { name: "Distinct operation", description: "x".repeat(LONG_TEXT_LIMIT * 2) } };
  const continued = vi.fn();
  await act(async () => root.render(<EndedDecisionCard request="a" decision={{ ...createWorkflow, input }} canAct busy={false} onContinue={continued} />));
  expect(continued).not.toHaveBeenCalled();
  await act(async () => buttons(conversationText("continueTask"))[0]!.click());
  const operation = continued.mock.calls[0]![0] as string;
  const title = describeOperation("ferry_request", input).title;
  expect(operation).toBe(`${title}\n${JSON.stringify(input, null, 2).slice(0, LONG_TEXT_LIMIT)}\n${conversationText("truncatedNote")}`);
  expect(operation).toContain("Distinct operation");
});
