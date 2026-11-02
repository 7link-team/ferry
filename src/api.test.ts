import { afterEach, expect, it, vi } from "vitest";
import { api, localizedFetch } from "./api";
import { setLocale } from "./locale";

afterEach(() => { localStorage.clear(); vi.unstubAllGlobals(); });

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

it("sends the selected URL locale while preserving caller headers and raw upload bodies", async () => {
  history.replaceState({}, "", "/ko-KR/workspaces/1/nodes");
  setLocale("ko-KR", false);
  const request = vi.fn(async (_input: RequestInfo | URL, _init?: RequestInit) => new Response("{}", { headers: { "Content-Type": "application/json" } }));
  vi.stubGlobal("fetch", request);
  await api("/me", { headers: new Headers({ "X-Test": "preserved", "Accept-Language": "en-US", "Content-Type": "application/problem+json" }) });
  const init = request.mock.calls[0]?.[1] as RequestInit | undefined;
  expect(new Headers(init?.headers).get("Accept-Language")).toBe("ko-KR");
  expect(new Headers(init?.headers).get("X-Test")).toBe("preserved");
  expect(new Headers(init?.headers).get("Content-Type")).toBe("application/problem+json");
  const blob = new Blob(["stream me"]);
  await localizedFetch("/api/files", { method: "PUT", body: blob, headers: { "X-Ferry-Request": "1" } });
  const upload = request.mock.calls[1]?.[1] as RequestInit | undefined;
  expect(upload?.body).toBe(blob);
  expect(new Headers(upload?.headers).get("Accept-Language")).toBe("ko-KR");
  expect(new Headers(upload?.headers).has("Content-Type")).toBe(false);
});

it("localizes network failures but preserves abort identity and server diagnostics", async () => {
  setLocale("en-US", false);
  vi.stubGlobal("fetch", vi.fn(async () => { throw new TypeError("offline"); }));
  await expect(api("/me")).rejects.toMatchObject({ status: 0, message: "Cannot connect to the service. Check your network and try again." });
  const aborted = new DOMException("Cancelled", "AbortError");
  vi.stubGlobal("fetch", vi.fn(async () => { throw aborted; }));
  await expect(api("/me")).rejects.toBe(aborted);
  setLocale("zh-CN", false);
  vi.stubGlobal("fetch", vi.fn(async () => { throw new TypeError("offline"); }));
  await expect(api("/me")).rejects.toMatchObject({ status: 0, message: "无法连接服务，请检查网络后重试。" });
});
