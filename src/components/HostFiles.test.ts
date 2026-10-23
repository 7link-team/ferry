import { describe, expect, it } from "vitest";
import { join, parent } from "./HostFiles";

describe("remote path arithmetic", () => {
  it("joins without doubling the root slash", () => {
    expect(join("/", "etc")).toBe("/etc");
    expect(join("/etc", "nginx")).toBe("/etc/nginx");
    expect(join("/a/b", "c")).toBe("/a/b/c");
  });

  it("walks up and stops at the root", () => {
    expect(parent("/a/b/c")).toBe("/a/b");
    expect(parent("/a")).toBe("/");
    expect(parent("/")).toBe("/");
  });

  it("round-trips: parent(join(dir, name)) === dir", () => {
    for (const dir of ["/", "/etc", "/var/log", "/a/b/c"]) {
      expect(parent(join(dir, "x"))).toBe(dir);
    }
  });
});
