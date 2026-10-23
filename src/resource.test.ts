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
