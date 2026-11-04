import { afterEach, expect, it, vi } from "vitest";
import { api, localizedFetch } from "./api";
import { activitySnapshot, beginActivity, beginRoute, commitRoute } from "./progress";

afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals(); });

it("keeps concurrent lifetimes independent, including repeated URLs and idempotent completion", () => {
  vi.useFakeTimers();
  const first = beginActivity("request", "/api/projects?token=secret");
  const second = beginActivity("request", "/api/projects?page=2");
  const running = activitySnapshot().filter(activity => !activity.done);
  expect(running).toHaveLength(2);
  expect(running[0]!.hueOffset).toBe(running[1]!.hueOffset);
  expect(running[0]!.hueOffset).toBeGreaterThanOrEqual(75);
  expect(running[0]!.hueOffset).toBeLessThanOrEqual(285);
  expect(JSON.stringify(running)).not.toContain("secret");
  first(); first();
  expect(activitySnapshot().filter(activity => !activity.done)).toHaveLength(1);
  vi.advanceTimersByTime(200);
  expect(activitySnapshot()).toHaveLength(1);
  second();
  vi.advanceTimersByTime(200);
  expect(activitySnapshot()).toHaveLength(0);
});

it("does not let a stale route commit finish a newer route or its network request", () => {
  vi.useFakeTimers();
  const frames: FrameRequestCallback[] = [];
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { frames.push(callback); return frames.length; });
  beginRoute("/en-US/workspaces/1/nodes");
  commitRoute("/en-US/workspaces/1/nodes");
  beginRoute("/en-US/workspaces/1/pipelines");
  const finishRequest = beginActivity("request", "/api/pipelines");
  while (frames.length) frames.shift()!(0);
  expect(activitySnapshot().filter(activity => !activity.done).map(activity => activity.kind)).toEqual(["route", "request"]);
  commitRoute("/en-US/workspaces/1/pipelines");
  while (frames.length) frames.shift()!(0);
  expect(activitySnapshot().filter(activity => !activity.done).map(activity => activity.kind)).toEqual(["request"]);
  finishRequest(); vi.advanceTimersByTime(200);
  expect(activitySnapshot()).toHaveLength(0);
  beginRoute("/en-US/workspaces/1/pipelines");
  expect(activitySnapshot()).toHaveLength(0);
});

it("tracks API body parsing, and cleans up success, HTTP failure, abort and transport failure", async () => {
  vi.useFakeTimers();
  let resolveBody!: (value: unknown) => void;
  vi.stubGlobal("fetch", vi.fn(async () => ({ ok: true, status: 200, headers: new Headers(), json: () => new Promise(resolve => { resolveBody = resolve; }) })));
  const pending = api("/projects");
  await Promise.resolve();
  expect(activitySnapshot().filter(activity => !activity.done)).toHaveLength(1);
  resolveBody({ ok: true });
  await pending;
  expect(activitySnapshot().every(activity => activity.done)).toBe(true);
  for (const error of [new DOMException("Cancelled", "AbortError"), new TypeError("Offline")]) {
    vi.stubGlobal("fetch", vi.fn(async () => { throw error; }));
    await expect(api("/projects")).rejects.toThrow();
    await expect(localizedFetch("/api/projects")).rejects.toBe(error);
  }
  vi.stubGlobal("fetch", vi.fn(async () => new Response('{"error":"Rejected"}', { status: 400 })));
  await expect(api("/projects")).rejects.toThrow("Rejected");
  expect(activitySnapshot().every(activity => activity.done)).toBe(true);
  vi.advanceTimersByTime(200);
  expect(activitySnapshot()).toHaveLength(0);
});

it("does not consume or hold open a raw streaming response", async () => {
  vi.useFakeTimers();
  const response = new Response(new ReadableStream({ start(controller) { controller.enqueue(new TextEncoder().encode("event: hello\n")); } }));
  vi.stubGlobal("fetch", vi.fn(async () => response));
  expect(await localizedFetch("/api/events")).toBe(response);
  expect(response.bodyUsed).toBe(false);
  expect(activitySnapshot().every(activity => activity.done)).toBe(true);
  await response.body!.cancel();
  vi.advanceTimersByTime(200);
});
