import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { ConversationMembers } from "./ConversationMembers";
import { Conversation } from "./Conversation";
import * as apiModule from "../../shared/api/api";
import { conversationText as t } from "./resource-config";
import { i18n } from "../../shared/i18n/i18n";

let host: HTMLDivElement, root: Root;
beforeEach(async () => {
  await i18n.changeLanguage("zh-CN");
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

const agent = { id: "3", name: "Builder", kind: "agent" as const };
const at = Date.now() - 65_000;
const turn = (id: string, state: string, extra: Record<string, unknown> = {}) => ({
  id, agent_id: "3", state, error: "", created_at: at, updated_at: at + 42_000, decisions: {}, ...extra,
});
function panel(turns: ReturnType<typeof turn>[], activity: Map<string, Map<string, { name?: string; state?: string }>>, onJump = vi.fn()) {
  return (
    <ConversationMembers open onOpenChange={() => {}} selected="agent:3" onSelect={() => {}} title="Room"
      members={[agent]} turns={turns} connected ownerId="2" viewerId="2" primaryAgentId="3" workspace="1"
      permissions={new Set()} manage={null} executionDetails={null} activity={activity} onJumpToTurn={onJump} />
  );
}
const section = () => document.querySelector("[data-member-activity]") as HTMLElement;

it("shows what a running Agent is doing now, including a pending approval and elapsed time", async () => {
  const activity = new Map([["10", new Map([["a", { name: "ferry_request", state: "running" }]])]]);
  await act(async () => root.render(panel([turn("10", "running")], activity)));
  expect(section().querySelector("[data-activity-current]")!.textContent).toContain("ferry_request · running");
  expect(section().textContent).toContain(t("activityElapsed"));
  expect(section().textContent).toMatch(/1\s*分/);
  await act(async () => root.render(panel([turn("10", "running", { decisions: { r: { tool: "ferry_request", decision: null, sent: false } } })], activity)));
  expect(section().querySelector("[data-activity-current]")!.textContent).toContain(`${t("activityWaitingApproval")}ferry_request`);
});

it("lists the last five results newest first with duration, reason and call count", async () => {
  const turns = Array.from({ length: 7 }, (_, i) => turn(String(i + 1), i === 6 ? "failed" : "completed", i === 6 ? { error: "Codex 无法运行" } : {}));
  const activity = new Map([["7", new Map([["a", { name: "x" }], ["b", { name: "y" }]])]]);
  await act(async () => root.render(panel(turns, activity)));
  const rows = [...section().querySelectorAll("[data-activity-turn]")];
  expect(rows.map((row) => row.getAttribute("data-activity-turn"))).toEqual(["7", "6", "5", "4", "3"]);
  expect(rows[0]!.textContent).toContain(t("failed"));
  expect(rows[0]!.textContent).toContain("Codex 无法运行");
  expect(rows[0]!.textContent).toContain(`2 ${t("activityCalls")}`);
  expect(rows[0]!.textContent).toMatch(/42\s*秒/);
  expect(rows[1]!.textContent).toContain(`0 ${t("activityCalls")}`);
});

it("says plainly when the Agent has not run in this conversation", async () => {
  await act(async () => root.render(panel([], new Map())));
  expect(section().textContent).toContain(t("activityEmpty"));
  expect(document.body.textContent).toContain(t("memberStatusScope"));
});

it("clicking a past run closes the panel and scrolls to its message", async () => {
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  vi.stubGlobal("IntersectionObserver", class { observe() {} disconnect() {} });
  const scrolled: string[] = [];
  Element.prototype.scrollIntoView = function () { scrolled.push(this.getAttribute("data-turn-id") ?? ""); };
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { callback(0); return 0; });
  const row = {
    id: "2", title: "Room", kind: "private", state: "active", viewer_id: "2", owner_id: "2", can_manage: true, agent_id: "3",
    members: [], agents: [], version: 1, revision_id: "1", members_version: "1",
    messages: [{ id: "11", role: "assistant", turn_id: "10", reply_to: null, agent_id: "3", sender_name: "Builder", owner_id: "2", prompt: "done", attachments: [], targets: [], created_at: at }],
    turns: [{ ...turn("10", "completed"), owner_id: "2", prompt: "go", attachments: [], usage: null }],
    config: { name: "Builder", runtime: "codex", model: "m", cwd: "/tmp" },
    binding: { execution_user: "user", canonical_directory: "/tmp" },
  };
  vi.spyOn(apiModule, "api").mockImplementation(async (path) =>
    (String(path).includes("/events?") ? { items: [], next: 0, has_more: false } : structuredClone(row)) as never);
  await act(async () => root.render(<Conversation workspace="1" id="2" permissions={new Set(["conversations.read"])} />));
  const avatar = [...host.querySelectorAll("button")].find((b) => b.getAttribute("aria-label")?.includes("Builder"))!;
  await act(async () => avatar.click());
  const runRow = document.querySelector('[data-activity-turn="10"]') as HTMLButtonElement;
  expect(runRow).not.toBeNull();
  await act(async () => runRow.click());
  expect(document.querySelector("[data-member-activity]")).toBeNull();
  expect(scrolled).toEqual(["10"]);
});
