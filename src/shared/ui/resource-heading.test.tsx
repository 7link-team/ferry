import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { RemoteResourceHeading } from "./resource-heading";

let host: HTMLDivElement, root: Root;
const pending: { resolve: (value: Response) => void; signal: AbortSignal | null | undefined }[] = [];
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  pending.length = 0;
  vi.stubGlobal("fetch", vi.fn((_url: string, init: RequestInit) => new Promise<Response>(resolve => pending.push({ resolve, signal: init.signal }))));
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); });
it("clears the prior identity on workspace changes and ignores a late aborted response", async () => {
  await act(async () => root.render(<RemoteResourceHeading endpoint="/workspaces/one/pipelines/a" fallback="a" />));
  await act(async () => pending[0]!.resolve(new Response(JSON.stringify({ name: "Workspace one" }), { headers: { "Content-Type": "application/json" } })));
  expect(host.querySelector("h2")?.textContent).toBe("Workspace one");
  await act(async () => root.render(<RemoteResourceHeading endpoint="/workspaces/two/pipelines/b" fallback="b" />));
  expect(host.textContent).not.toContain("Workspace one");
  await act(async () => root.render(<RemoteResourceHeading endpoint="/workspaces/three/pipelines/c" fallback="c" />));
  expect(pending[1]!.signal?.aborted).toBe(true);
  await act(async () => pending[2]!.resolve(new Response(JSON.stringify({ name: "Workspace three" }), { headers: { "Content-Type": "application/json" } })));
  await act(async () => pending[1]!.resolve(new Response(JSON.stringify({ name: "Late workspace two" }), { headers: { "Content-Type": "application/json" } })));
  expect(host.querySelector("h2")?.textContent).toBe("Workspace three");
});
it("shows a failed read as an error without retaining the previous name", async () => {
  await act(async () => root.render(<RemoteResourceHeading endpoint="/workspaces/one/pipelines/a" fallback="a" />));
  await act(async () => pending[0]!.resolve(new Response(JSON.stringify({ error: "Forbidden" }), { status: 403 })));
  expect(host.querySelector("h2")?.textContent).toBe("a");
  expect(host.querySelector('[role="alert"]')).not.toBeNull();
});
