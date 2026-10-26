import { describe, expect, it } from "vitest";
import { href, parse } from "./router";

describe("platform route boundary", () => {
  it("keeps the previously valid platform workspace in the tenant router", () => {
    expect(parse("/platform/assets/hosts").workspace).toBe("platform");
    expect(parse("/platform/assets/hosts").group).not.toBe("platform");
  });
  it("round trips global list, create, detail and edit routes", () => {
    for (const path of ["/_platform/users", "/_platform/workspaces/new", "/_platform/users/123", "/_platform/users/123/edit", "/_platform/settings/1/edit"]) {
      const route = parse(path);
      expect(route.workspace).toBe("_platform");
      expect(route.group).toBe("platform");
      expect(href(route, {})).toBe(path);
    }
  });
});
