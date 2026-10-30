import { describe, expect, it } from "vitest";
import { pageRange, queryString } from "./resource";

describe("pageRange", () => {
  it("lists every page while they fit", () => {
    expect(pageRange(1, 1)).toEqual([1]);
    expect(pageRange(2, 5)).toEqual([1, 2, 3, 4, 5]);
    expect(pageRange(1, 0)).toEqual([]);
  });

  it("always keeps the first and last page reachable", () => {
    for (const page of [1, 2, 50, 98, 99]) {
      const range = pageRange(page, 99);
      expect(range[0]).toBe(1);
      expect(range.at(-1)).toBe(99);
      expect(range).toContain(page);
    }
  });

  it("gaps in the middle, never at the ends", () => {
    expect(pageRange(50, 99)).toEqual([1, null, 49, 50, 51, null, 99]);
    expect(pageRange(1, 99)).toEqual([1, 2, 3, 4, null, 99]);
    expect(pageRange(99, 99)).toEqual([1, null, 96, 97, 98, 99]);
  });

  it("keeps a stable width so the number under the cursor does not move", () => {
    // Paging 1 -> 2 -> 3 must not reflow the control.
    const widths = [1, 2, 3].map((page) => pageRange(page, 99).length);
    expect(new Set(widths).size).toBe(1);
  });
});

describe("queryString", () => {
  it("drops empty values so a cleared filter leaves the URL", () => {
    expect(queryString({ q: "", page: 1 })).toBe("?page=1");
    expect(queryString({ q: undefined })).toBe("");
    expect(queryString({})).toBe("");
  });

  it("encodes values rather than pasting them in", () => {
    expect(queryString({ q: "a&b=c" })).toBe("?q=a%26b%3Dc");
  });
});

describe("action URLs", () => {
  it("uses HTTP methods and noun subresources for collection and item operations", async () => {
    const calls: { url: string; method: string | undefined }[] = [];
    const original = globalThis.fetch;
    globalThis.fetch = (async (url: string, options?: RequestInit) => {
      calls.push({ url: String(url), method: options?.method });
      return new Response("{}", { status: 200, headers: { "content-type": "application/json" } });
    }) as typeof fetch;
    const { runAction, runItemAction } = await import("./resource");
    const workspace = "109124796398174208";
    await runAction(workspace, "hosts", "bulk-delete", {});
    await runItemAction(workspace, "hosts", "h1", "test-connection", {});
    await runAction(workspace, "host-groups", "reorder", {});
    globalThis.fetch = original;
    expect(calls).toEqual([
      { url: `/api/workspaces/${workspace}/nodes`, method: "DELETE" },
      { url: `/api/workspaces/${workspace}/nodes/h1/connection-tests`, method: "POST" },
      { url: `/api/workspaces/${workspace}/node-groups/ordering`, method: "PATCH" },
    ]);
  });
});

describe("secret handling", () => {
  it("a PEM key survives a textarea but not an input", () => {
    // Why ResourceForm renders `secret` as a textarea: assigning a multi-line
    // value to an <input> drops the newlines, which corrupts a private key.
    const pem = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaA\nAAAA\n-----END OPENSSH PRIVATE KEY-----";
    const input = document.createElement("input");
    input.type = "password";
    input.value = pem;
    expect(input.value).not.toBe(pem);
    expect(input.value).not.toContain("\n");

    const textarea = document.createElement("textarea");
    textarea.value = pem;
    expect(textarea.value).toBe(pem);
  });
});
