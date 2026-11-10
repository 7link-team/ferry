import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { CollapsibleMessage } from "./CollapsibleMessage";
import { conversationBackground } from "./ConversationAppearance";
vi.mock("./resource-config", () => ({ conversationText: (key: string) => key }));
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
afterEach(() => { vi.unstubAllGlobals(); vi.restoreAllMocks(); });
it("collapses tall content, keeps its full source, and retains expansion across polling", async () => {
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  vi.spyOn(HTMLElement.prototype, "scrollHeight", "get").mockReturnValue(900);
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host), source = "full source ".repeat(200);
  try {
    await act(() => root.render(<CollapsibleMessage><p>{source}</p></CollapsibleMessage>));
    expect(host.querySelector("[inert]")).not.toBeNull();
    expect(host.querySelector("p")!.textContent).toBe(source);
    await act(() => host.querySelector("button")!.click());
    expect(host.querySelector("[inert]")).toBeNull();
    await act(() => root.render(<CollapsibleMessage><p>{source}</p></CollapsibleMessage>));
    expect(host.querySelector("button")!.getAttribute("aria-expanded")).toBe("true");
  } finally { await act(() => root.unmount()); host.remove(); }
});
it("keeps short messages and live responses open", async () => {
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  vi.spyOn(HTMLElement.prototype, "scrollHeight", "get").mockReturnValue(800);
  const host=document.createElement("div"), root=createRoot(host);
  try {
    await act(() => root.render(<CollapsibleMessage streaming><p>Live</p></CollapsibleMessage>));
    expect(host.querySelector("[inert],button")).toBeNull();
  } finally { await act(() => root.unmount()); }
});
it("uses only authenticated room images or local patterns, rejecting injected CSS values", () => {
  expect(conversationBackground({ pattern:"dots",color:"red;url(evil)",image:"https://evil" },"/room")).toMatchObject({ backgroundColor:"var(--background)" });
  expect(conversationBackground({pattern:"plain",color:"#112233",image:"123"},"/room").backgroundImage).toContain('/api/room/files/123');
});
