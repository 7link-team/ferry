import { act, useCallback, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { parse } from "yaml";
import { ManifestEditor, readManifest } from "./ManifestEditor";

const fixture = `# original application
apiVersion: ferry/v1
kind: Application
metadata:
  name: web
  labels: {release: "01"}
services:
  - name: web
    image: nginx:alpine # keep image comment
    env:
      - name: TOKEN
        valueFrom: variable/token
    dependsOn: [db]
    resources: null
    health: null
  - name: db
    image: postgres:17
    command: ["postgres", "-c", "shared_buffers=256MB"]
overrides:
  compose:
    services:
      web:
        restart: unless-stopped
`;
let root: Root, host: HTMLDivElement, latest: string, blocked: boolean;
function Harness({ source = fixture, readOnly = false, mode = "compose" }) {
  const [value, setValue] = useState(source);
  latest = value;
  const validity = useCallback((value: boolean) => {
    blocked = value;
  }, []);
  return (
    <ManifestEditor
      value={value}
      mode={mode}
      readOnly={readOnly}
      onChange={setValue}
      onValidityChange={validity}
    />
  );
}
const button = (name: string, region: ParentNode = host) =>
  [...region.querySelectorAll("button")].find((b) => b.textContent === name || b.getAttribute("aria-label")?.startsWith(name))!;
const input = (name: string, region: ParentNode = host) =>
  [...region.querySelectorAll("label")]
    .find((l) => l.firstChild?.textContent === name)!
    .querySelector("input,textarea,select")! as HTMLInputElement;
const fill = async (element: Element, value: string) => {
  await act(async () => {
    const prototype =
      element instanceof HTMLTextAreaElement
        ? HTMLTextAreaElement.prototype
        : element instanceof HTMLSelectElement
          ? HTMLSelectElement.prototype
          : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(prototype, "value")!.set!.call(
      element,
      value,
    );
    element.dispatchEvent(
      new Event(element instanceof HTMLSelectElement ? "change" : "input", {
        bubbles: true,
      }),
    );
  });
};
const click = async (name: string, region: ParentNode = host) => {
  await act(async () => button(name, region).click());
};
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.stubGlobal("confirm", () => true);
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  blocked = false;
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  vi.unstubAllGlobals();
});
it("edits one field without losing comments, typed labels, secret references, commands or overrides", async () => {
  await act(async () => root.render(<Harness />));
  await fill(input("镜像"), "nginx:stable");
  const expected = parse(fixture);
  expected.services[0].image = "nginx:stable";
  expect(parse(latest)).toEqual(expected);
  expect(latest).toContain("# original application");
  expect(latest).toContain("# keep image comment");
  await click("源码编辑");
  const source = latest;
  await click("表单编辑");
  expect(latest).toBe(source);
});
it("renames dependency references and refuses deleting a service still in use", async () => {
  await act(async () => root.render(<Harness />));
  const db = host.querySelector('[aria-label="服务 2"]')!;
  await fill(input("服务名称", db), "web");
  expect(parse(latest).services[1].name).toBe("db");
  await fill(input("服务名称", db), "database");
  expect(parse(latest).services[0].dependsOn).toEqual(["database"]);
  expect(parse(latest).services[1]).toMatchObject({
    name: "database",
    command: ["postgres", "-c", "shared_buffers=256MB"],
  });
  await click("删除服务", db);
  expect(parse(latest).services).toHaveLength(2);
  expect(host.textContent).toContain("其他服务仍依赖此服务");
});
it("creates missing optional objects including a complete strategy and handles explicit nulls", async () => {
  await act(async () => root.render(<Harness />));
  await fill(input("CPU 上限"), "500m");
  await fill(input("等待健康检查（秒）"), "90");
  await act(async () =>
    (host.querySelector('[role="checkbox"]') as HTMLElement).click(),
  ); // dependency checkbox
  const readiness = [...host.querySelectorAll("label")].find(
    (l) => l.textContent === "就绪检查（HTTP）",
  )!;
  await act(async () => readiness.querySelector("button")!.click());
  expect(parse(latest).services[0].resources).toEqual({
    limits: { cpu: "500m" },
  });
  expect(parse(latest).strategy).toEqual({
    type: "recreate",
    healthTimeout: 90,
  });
  expect(parse(latest).services[0].health.readiness.httpGet.port).toBe(80);
});
it("blocks malformed source without replacing it, then renders repaired content", async () => {
  await act(async () => root.render(<Harness />));
  await click("源码编辑");
  await fill(input("应用配置源码（YAML / JSON）"), "services: [");
  expect(blocked).toBe(true);
  expect(latest).toBe("services: [");
  await click("表单编辑");
  expect(latest).toBe("services: [");
  await fill(input("应用配置源码（YAML / JSON）"), fixture);
  expect(blocked).toBe(false);
  expect(input("镜像").value).toBe("nginx:alpine");
});
it("invalid advanced JSON blocks saving and source switching until corrected", async () => {
  await act(async () => root.render(<Harness />));
  await fill(input("应用标签"), "{");
  expect(blocked).toBe(true);
  expect(button("源码编辑").disabled).toBe(true);
  expect(parse(latest).metadata.labels).toEqual({ release: "01" });
  await fill(input("应用标签"), '{"stage":"dev"}');
  expect(blocked).toBe(false);
  expect(parse(latest).metadata.labels).toEqual({ stage: "dev" });
});
it("adds native artifact services and read-only mode cannot mutate config", async () => {
  await act(async () => root.render(<Harness mode="native" />));
  await click("添加服务");
  expect(parse(latest).services[2]).toEqual({
    name: "service-3",
    artifact: { source: "pipeline", pipeline: "", path: "" },
  });
  await act(async () => root.render(<Harness mode="native" readOnly />));
  expect(button("添加服务")).toBeUndefined();
  expect(
    [...host.querySelectorAll("input,select")].every(
      (el) => (el as HTMLInputElement).disabled,
    ),
  ).toBe(true);
});
it("refuses duplicate keys, aliases and malformed collections without discarding original source", () => {
  expect(() => readManifest(fixture + "\nservices: []")).toThrow();
  expect(() =>
    readManifest("metadata: &m {name: test}\nservices: [*m]"),
  ).toThrow("别名");
  expect(() =>
    readManifest(
      "metadata: {name: test}\nservices: [{name: test, env: wrong}]",
    ),
  ).toThrow("env");
  expect(() =>
    readManifest("metadata: {name: test}\nservices: [wrong]"),
  ).toThrow("services");
});
