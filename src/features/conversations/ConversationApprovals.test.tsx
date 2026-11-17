import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { Conversation } from "./Conversation";
import { describeOperation, ExecutionTimeline } from "./TurnActivity";
import * as apiModule from "../../shared/api/api";
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
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});
const permissions = new Set(["conversations.read", "conversations.run"]);
const buttons = (text: string) => [...host.querySelectorAll("button")].filter((button) => button.textContent?.trim() === text);

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
  // Banner above the composer counts it and scrolls to it.
  const banner = host.querySelector("[data-pending-banner]")!;
  expect(banner.textContent).toContain(commonText("conversations:pendingBanner", { count: 1 }));
  const scrolled = vi.fn();
  (card as HTMLElement).scrollIntoView = scrolled;
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { callback(0); return 0; });
  await act(async () => (banner.querySelector("button") as HTMLButtonElement).click());
  expect(scrolled).toHaveBeenCalled();
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
  expect(posts[1]).toEqual([expect.stringMatching(/\/turns\/10\/decision-resolutions$/), { request_id: "req-1", action: "resumed" }]);
  expect(host.querySelector("[data-ended-decision]")).toBeNull();
  // The composer draft was not touched by the follow-up.
  expect(host.querySelector<HTMLTextAreaElement>("#conversation-message")?.value).toBe("");
});

it("summarises tool calls, platform requests and decisions readably, with raw data per item and a remembered view", async () => {
  const events = [
    { type: "tool", item_id: "t1", name: "commandExecution", state: "finished" },
    { type: "approval", request_id: "p1" },
    { type: "platform_result", request_id: "p1", call: { tool: "ferry_request", method: "POST", path: "/workflows" }, result: { status: 201, ok: true, data: { id: "7", name: "Weekly changelog" } } },
    { type: "platform_result", request_id: "p2", call: { tool: "ferry_request", method: "POST", path: "/workflows" }, result: { status: 400, ok: false, data: { message: "Invalid workflow graph: /nodes/0 (missing type)" }, guidance: "Fix the named field and send the request again." } },
  ];
  const decisions = { p1: { ...createWorkflow, decision: true, sent: true } };
  localStorage.removeItem("ferry.conversation.details-view");
  const render = () => act(async () => root.render(<ExecutionTimeline events={events} decisions={decisions} turnState="completed" usage={{ input_tokens: 5, output_tokens: 2 }} stateLabel="done" deciderName="Ann" />));
  await render();
  const text = host.textContent!;
  expect(text).toContain("commandExecution");
  expect(text).toContain(`201 · ${conversationText("platformOk")}`);
  expect(text).toContain(commonText("conversations:platformCreated", { name: "Weekly changelog #7" }));
  expect(text).toContain(`400 · ${conversationText("platformFailed")}`);
  expect(text).toContain("/nodes/0 (missing type)");
  expect(text).toContain("Fix the named field");
  expect(text).toContain(conversationText("decisionAllowed"));
  expect(text).toContain(commonText("conversations:decisionBy", { name: "Ann" }));
  expect(host.querySelectorAll("[data-platform-entry=failed]")).toHaveLength(1);
  expect(host.querySelectorAll("[data-raw-toggle]")).toHaveLength(4);
  expect(host.querySelector("pre")).toBeNull();
  await act(async () => buttons(conversationText("raw"))[0]!.click());
  expect(host.querySelector("[data-execution-timeline]")?.getAttribute("data-execution-timeline")).toBe("raw");
  expect(localStorage.getItem("ferry.conversation.details-view")).toBe("raw");
  await act(async () => root.unmount());
  root = createRoot(host);
  await render();
  expect(host.querySelector("[data-execution-timeline]")?.getAttribute("data-execution-timeline")).toBe("raw");
  localStorage.removeItem("ferry.conversation.details-view");
});

it("still renders when storage throws", async () => {
  vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => { throw Error("blocked"); });
  vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw Error("blocked"); });
  await act(async () => root.render(<ExecutionTimeline events={[]} decisions={{}} turnState="completed" usage={null} stateLabel="done" />));
  expect(host.querySelector("[data-execution-timeline=readable]")).not.toBeNull();
  await act(async () => buttons(conversationText("raw"))[0]!.click());
  expect(host.querySelector("[data-execution-timeline=raw]")).not.toBeNull();
});

it("describes operations in words and falls back to the tool name", () => {
  expect(describeOperation("ferry_request", { method: "DELETE", path: "/cron-jobs/4" }).title).toBe(
    `${commonText("conversations:operationDelete", { resource: conversationText("resourceCronJobs") })} /cron-jobs/4`);
  expect(describeOperation("ferry_request", { method: "PATCH", path: "/workflows/3", body: { name: "x", version: 2 } }).fields.at(-1)).toEqual([conversationText("fieldChanges"), "name"]);
  expect(describeOperation("ferry_request", { method: "POST", path: "/unknown/thing" }).title).toBe("POST /unknown/thing");
  expect(describeOperation("commandExecution", { command: "ls -la" }).title).toBe("commandExecution: ls -la");
});
