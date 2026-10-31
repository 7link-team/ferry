import { describe, expect, it } from "vitest";
import { join, parent, normalizePath } from "./HostFiles";

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

  it("handles Windows drive roots, spaces and UNC shares without drive-relative paths", () => {
    expect(join("C:/", "Users")).toBe("C:/Users");
    expect(join("D:\\Projects", "a b")).toBe("D:/Projects/a b");
    expect(parent("C:/Users")).toBe("C:/");
    expect(parent("D:\\")).toBe("/");
    expect(parent("D:/Projects/")).toBe("D:/");
    expect(parent("//server/share/folder")).toBe("//server/share");
    expect(parent("//server/share")).toBe("/");
    expect(normalizePath("/var/a\\b")).toBe("/var/a\\b");
  });
});
