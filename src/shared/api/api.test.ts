import { afterEach,expect,it,vi } from "vitest";
import { setLocale } from "../i18n/locale";
import { api,localizedFetch } from "./api";

afterEach(() => { localStorage.clear(); vi.unstubAllGlobals(); });

it("shows Ferry's own localized plain-text rejection instead of a generic error", async () => {
  vi.stubGlobal("fetch", vi.fn(async () => new Response("不允许此来源发起操作。", {
    status: 403,
    headers: { "Content-Type": "text/plain; charset=utf-8", "Content-Language": "zh-CN" },
  })));
  await expect(api("/users", { method: "POST", body: "{}" })).rejects.toMatchObject({
    status: 403,
    message: "不允许此来源发起操作。",
  });
});

it("keeps proxy and framework bodies out of the UI and names the HTTP status", async () => {
  setLocale("zh-CN", false);
  const debug = vi.spyOn(console, "debug").mockImplementation(() => {});
  vi.stubGlobal("fetch", vi.fn(async () => new Response("Bad Gateway", {
    status: 502,
    headers: { "Content-Type": "text/plain" },
  })));
  await expect(api("/users")).rejects.toMatchObject({ status: 502, message: "请求失败（HTTP 502），请稍后重试。" });
  vi.stubGlobal("fetch", vi.fn(async () => new Response("<html><body>502</body></html>", {
    status: 502,
    headers: { "Content-Type": "text/plain", "Content-Language": "en-US" },
  })));
  await expect(api("/users")).rejects.toMatchObject({ message: "请求失败（HTTP 502），请稍后重试。" });
  vi.stubGlobal("fetch", vi.fn(async () => new Response("<!doctype html>oops", {
    status: 500,
    headers: { "Content-Type": "text/html" },
  })));
  await expect(api("/users")).rejects.toMatchObject({ status: 500, message: "请求失败（HTTP 500），请稍后重试。" });
  vi.stubGlobal("fetch", vi.fn(async () => new Response(JSON.stringify({ error: "服务器给出的原因" }), {
    status: 409,
    headers: { "Content-Type": "application/json" },
  })));
  await expect(api("/users")).rejects.toMatchObject({ message: "服务器给出的原因" });
  expect(debug).toHaveBeenCalled();
  debug.mockRestore();
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
