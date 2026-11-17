import { expect, it, vi } from "vitest";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { conversationText } from "../../features/conversations/resource-config";

vi.mock("../router", () => ({
  href: () => "#",
  navigate: () => undefined,
  useRoute: () => ({ mode: "view", id: "1" }),
}));
vi.mock("../../features/conversations/Conversation", () => ({ Conversation: () => null }));
vi.mock("../../shared/api/resource", () => ({
  listResource: async () => ({
    items: [
      { id: "1", title: "Private chat", agent_name: "Agent", state: "active", kind: "private", pending: 2 },
      { id: "2", title: "Team room", agent_name: "", state: "active", kind: "group", pending: 1, unread: 3 },
      { id: "3", title: "Quiet", agent_name: "Agent", state: "active", kind: "private", pending: 0 },
    ],
    total: 3,
    page: 1,
    page_size: 50,
  }),
}));

it("badges private and group conversations that have approvals waiting for the viewer", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const { ConversationsWorkspace } = await import("./ConversationsWorkspace");
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<ConversationsWorkspace workspace="1" permissions={new Set(["conversations.read"])} form={undefined} />));
    const badges = [...host.querySelectorAll("[data-pending-badge]")];
    expect(badges.map((badge) => badge.textContent)).toEqual(["2", "1"]);
    expect(badges[0]?.getAttribute("aria-label")).toBe(`${conversationText("pendingFilter")} 2`);
    expect(badges[1]?.closest("a")?.textContent).toContain("Team room");
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});
