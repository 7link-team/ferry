import { beforeEach,describe,expect,it } from "vitest";
import { setLocale } from "../shared/i18n/locale";
import { href,navigateToLogin,parse,safeLoginRedirect } from "./router";
import { nav } from "./nav";

beforeEach(() => { localStorage.clear(); history.replaceState({}, "", "/en-US/"); setLocale("en-US", false); });

const workspace = "109124796398174208";
it("uses an explicit login URL and preserves deep-link query and hash only for reauthentication", () => {
  const target = `/en-US/workspaces/${workspace}/code-connections/42?tab=events&page=2#latest`;
  history.replaceState({}, "", target);
  navigateToLogin();
  expect(location.pathname).toBe("/en-US/login");
  expect(new URLSearchParams(location.search).get("redirect")).toBe(target);
  navigateToLogin();
  expect(new URLSearchParams(location.search).get("redirect")).toBe(target);
  navigateToLogin(false);
  expect(location.pathname).toBe("/en-US/login");
  expect(location.search).toBe("");
});
it("rejects offsite and recursive login destinations", () => {
  for (const target of [null,"https://example.com/","//example.com/","/\\example.com/","/en-US/login?redirect=/account","/login/","/\n/evil","javascript:alert(1)"]) {
    expect(safeLoginRedirect(target)).toBeUndefined();
  }
  expect(safeLoginRedirect("/en-US/account?tab=profile#name")).toBe("/en-US/account?tab=profile#name");
});
it("unifies both editors without changing resource URLs or legacy AI credential bookmarks", () => {
  for (const resource of ["pipelines", "workflows"]) {
    for (const suffix of ["", "/new", "/123", "/123/edit"]) {
      const path = `/workspaces/${workspace}/${resource}${suffix}`;
      expect(parse(path, "")).toMatchObject({ group: "workflow", item: resource });
      expect(href(parse(path, ""), {})).toBe(`/en-US${path}`);
    }
  }
  const legacy = parse(`/${workspace}/workflow/credentials/new`, "");
  expect(legacy).toMatchObject({ group: "ops", item: "ai-models", mode: "new" });
  expect(parse(`/workspaces/${workspace}/credentials/new`, "?scope=ai")).toMatchObject({ group: "ops", item: "ai-models", mode: "new" });
  expect(parse(`/workspaces/${workspace}/ai-models/new`, "").group).toBe(legacy.group);
  expect(href(legacy, {})).toBe(`/en-US/workspaces/${workspace}/ai-models/new`);
});
it("preserves each module through credential list, create, detail and edit links", () => {
  for (const scope of ["host", "build", "deploy"]) {
    for (const suffix of ["", "/new", "/123", "/123/edit"]) {
      const path = `/workspaces/${workspace}/credentials${suffix}`;
      const route = parse(path, `?scope=${scope}`);
      expect(route.group).toBe(scope === "build" ? "repository" : scope);
      expect(href(route, {})).toBe(`/en-US${path}?scope=${scope}`);
    }
  }
  expect(parse(`/workspaces/${workspace}/credentials`, "").group).toBe("workflow");
});
it("places repository management between build and hosts and preserves old bookmarks", () => {
  const keys = nav.map(group => group.key);
  expect(keys.indexOf("repository")).toBe(keys.indexOf("workflow") + 1);
  expect(keys).not.toContain("build");
  expect(keys.indexOf("host")).toBe(keys.indexOf("repository") + 1);
  const repositoryItems = nav.find(group => group.key === "repository")!.items;
  const events = repositoryItems.findIndex(item => item.key === "repository-events");
  expect(events).toBe(repositoryItems.findIndex(item => item.key === "code-insights") - 1);
  expect(repositoryItems[events]!.hidden).not.toBe(true);
  expect(repositoryItems[events]!.permission).toBe("repositories.read");
  for (const [item, resource] of [["repositories", "code-connections"], ["code-insights", "code-insights"], ["repository-events", "repository-events"]]) {
    for (const suffix of ["", "/new", "/109124836743184391", "/109124836743184391/edit"]) {
      const legacy = parse(`/zh-CN/${workspace}/build/${item}${suffix}`, "");
      expect(legacy).toMatchObject({ group: "repository", item, workspace });
      expect(href(legacy, { page: "2" })).toBe(`/zh-CN/workspaces/${workspace}/${resource}${suffix}?page=2`);
    }
  }
});
it("round trips node workspace URLs without changing snowflake precision", () => {
  const path = `/workspaces/${workspace}/nodes/109174082875949056/workspace`;
  expect(parse(path)).toMatchObject({ workspace, group: "host", item: "hosts", id: "109174082875949056", mode: "workspace" });
  expect(href(parse(path), {})).toBe(`/en-US${path}`);
});
it("resolves legacy host bookmarks into canonical resource paths", () => {
  for (const path of [`/${workspace}/host/hosts/123/workspace`, `/${workspace}/host/nodes/123/workspace`, `/${workspace}/infrastructure/hosts/123/workspace`]) {
    expect(href(parse(path), {})).toBe(`/en-US/workspaces/${workspace}/nodes/123/workspace`);
  }
});
describe("platform route boundary", () => {
  it("defaults platform entry to workspaces and preserves the server nodes route", () => {
    for (const path of ["/platform", "/_platform", "/zh-CN/platform"]) {
      expect(parse(path, "")).toMatchObject({ workspace: "_platform", item: "workspaces", mode: "list" });
    }
    expect(href(parse("/platform/server-nodes", ""), {})).toBe("/en-US/platform/server-nodes");
  });
  it("reserves platform paths outside ID-scoped workspaces", () => {
    expect(parse(`/workspaces/${workspace}/nodes`).workspace).toBe(workspace);
    expect(parse('/platform/users').workspace).toBe('_platform');
  });
  it("routes platform template sources", () => {
    expect(parse("/platform/marketplace-sources", "")).toMatchObject({ workspace: "_platform", item: "marketplace-sources", mode: "list" });
    expect(parse("/platform/marketplace-sources/123/edit", "")).toMatchObject({ item: "marketplace-sources", id: "123", mode: "edit" });
    expect(href(parse("/platform/marketplace-sources/new", ""), {})).toBe("/en-US/platform/marketplace-sources/new");
  });
  it("round trips list, create, detail and edit pages", () => {
    for (const path of ["/platform/users", "/platform/workspaces/new", "/platform/users/123", "/platform/users/123/edit", "/platform/settings/1/edit"]) {
      expect(href(parse(path), {})).toBe(`/en-US${path}`);
      expect(href(parse(path.replace('/platform/', '/_platform/')), {})).toBe(`/en-US${path}`);
    }
  });
});
it("uses resource names for pipeline groups and preserves legacy group IDs", () => {
  for (const suffix of ['', '/new', '/123', '/123/edit']) {
    const path = `/workspaces/${workspace}/pipeline-groups${suffix}`;
    expect(parse(path)).toMatchObject({ workspace, group: 'workflow', item: 'pipeline-groups' });
    expect(href(parse(path), {})).toBe(`/en-US${path}`);
    expect(href(parse(`/${workspace}/build/pipelines/groups${suffix}`), {})).toBe(`/en-US${path}`);
  }
});
it("keeps shared resource menu contexts in query parameters", () => {
  for (const group of ['host', 'build', 'deploy']) {
    const path = href({ workspace, group, item: 'cron-jobs', mode: 'list' }, {});
    const url = new URL(path, 'http://localhost');
    expect(url.pathname).toBe(`/en-US/workspaces/${workspace}/cron-jobs`);
    expect(url.searchParams.get('scope')).toBe(group);
    expect(parse(url.pathname, url.search)).toMatchObject({ group: group === 'build' ? 'workflow' : group, item: 'cron-jobs' });
  }
});

it("round trips explicit locale deep links and account routes", () => {
  const route = parse(`/ja-JP/workspaces/${workspace}/nodes/123/edit`, "?page=2");
  expect(route).toMatchObject({ locale: "ja-JP", workspace, item: "hosts", id: "123", mode: "edit" });
  expect(href(route, { page: "2" })).toBe(`/ja-JP/workspaces/${workspace}/nodes/123/edit?page=2`);
  expect(href(parse("/ko-KR/account"), {})).toBe("/ko-KR/account");
  expect(href(parse("/zh-TW/platform/settings/1/edit"), {})).toBe("/zh-TW/platform/settings/1/edit");
});
