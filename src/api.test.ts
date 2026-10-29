import { afterEach, expect, it, vi } from "vitest";
import { api } from "./api";

afterEach(() => vi.unstubAllGlobals());

it("shows the server's plain-text request rejection instead of a generic error", async () => {
  vi.stubGlobal("fetch", vi.fn(async () => new Response("不允许此来源发起操作。", {
    status: 403,
    headers: { "Content-Type": "text/plain; charset=utf-8" },
  })));
  await expect(api("/users", { method: "POST", body: "{}" })).rejects.toMatchObject({
    status: 403,
    message: "不允许此来源发起操作。",
  });
});
