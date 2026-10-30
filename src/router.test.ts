import { describe, expect, it } from "vitest";
import { href, parse } from "./router";

const workspace = "109124796398174208";
it("round trips node workspace URLs without changing snowflake precision", () => {
  const path = `/workspaces/${workspace}/nodes/109174082875949056/workspace`;
  expect(parse(path)).toMatchObject({ workspace, group: "host", item: "hosts", id: "109174082875949056", mode: "workspace" });
  expect(href(parse(path), {})).toBe(path);
});
it("resolves legacy host bookmarks into canonical resource paths", () => {
  for (const path of [`/${workspace}/host/hosts/123/workspace`, `/${workspace}/host/nodes/123/workspace`, `/${workspace}/infrastructure/hosts/123/workspace`]) {
    expect(href(parse(path), {})).toBe(`/workspaces/${workspace}/nodes/123/workspace`);
  }
});
describe("platform route boundary", () => {
  it("reserves platform paths outside ID-scoped workspaces", () => {
    expect(parse(`/workspaces/${workspace}/nodes`).workspace).toBe(workspace);
    expect(parse('/platform/users').workspace).toBe('_platform');
  });
  it("round trips list, create, detail and edit pages", () => {
    for (const path of ["/platform/users", "/platform/workspaces/new", "/platform/users/123", "/platform/users/123/edit", "/platform/settings/1/edit"]) {
      expect(href(parse(path), {})).toBe(path);
      expect(href(parse(path.replace('/platform/', '/_platform/')), {})).toBe(path);
    }
  });
});
it("uses resource names for pipeline groups and preserves legacy group IDs", () => {
  for (const suffix of ['', '/new', '/123', '/123/edit']) {
    const path = `/workspaces/${workspace}/pipeline-groups${suffix}`;
    expect(parse(path)).toMatchObject({ workspace, group: 'build', item: 'pipeline-groups' });
    expect(href(parse(path), {})).toBe(path);
    expect(href(parse(`/${workspace}/build/pipelines/groups${suffix}`), {})).toBe(path);
  }
});
it("keeps shared resource menu contexts in query parameters", () => {
  for (const group of ['host', 'build', 'deploy']) {
    const path = href({ workspace, group, item: 'cron-jobs', mode: 'list' }, {});
    const url = new URL(path, 'http://localhost');
    expect(url.pathname).toBe(`/workspaces/${workspace}/cron-jobs`);
    expect(url.searchParams.get('scope')).toBe(group);
    expect(parse(url.pathname, url.search)).toMatchObject({ group, item: 'cron-jobs' });
  }
});
