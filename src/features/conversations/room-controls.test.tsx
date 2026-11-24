import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import * as apiModule from "../../shared/api/api";
import { ConversationMembers } from "./ConversationMembers";
import { conversationText as t } from "./resource-config";
import { HeldRelays, RoomControls } from "./room-controls";

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  vi.restoreAllMocks();
});

it("adds the viewer's own Agent and switches relay for the room", async () => {
  const request = vi.spyOn(apiModule, "api").mockImplementation(async (path: string) =>
    path.includes("ai-agents") ? { items: [{ id: "7", name: "Codex", state: "enabled", visibility: "workspace" }, { id: "8", name: "Private", state: "enabled", visibility: "private" }] } : {});
  const changed = vi.fn(async () => {});
  await act(async () => root.render(<RoomControls workspace="2" endpoint="/workspaces/2/conversations/9" relayEnabled canRun onChanged={changed} />));
  const select = host.querySelector<HTMLSelectElement>("select")!;
  expect([...select.options].map(option => option.textContent)).toEqual([t("addMyAgent"), "Codex"]);
  await act(async () => { select.value = "7"; select.dispatchEvent(new Event("change", { bubbles: true })); });
  const add = [...host.querySelectorAll("button")].find(button => button.textContent === t("addMyAgent"))!;
  await act(async () => add.click());
  expect(request).toHaveBeenCalledWith("/workspaces/2/conversations/9/agents", { method: "POST", body: JSON.stringify({ agent_id: "7" }) });
  await act(async () => host.querySelector<HTMLInputElement>('input[type="checkbox"]')!.click());
  expect(request).toHaveBeenCalledWith("/workspaces/2/conversations/9/relay", { method: "PATCH", body: JSON.stringify({ enabled: false }) });
  expect(changed).toHaveBeenCalledTimes(2);
});

it("continues held relays", async () => {
  const request = vi.spyOn(apiModule, "api").mockResolvedValue({ queued: 2 });
  const changed = vi.fn(async () => {});
  await act(async () => root.render(<HeldRelays endpoint="/workspaces/2/conversations/9" onChanged={changed} />));
  expect(host.textContent).toContain(t("relayHeld"));
  await act(async () => host.querySelector("button")!.click());
  expect(request).toHaveBeenCalledWith("/workspaces/2/conversations/9/relay/continue", { method: "POST" });
  expect(changed).toHaveBeenCalled();
});

it("lets only the sponsor change an Agent's approval mode and remove it", async () => {
  const onApprovalMode = vi.fn(async () => {});
  const onRemoveAgent = vi.fn(async () => {});
  const props = {
    open: true, onOpenChange: vi.fn(), selected: "agent:102", onSelect: vi.fn(), title: "Room", turns: [], connected: true,
    ownerId: "1", viewerId: "2", primaryAgentId: "7", workspace: "2", permissions: new Set<string>(), manage: null, executionDetails: null,
    onApprovalMode, onRemoveAgent,
  };
  const agent = { id: "102", name: "Codex (Bo)", kind: "agent" as const, sponsor: "Bo", approvalMode: "request" };
  await act(async () => root.render(<ConversationMembers {...props} members={[{ ...agent, canEditMode: false, canRemove: false }]} />));
  expect(document.body.textContent).toContain("Bo");
  expect(document.querySelector("#room-agent-approval")).toBeNull();
  expect([...document.querySelectorAll("button")].some(button => button.textContent === t("removeAgent"))).toBe(false);
  await act(async () => root.render(<ConversationMembers {...props} members={[{ ...agent, canEditMode: true, canRemove: true }]} />));
  const select = document.querySelector<HTMLSelectElement>("#room-agent-approval")!;
  await act(async () => { select.value = "assisted"; select.dispatchEvent(new Event("change", { bubbles: true })); });
  expect(onApprovalMode).toHaveBeenCalledWith("102", "assisted");
  await act(async () => [...document.querySelectorAll("button")].find(button => button.textContent === t("removeAgent"))!.click());
  expect(onRemoveAgent).toHaveBeenCalledWith("102");
});
