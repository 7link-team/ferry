import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { FileContentPreview } from "./WorkflowAssets";

const hosts: { root: ReturnType<typeof createRoot>; element: HTMLDivElement }[] = [];
async function mount(mime = "text/markdown") {
  const element = document.createElement("div"); document.body.append(element);
  const root = createRoot(element); hosts.push({root, element});
  await act(async () => { root.render(<FileContentPreview file={{name:"sample", mime, size:1}} url="/api/test-file" />); });
  return element;
}
afterEach(async () => { for (const {root, element} of hosts.splice(0)) { await act(async () => root.unmount()); element.remove(); } vi.unstubAllGlobals(); });
it("renders actual text and escapes HTML without executing it", async () => {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response("# Actual file\n<script>alert(1)</script>")));
  const host = await mount();
  expect(host.querySelector("pre")?.textContent).toContain("# Actual file");
  expect(host.querySelector("pre")?.textContent).toContain("<script>");
  expect(host.querySelector("script")).toBeNull();
  expect(host.querySelector("a")?.hasAttribute("download")).toBe(true);
});
it("bounds the preview even when a server ignores Range", async () => {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response("a".repeat(400_000))));
  const host = await mount("application/json");
  expect(host.querySelector("pre")?.textContent?.length).toBe(256 * 1024);
  expect(host.querySelector('[role="status"]')?.textContent).toContain("256 KiB");
});
it("shows an accessible failure and allows retry", async () => {
  const fetcher=vi.fn().mockResolvedValueOnce(new Response("denied",{status:403})).mockResolvedValueOnce(new Response("Recovered"));
  vi.stubGlobal("fetch", fetcher);
  const host = await mount();
  expect(host.querySelector('[role="alert"]')).not.toBeNull();
  await act(async () => host.querySelector("button")!.click());
  expect(host.querySelector("pre")?.textContent).toBe("Recovered");
});
it("recognizes an empty ranged response and keeps binary files download-only", async () => {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(null,{status:416,headers:{"Content-Range":"bytes */0"}})));
  const empty = await mount(); expect(empty.querySelector("pre")?.textContent).toBeTruthy();
  const binary = await mount("application/octet-stream");
  expect(binary.querySelector("pre")).toBeNull(); expect(binary.querySelector("a")).not.toBeNull();
  expect(fetch).toHaveBeenCalledTimes(1);
});
