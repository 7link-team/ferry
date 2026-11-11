import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { ConversationMembers, memberWorkState } from "./ConversationMembers";
import { conversationText as t } from "./resource-config";
import * as apiModule from "../../shared/api/api";

it("distinguishes live work, approval, idle after failure and lost connection", () => {
  const turn = { id: "1", state: "running", decisions: {} };
  expect(memberWorkState([turn], true)).toBe("running");
  expect(memberWorkState([{ ...turn, state: "queued" }], true)).toBe("queued");
  expect(memberWorkState([{ ...turn, decisions: { a: { decision: null, sent: false } } }], true)).toBe("statusApproval");
  expect(memberWorkState([{ ...turn, state: "stopping" }], true)).toBe("stopping");
  expect(memberWorkState([{ ...turn, state: "failed" }], true)).toBe("statusIdle");
  expect(memberWorkState([turn], false)).toBe("statusUnknown");
});
it("gates direct chat and surfaces failure before opening a new conversation", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  const open = vi.fn(); const request = vi.spyOn(apiModule, "api").mockRejectedValue(Error("Permission changed"));
  const props = { open: true, onOpenChange: vi.fn(), selected: "agent:7", onSelect: vi.fn(), title: "Room", members: [{ id: "7", name: "Helper", kind: "agent" as const }], turns: [], connected: true, ownerId: "1", viewerId: "1", primaryAgentId: "7", workspace: "2", onOpenConversation: open, manage: null, executionDetails: null };
  try {
    await act(async () => root.render(<ConversationMembers {...props} permissions={new Set()} />));
    expect([...document.querySelectorAll("button")].some(button => button.textContent?.includes(t("startDirectChat")))).toBe(false);
    await act(async () => root.render(<ConversationMembers {...props} permissions={new Set(["conversations.create", "ai-agents.read", "ai-agents.use"])} />));
    const button = [...document.querySelectorAll("button")].find(button => button.textContent?.includes(t("startDirectChat")))!;
    await act(async () => button.click());
    expect(document.querySelector('[role="alert"]')?.textContent).toBe("Permission changed");
    expect(open).not.toHaveBeenCalled();
    request.mockResolvedValue({ id: "20" });
    await act(async () => button.click());
    expect(open).toHaveBeenCalledWith("20");
    expect(request).toHaveBeenLastCalledWith("/workspaces/2/conversations", expect.objectContaining({ body: JSON.stringify({ agent_id: "7", title: "Helper" }) }));
  } finally { await act(async () => root.unmount()); host.remove(); request.mockRestore(); }
});
