import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, Profiler } from "react";
import { createRoot, type Root } from "react-dom/client";
import { Conversation } from "./Conversation";
import * as apiModule from "../../shared/api/api";
import { conversationText } from "./resource-config";
import { LONG_TEXT_LIMIT } from "./LazyDetails";
import { i18n } from "../../shared/i18n/i18n";

type Row = Record<string, unknown> & { turns: Record<string, unknown>[] };
let host: HTMLDivElement, root: Root;
let row: Row;
let events: Record<string, unknown>[];
let posts: { path: string; body: Record<string, unknown> }[];
let mocked: ReturnType<typeof vi.spyOn>;

const permissions = new Set(["conversations.read", "conversations.run"]);
const turn = (id: string, state: string) => ({
  id, owner_id: "2", state, prompt: "first", attachments: [], decisions: {}, error: null,
  created_at: 1, updated_at: 1, agent_id: "3",
});
const reply = (id: string, turnId: string, prompt: string) => ({
  id, role: "assistant", turn_id: turnId, reply_to: null, agent_id: "3", sender_name: "Agent", owner_id: "2",
  prompt, attachments: [], targets: [], created_at: 2,
});
function conversation(kind: "private" | "group", turns: Record<string, unknown>[], messages: unknown[] = []): Row {
  return {
    id: "2", title: "Smooth", kind, state: "active", viewer_id: "2", owner_id: "2", can_manage: true,
    agent_id: "3", members: [], agents: kind === "group" ? [{ id: "3", name: "Agent", state: "enabled" }] : [],
    messages, turns, version: 1, revision_id: "1", members_version: "1",
    config: { name: "Agent", runtime: "codex", model: "m", cwd: "/tmp" },
    binding: { execution_user: "user", canonical_directory: "/tmp" },
  };
}

beforeEach(async () => {
  await i18n.changeLanguage("zh-CN");
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  vi.stubGlobal("IntersectionObserver", class { observe() {} disconnect() {} });
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  events = [];
  posts = [];
  mocked = vi.spyOn(apiModule, "api").mockImplementation(async (path, options) => {
    const url = String(path);
    if (url.includes("/events?")) return { items: events, next: events.length, has_more: false } as never;
    if (options?.method === "POST") {
      if (url.endsWith("/read")) return { ok: true } as never;
      posts.push({ path: url, body: JSON.parse(String(options.body)) });
      return { id: "99" } as never;
    }
    return structuredClone(row) as never;
  });
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  mocked.mockRestore();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

const textarea = () => host.querySelector("textarea") as HTMLTextAreaElement;
const type = async (value: string) => {
  const area = textarea();
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(area, value);
    area.dispatchEvent(new Event("input", { bubbles: true }));
  });
};
const submit = () => act(async () => {
  host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
});
const poll = () => act(async () => { await vi.advanceTimersByTimeAsync(1100); });
const button = (text: string) => [...host.querySelectorAll("button")].find((b) => b.textContent === text);

it("keeps the composer editable while a private reply runs and sends a queued message once it completes", async () => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  row = conversation("private", [turn("10", "running")]);
  await act(async () => root.render(<Conversation workspace="1" id="2" permissions={permissions} />));
  expect(textarea().disabled).toBe(false);
  await type("next question");
  const send = host.querySelector('form button[type="submit"]') as HTMLButtonElement;
  expect(send.disabled).toBe(false);
  expect(send.textContent).toContain("排队发送");
  await submit();
  expect(posts).toHaveLength(0);
  expect(host.querySelector("[data-composer-queue]")?.getAttribute("data-composer-queue")).toBe("waiting");
  expect(host.textContent).toContain(conversationText("queuedWaiting"));
  // Editing continues while queued; the latest draft is what gets sent.
  await type("next question, refined");
  expect(textarea().value).toBe("next question, refined");
  await poll();
  expect(posts).toHaveLength(0);
  row = conversation("private", [turn("10", "completed")]);
  await poll();
  await poll();
  expect(posts).toHaveLength(1);
  expect(posts[0]!.path).toMatch(/\/turns$/);
  expect(posts[0]!.body).toMatchObject({ prompt: "next question, refined" });
  expect(host.querySelector("[data-composer-queue]")).toBeNull();
  await poll();
  expect(posts).toHaveLength(1);
});

it("pauses a queued message when the reply does not finish, and offers send now or cancel", async () => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  row = conversation("private", [turn("10", "running")]);
  await act(async () => root.render(<Conversation workspace="1" id="2" permissions={permissions} />));
  await type("after the failure");
  await submit();
  row = conversation("private", [turn("10", "failed")]);
  await poll();
  await poll();
  expect(posts).toHaveLength(0);
  expect(host.querySelector("[data-composer-queue]")?.getAttribute("data-composer-queue")).toBe("paused");
  expect(host.textContent).toContain(conversationText("queuedPaused"));
  expect(textarea().value).toBe("after the failure");
  await act(async () => button(conversationText("queuedSendNow"))!.click());
  expect(posts).toHaveLength(1);
  expect(posts[0]!.body).toMatchObject({ prompt: "after the failure" });
});

it("cancelling the queue keeps the draft and sends nothing", async () => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  row = conversation("private", [turn("10", "running")]);
  await act(async () => root.render(<Conversation workspace="1" id="2" permissions={permissions} />));
  await type("maybe later");
  await submit();
  await act(async () => button(conversationText("queuedCancel"))!.click());
  row = conversation("private", [turn("10", "completed")]);
  await poll();
  await poll();
  expect(posts).toHaveLength(0);
  expect(textarea().value).toBe("maybe later");
});

it("group chats still send immediately while an agent replies", async () => {
  row = conversation("group", [turn("10", "running")]);
  await act(async () => root.render(<Conversation workspace="1" id="2" permissions={permissions} />));
  await type("@Agent hello");
  await submit();
  expect(posts.map((p) => p.path.split("/").at(-1))).toEqual(["messages"]);
  expect(host.querySelector("[data-composer-queue]")).toBeNull();
});

it("does not re-render the conversation when polling returns the same data", async () => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  row = conversation("private", [turn("10", "completed")], [reply("11", "10", "**hello**")]);
  let commits = 0;
  await act(async () => root.render(
    <Profiler id="chat" onRender={() => { commits += 1; }}>
      <Conversation workspace="1" id="2" permissions={permissions} />
    </Profiler>,
  ));
  await poll();
  const settled = commits;
  for (let i = 0; i < 5; i += 1) await poll();
  expect(commits).toBe(settled);
  row = conversation("private", [turn("10", "completed")], [reply("11", "10", "**hello** again")]);
  await poll();
  expect(commits).toBeGreaterThan(settled);
});

it("builds execution details only when opened and caps very large results", async () => {
  const large = "x".repeat(3 * 1024 * 1024);
  row = conversation("private", [turn("10", "completed")], [reply("11", "10", "done")]);
  events = [{ sequence: 1, turn_id: "10", event: { type: "platform_result", request_id: "r1", result: large } }];
  await act(async () => root.render(<Conversation workspace="1" id="2" permissions={permissions} />));
  const trigger = host.querySelector("button[data-execution-details]") as HTMLButtonElement;
  expect(trigger).not.toBeNull();
  expect(document.querySelector("[data-execution-panel]")).toBeNull();
  await act(async () => trigger.click());
  const details = document.querySelector("[data-execution-panel]")!;
  expect(details).not.toBeNull();
  expect(details.querySelector("pre")).toBeNull();
  // The readable view builds the raw result only when it is asked for.
  expect(details.querySelector("pre")).toBeNull();
  await act(async () => (details.querySelector("[data-raw-toggle]") as HTMLButtonElement).click());
  const pre = details.querySelector("[data-long-text] pre")!;
  expect(pre.textContent!.length).toBe(LONG_TEXT_LIMIT);
  expect(details.textContent).toContain(conversationText("truncatedNote"));
  await act(async () => ([...details.querySelectorAll("button")].find(item => item.textContent === conversationText("showAll")) as HTMLButtonElement).click());
  expect(details.querySelector("[data-long-text] pre")!.textContent!.length).toBe(large.length);
});
