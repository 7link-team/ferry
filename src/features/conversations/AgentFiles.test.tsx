import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import * as apiModule from "../../shared/api/api";
import { fileType } from "./AttachmentView";
import { Conversation } from "./Conversation";
import { MemberActivity } from "./MemberActivity";
import { conversationText } from "./resource-config";

it("shows files an Agent produced on its reply with type, size and a download link", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  const reply = {
    id: "13", role: "assistant", reply_to: "11", turn_id: "12", agent_id: "1", sender_name: "Agent", owner_id: "2",
    prompt: "Here is the deck.", targets: [], created_at: 2,
    attachments: [
      { name: "周报.pptx", file_id: "501", mime: "application/vnd.openxmlformats-officedocument.presentationml.presentation", size: 2 * 1024 * 1024 },
      { name: "data.csv", file_id: "502", mime: "text/csv", size: 300 },
    ],
  };
  const row = {
    id: "2", title: "Files", kind: "private", state: "active", viewer_id: "2", owner_id: "2", agent_has_model: true,
    can_manage: true, members: [], agents: [], version: 2, revision_id: "1", members_version: "1",
    config: { name: "Agent", runtime: "codex", model: "", cwd: "" }, binding: null, host: { id: "31", name: "alpha" },
    messages: [
      { ...reply, id: "11", role: "user", reply_to: null, attachments: [], prompt: "做一个周报 PPT" },
      reply,
    ],
    turns: [{ id: "12", owner_id: "2", state: "completed", prompt: "做一个周报 PPT", attachments: [], error: "", usage: null, decisions: {} }],
  };
  const mocked = vi.spyOn(apiModule, "api").mockImplementation(async (path) => {
    if (String(path).includes("/events?")) return { items: [], next: 0, has_more: false } as never;
    return structuredClone(row) as never;
  });
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  try {
    await act(async () => root.render(<Conversation workspace="1" id="2" permissions={new Set(["conversations.read"])} />));
    const files = host.querySelector("[data-agent-files]")!;
    expect(files).not.toBeNull();
    const meta = [...files.querySelectorAll("[data-file-meta]")].map((node) => node.textContent);
    expect(meta).toEqual(["PPTX · 2.0 MB", "CSV · 1 KB"]);
    const links = [...files.querySelectorAll<HTMLAnchorElement>("a[download]")].map((a) => a.getAttribute("href"));
    expect(links).toEqual([
      "/api/workspaces/1/conversations/2/files/501?download=true",
      "/api/workspaces/1/conversations/2/files/502?download=true",
    ]);
  } finally {
    await act(async () => root.unmount()); host.remove(); mocked.mockRestore(); vi.unstubAllGlobals();
  }
});

it("labels file types and counts delivered files in the member activity", async () => {
  expect(fileType({ name: "report.PDF" })).toBe("PDF");
  expect(fileType({ name: "noext", mime: "image/png" })).toBe("PNG");
  expect(fileType({ name: "noext", mime: "application/octet-stream" })).toBe("");
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  try {
    await act(async () => root.render(
      <MemberActivity
        turns={[{ id: "12", state: "completed", created_at: 1, updated_at: 2, decisions: {} }]}
        items={new Map()}
        files={new Map([["12", 2]])}
        onJump={() => {}}
      />,
    ));
    expect(host.textContent).toContain(`2 ${conversationText("activityFiles")}`);
  } finally {
    await act(async () => root.unmount()); host.remove();
  }
});
