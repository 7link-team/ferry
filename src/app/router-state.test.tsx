import { act, useEffect } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it } from "vitest";
import { navigate, parse, href, useQueryValue } from "./router";

let cleanup = () => {};
afterEach(() => cleanup());
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
