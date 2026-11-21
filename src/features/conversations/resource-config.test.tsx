import { afterEach, expect, it, vi } from "vitest";
import { conversationAgentOptions } from "./resource-config";

afterEach(() => vi.unstubAllGlobals());

it("offers both API and CLI Agents while retaining the server's use filter", async () => {
  const urls: string[] = [];
  vi.stubGlobal("fetch", vi.fn(async (url: string) => {
    urls.push(url);
    return { ok: true, status: 200, json: async () => ({ items: [
      { id: "1", name: "CLI agent", runtime: "codex" },
      { id: "2", name: "API agent", runtime: "api" },
      { id: "3", name: "Claude agent", runtime: "claude" },
    ], total: 3, page: 1, page_size: 200 }) };
  }));
  const options = await conversationAgentOptions("ops", { for_use: "true" })();
  expect(options.map(o => o.label)).toEqual(["CLI agent", "API agent", "Claude agent"]);
  expect(urls[0]).toContain("for_use=true");
});
