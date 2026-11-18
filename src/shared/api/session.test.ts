import { afterEach, beforeEach, expect, it, vi } from "vitest";

const response = (data: unknown, status = 200) =>
  new Response(JSON.stringify(data), {
    status,
    headers: { "Content-Type": "application/json" },
  });
const metadata = (auto = true) => {
  const now = Date.now();
  return {
    access_expires_at_ms: now - 1,
    refresh_expires_at_ms: now + 14 * 86400000,
    server_time_ms: now,
    refreshable: true,
    auto_renew: auto,
  };
};
const unauthorized = vi.fn(() => {});
beforeEach(() => {
  vi.resetModules();
  unauthorized.mockClear();
  window.addEventListener("ferry:unauthorized", unauthorized);
});
afterEach(() => {
  window.removeEventListener("ferry:unauthorized", unauthorized);
  vi.unstubAllGlobals();
});

it("keeps the page signed in while idle access expires without renewing from background polling", async () => {
  const request = vi
    .fn()
    .mockResolvedValueOnce(response({ session: metadata() }))
    .mockResolvedValueOnce(response({ error: "Sign in" }, 401));
  vi.stubGlobal("fetch", request);
  const { api } = await import("./api");
  await api("/me");
  await expect(api("/workspaces/1/workflows")).rejects.toMatchObject({
    status: 401,
  });
  expect(unauthorized).not.toHaveBeenCalled();
  expect(request).toHaveBeenCalledTimes(2);
  expect(
    request.mock.calls.some(([url]) =>
      String(url).includes("/sessions/refresh"),
    ),
  ).toBe(false);
});

it("uses the normal sign-in flow when automatic renewal is disabled", async () => {
  const request = vi
    .fn()
    .mockResolvedValueOnce(response({ session: metadata(false) }))
    .mockResolvedValueOnce(response({ error: "Sign in" }, 401));
  vi.stubGlobal("fetch", request);
  const { api } = await import("./api");
  await api("/me");
  await expect(api("/workspaces/1/workflows")).rejects.toMatchObject({
    status: 401,
  });
  expect(unauthorized).toHaveBeenCalledOnce();
  expect(request).toHaveBeenCalledTimes(2);
});

it("never replays a failed write to try to renew a session", async () => {
  const request = vi
    .fn()
    .mockResolvedValueOnce(response({ session: metadata() }))
    .mockResolvedValueOnce(response({ error: "Sign in" }, 401));
  vi.stubGlobal("fetch", request);
  const { api } = await import("./api");
  await api("/me");
  await expect(
    api("/workspaces/1/workflows", { method: "POST", body: "{}" }),
  ).rejects.toMatchObject({ status: 401 });
  expect(request).toHaveBeenCalledTimes(2);
  expect(
    request.mock.calls.filter(([, options]) => options?.method === "POST"),
  ).toHaveLength(1);
  expect(unauthorized).not.toHaveBeenCalled();
});

it("checks fresh cookies on the next user action after another tab rotated access early", async () => {
  const fresh = metadata();
  fresh.access_expires_at_ms = fresh.server_time_ms + 1800000;
  const request = vi
    .fn()
    .mockResolvedValueOnce(response({ session: fresh }))
    .mockResolvedValueOnce(response({ error: "Sign in" }, 401))
    .mockResolvedValueOnce(response({ session: fresh }));
  vi.stubGlobal("fetch", request);
  const { api } = await import("./api");
  const { watchSessionActivity } = await import("./session");
  const stop = watchSessionActivity();
  try {
    await api("/me");
    await expect(api("/workspaces/1/workflows")).rejects.toMatchObject({
      status: 401,
    });
    window.dispatchEvent(new Event("pointerdown"));
    await vi.waitFor(() => expect(request).toHaveBeenCalledTimes(3));
    expect(request.mock.calls[2]?.[0]).toBe("/api/me");
    expect(unauthorized).not.toHaveBeenCalled();
  } finally {
    stop();
  }
});

it("does not count programmatic field focus as human activity", async () => {
  const request = vi.fn().mockResolvedValue(response({ session: metadata() }));
  vi.stubGlobal("fetch", request);
  const { api } = await import("./api");
  const { watchSessionActivity } = await import("./session");
  const stop = watchSessionActivity();
  const input = document.createElement("input");
  document.body.append(input);
  try {
    await api("/me");
    input.focus();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(request).toHaveBeenCalledTimes(1);
  } finally {
    input.remove();
    stop();
  }
});

it("discards renewal metadata when a later identity has no renewable session", async () => {
  const { observeSession, canResumeSession } = await import("./session");
  observeSession(metadata());
  expect(canResumeSession()).toBe(true);
  observeSession(null);
  expect(canResumeSession()).toBe(false);
});

it("does not continually rotate a freshly issued one-minute access grant", async () => {
  const short = { ...metadata(), access_ttl_ms: 60_000 };
  short.access_expires_at_ms = short.server_time_ms + 60_000;
  const request = vi.fn().mockResolvedValue(response({ session: short }));
  vi.stubGlobal("fetch", request);
  const { observeSession, watchSessionActivity } = await import("./session");
  observeSession(short);
  const stop = watchSessionActivity();
  try {
    window.dispatchEvent(new Event("pointerdown"));
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(request).not.toHaveBeenCalled();
  } finally {
    stop();
  }
});

it("does not rotate when access already reaches the absolute refresh deadline", async () => {
  const final = { ...metadata(), access_ttl_ms: 1_800_000 };
  final.access_expires_at_ms = final.server_time_ms + 30_000;
  final.refresh_expires_at_ms = final.access_expires_at_ms;
  const request = vi.fn().mockResolvedValue(response({ session: final }));
  vi.stubGlobal("fetch", request);
  const { observeSession, watchSessionActivity } = await import("./session");
  observeSession(final);
  const stop = watchSessionActivity();
  try {
    window.dispatchEvent(new Event("pointerdown"));
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(request).not.toHaveBeenCalled();
  } finally { stop(); }
});

it("shares one active refresh and holds business requests until rotation finishes", async () => {
  let release!: (response: Response) => void;
  const rotated = new Promise<Response>((resolve) => {
    release = resolve;
  });
  const fresh = metadata();
  fresh.access_expires_at_ms += 1_800_000;
  const request = vi
    .fn()
    .mockResolvedValueOnce(response({}, 401))
    .mockReturnValueOnce(rotated)
    .mockResolvedValueOnce(response({ items: [] }));
  vi.stubGlobal("fetch", request);
  const { observeSession, watchSessionActivity } = await import("./session");
  const { api } = await import("./api");
  observeSession(metadata());
  const stop = watchSessionActivity();
  try {
    window.dispatchEvent(new Event("pointerdown"));
    window.dispatchEvent(new Event("keydown"));
    await vi.waitFor(() => expect(request).toHaveBeenCalledTimes(2));
    const read = api("/workspaces/1/workflows");
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(request).toHaveBeenCalledTimes(2);
    release(response(fresh));
    await read;
    expect(request).toHaveBeenCalledTimes(3);
    expect(
      request.mock.calls.filter(([url]) => url === "/api/sessions/refresh"),
    ).toHaveLength(1);
  } finally {
    stop();
  }
});

it("retains the page through a network failure and retries only on another human action", async () => {
  const request = vi
    .fn()
    .mockRejectedValueOnce(new TypeError("offline"))
    .mockResolvedValueOnce(response({}, 401))
    .mockResolvedValueOnce(response(metadata()));
  vi.stubGlobal("fetch", request);
  const { observeSession, watchSessionActivity } = await import("./session");
  observeSession(metadata());
  const stop = watchSessionActivity();
  try {
    window.dispatchEvent(new Event("pointerdown"));
    await vi.waitFor(() => expect(request).toHaveBeenCalledTimes(1));
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(unauthorized).not.toHaveBeenCalled();
    window.dispatchEvent(new Event("keydown"));
    await vi.waitFor(() => expect(request).toHaveBeenCalledTimes(3));
    expect(unauthorized).not.toHaveBeenCalled();
  } finally {
    stop();
  }
});
