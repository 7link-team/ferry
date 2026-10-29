import { describe, expect, it } from "vitest";
import { href, parse } from "./router";

it("round trips the standalone host workspace route", () => {
  const path = "/ops/host/nodes/123/workspace";
  expect(parse(path)).toMatchObject({ workspace: "ops", group: "host", item: "hosts", id: "123", mode: "workspace" });
  expect(href(parse(path), {})).toBe(path);
  expect(parse("/ops/host/hosts/123/workspace")).toMatchObject(parse(path));
  expect(parse("/ops/infrastructure/hosts/123/workspace")).toMatchObject(parse(path));
});

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

describe("pipeline group routes", () => {
  it("keeps group pages under pipelines", () => {
    for (const path of [
      "/ops/build/pipelines/groups",
      "/ops/build/pipelines/groups/new",
      "/ops/build/pipelines/groups/123",
      "/ops/build/pipelines/groups/123/edit",
    ]) {
      const route = parse(path);
      expect(route).toMatchObject({ workspace: "ops", group: "build", item: "pipeline-groups" });
      expect(href(route, {})).toBe(path);
    }
  });
});
