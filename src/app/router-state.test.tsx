import { act, useEffect } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it } from "vitest";
import { navigate, parse, href, useQueryValue, useRoute, useInternalNavigation } from "./router";

let cleanup = () => {};
afterEach(() => cleanup());
it("routes ordinary links including portals without replacing the mounted application", async () => {
  history.replaceState({}, "", "/zh-CN/workspaces/42/tags/7");
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host); let mounts = 0;
  function View() {
    useInternalNavigation(); const route = useRoute();
    useEffect(() => { mounts++; }, []);
    return <p>{route.mode}:{route.id}</p>;
  }
  const portal = document.createElement("a"); portal.href = "/zh-CN/workspaces/42/tags/7/edit?section=general";
  portal.innerHTML = "<span>Edit</span>"; document.body.append(portal);
  cleanup = () => { act(() => root.unmount()); host.remove(); portal.remove(); };
  await act(async () => root.render(<View />));
  const event = new MouseEvent("click", { bubbles: true, cancelable: true });
  await act(async () => portal.firstElementChild!.dispatchEvent(event));
  expect(event.defaultPrevented).toBe(true);
  expect(location.pathname).toBe("/zh-CN/workspaces/42/tags/7/edit");
  expect(location.search).toBe("?section=general");
  expect(host.textContent).toBe("edit:7"); expect(mounts).toBe(1);
});

it("preserves external, download, alternate-target, modified, handled and fragment link behavior", async () => {
  history.replaceState({}, "", "/en-US/workspaces/42/tags");
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  function View() { useInternalNavigation(); return null; }
  cleanup = () => { act(() => root.unmount()); host.remove(); };
  await act(async () => root.render(<View />));
  const variants = [
    {href:"https://elsewhere.invalid/workspaces/42/tags"},
    {href:"/api/workspaces/42/files/file"}, {href:"#section"},
    {href:"/en-US/workspaces/42/tags#section"},
    {href:"/en-US/account",download:"profile"}, {href:"/en-US/account",target:"_blank"},
    {href:"/en-US/account",rel:"external"},
    {href:"/en-US/account",ctrlKey:true}, {href:"/en-US/account",metaKey:true},
    {href:"/en-US/account",shiftKey:true}, {href:"/en-US/account",altKey:true},
    {href:"/en-US/account",button:1}, {href:"app-settings://connection"},
  ];
  for (const variant of variants) {
    const anchor = document.createElement("a"); anchor.href = variant.href;
    for (const key of ["download","target","rel"] as const) if (key in variant) anchor.setAttribute(key, String(variant[key]));
    document.body.append(anchor);
    let intercepted = true;
    const suppressNative = (event: MouseEvent) => { intercepted = event.defaultPrevented; event.preventDefault(); };
    document.addEventListener("click", suppressNative);
    await act(async () => anchor.dispatchEvent(new MouseEvent("click", {bubbles:true,cancelable:true,...variant})));
    expect(intercepted, JSON.stringify(variant)).toBe(false);
    document.removeEventListener("click", suppressNative); anchor.remove();
  }
  expect(location.pathname).toBe("/en-US/workspaces/42/tags");
});
it("updates the locale for translated internal links without a document navigation", async () => {
  history.replaceState({}, "", "/en-US/workspaces/42/tags");
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  function View() { useInternalNavigation(); const route=useRoute();return <a href="/ms-MY/workspaces/42/tags">{route.locale}</a>; }
  cleanup=()=>{act(()=>root.unmount());host.remove();};
  await act(async()=>root.render(<View/>));
  await act(async()=>host.querySelector('a')!.click());
  expect(location.pathname).toBe("/ms-MY/workspaces/42/tags");
  expect(document.documentElement.lang).toBe("ms-MY");expect(host.textContent).toBe("ms-MY");
});
it.each(["", "/123"])("replaces legacy health URL with the application destination: %s", async suffix => {
  history.replaceState({}, "", `/zh-CN/workspaces/42/application-health${suffix}?page=2`);
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  function View() { const route = useRoute(); return <p>{route.item}:{route.mode}</p>; }
  cleanup = () => { act(() => root.unmount()); host.remove(); };
  await act(async () => root.render(<View />));
  expect(location.pathname).toBe(`/zh-CN/workspaces/42/deployments${suffix}`);
  expect(new URLSearchParams(location.search).get("page")).toBe("2");
  expect(new URLSearchParams(location.search).get("tab")).toBe(suffix ? "health" : null);
  expect(host.textContent).toBe(suffix ? "apps:view" : "apps:list");
});
it("gives create/join and public entry pages independent, round-trip URLs", () => {
  for (const page of ["workspaces", "login", "register", "initialize"]) {
    const path = `/en-US/${page}`;
    expect(href(parse(path), {})).toBe(path);
  }
});
it("restores query-backed tabs and pagination on back/forward without remounting", async () => {
  history.replaceState({}, "", "/en-US/platform/monitor/redis?tab=keys&page=3");
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host); let mounts = 0;
  let change: (value: string) => void = () => {};
  function View() {
    const [tab, setTab] = useQueryValue("tab", "overview");
    const [page] = useQueryValue("page", 1); change = setTab;
    useEffect(() => { mounts++; }, []);
    return <p>{tab}:{page}</p>;
  }
  cleanup = () => { act(() => root.unmount()); host.remove(); };
  await act(async () => root.render(<View />));
  expect(host.textContent).toBe("keys:3");
  await act(async () => change("info"));
  expect(new URLSearchParams(location.search).get("tab")).toBe("info");
  await act(async () => {
    history.replaceState({}, "", "/en-US/platform/monitor/redis?tab=keys&page=8");
    dispatchEvent(new PopStateEvent("popstate"));
  });
  expect(host.textContent).toBe("keys:8");
  expect(mounts).toBe(1);
  await act(async () => navigate({workspace:"_workspaces"}, {}));
  expect(location.pathname).toBe("/en-US/workspaces");
  expect(location.search).toBe("");
});
