import { act,useCallback,useState } from "react";
import { createRoot,type Root } from "react-dom/client";
import { afterEach,beforeEach,expect,it,vi } from "vitest";
import { parse } from "yaml";
import { named } from "../../shared/api/resource";
import { hostRepositoryText,i18n } from "../../shared/i18n/i18n";
import { ManifestEditor,readManifest } from "./ManifestEditor";

vi.mock("../../shared/editor/CodeEditor", () => ({ CodeEditor: ({ value, label, readOnly, onChange }: { value: string; label: string; readOnly: boolean; onChange: (value: string) => void }) => <textarea aria-label={label} value={value} readOnly={readOnly} onChange={event => onChange(event.target.value)} /> }));

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
it("shows empty artifact references and preserves unavailable IDs without claiming deletion", async () => {
  const onChange=vi.fn();
  const render=async(pipeline:string,run:string)=>{
    const value=fixture.replace('    image: nginx:alpine # keep image comment',`    artifact: {source: pipeline, pipeline: '${pipeline}', run: '${run}', path: dist}`);
    await act(async()=>root.render(<ManifestEditor workspace="ops" value={value} mode="native" readOnly={false} onChange={onChange} onValidityChange={()=>{}}/>));
  };
  const fetch=vi.fn(async(_url:string)=>({ok:true,status:200,json:async()=>({items:[],total:0,page_size:200})}));
  vi.stubGlobal('fetch',fetch);
  await render('0','0');
  expect(reference('流水线').textContent).toBe('请选择流水线');expect(reference('固定构建').textContent).toBe('最新成功构建');
  expect(fetch.mock.calls.every(call=>!String(call[0]).includes('pipeline-runs'))).toBe(true);
  await render('missing','saved-run');
  expect(reference('流水线').textContent).toBe(named(undefined,'missing'));
  expect(reference('固定构建').textContent).toBe(named(undefined,'saved-run'));
  expect(onChange).not.toHaveBeenCalled();
});
it("shows a successful artifact build without a completion timestamp as unavailable time", async () => {
  vi.stubGlobal('fetch',vi.fn(async(url:string)=>({ok:true,status:200,json:async()=>({items:url.includes('pipeline-runs')?[{id:'r1',number:1,finished_at:0}]:[{id:'p1',name:'Build'}],total:1,page_size:200})})));
  const value=fixture.replace('    image: nginx:alpine # keep image comment','    artifact: {source: pipeline, pipeline: p1, run: r1, path: dist}');
  await act(async()=>root.render(<ManifestEditor workspace="ops" value={value} mode="native" readOnly={false} onChange={()=>{}} onValidityChange={()=>{}}/>));
  expect(reference('固定构建').textContent).toBe('#1 · —');
});
let root: Root, host: HTMLDivElement, latest: string, blocked: boolean;
const reference=(label:string)=>[...host.querySelectorAll('[data-auto-field]')].find(field=>field.firstElementChild?.textContent?.trim().startsWith(label))!.querySelector('[role="combobox"]')!;
function Harness({ source = fixture, readOnly = false, mode = "compose", workspace }: { source?: string; readOnly?: boolean; mode?: string; workspace?: string }) {
  const [value, setValue] = useState(source);
  latest = value;
  const validity = useCallback((value: boolean) => {
    blocked = value;
  }, []);
  return (
    <ManifestEditor
      value={value}
      {...(workspace ? { workspace } : {})}
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
  [...region.querySelectorAll("[data-auto-field]")]
    .find((l) => l.firstElementChild?.textContent?.trim().startsWith(name))!
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
  void i18n.changeLanguage('zh-CN');
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
  await i18n.changeLanguage('en-US');
});
it("switches editor labels and existing validation feedback in six languages without changing YAML",async()=>{
 await act(async()=>root.render(<Harness/>));
 const original=latest;
 for(const locale of ['en-US','zh-CN','zh-TW','ja-JP','ko-KR','ms-MY']){
  await act(async()=>{await i18n.changeLanguage(locale);});
  expect(host.textContent).toContain(hostRepositoryText('manifest.016'));
  expect(host.textContent).toContain(hostRepositoryText('manifest.032'));
  expect(latest).toBe(original);
  expect(host.textContent).toContain('nginx:alpine');
 }
 await act(async()=>root.render(<ManifestEditor value={'metadata: &shared {}\nservices: []\nshared: *shared\n'} mode="compose" readOnly={false} onChange={()=>{}} onValidityChange={()=>{}}/>));
 for(const locale of ['en-US','zh-CN','zh-TW','ja-JP','ko-KR','ms-MY']){
  await act(async()=>{await i18n.changeLanguage(locale);});
  expect(host.textContent).toContain(hostRepositoryText('manifest.001'));
 }
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
it("switches a service's run source immediately without a browser confirmation", async () => {
  const confirm = vi.fn(() => { throw new Error("native confirm must not be used"); });
  vi.stubGlobal("confirm", confirm);
  await act(async () => root.render(<Harness mode="native" />));
  expect(host.textContent).toContain("Native 模式需要选择流水线构建产物。");
  await fill(host.querySelector('select[aria-label="运行来源"]')!, "artifact");
  expect(confirm).not.toHaveBeenCalled();
  expect(parse(latest).services[0].artifact).toEqual({ source: "pipeline", pipeline: "", path: "" });
  expect(parse(latest).services[0].image).toBeUndefined();
  expect(host.querySelector('select[aria-label="运行来源"]')!.closest("[data-auto-field]")?.parentElement?.textContent).not.toContain("Native 模式需要选择流水线构建产物。");
  expect(input("流水线 ID")).toBeDefined();
});
it.each(["docker", "compose", "stack"])("limits %s sources to images while preserving incompatible draft YAML", async mode => {
  const source=fixture.replace('    image: nginx:alpine # keep image comment','    artifact: {source: pipeline, pipeline: build, path: bin/web}');
  await act(async () => root.render(<Harness mode={mode} source={source} />));
  const picker=host.querySelector<HTMLSelectElement>('select[aria-label="运行来源"]')!;
  expect([...picker.options].filter(option=>!option.disabled).map(option=>option.value)).toEqual(["image"]);
  expect(picker.value).toBe("");
  expect(picker.validity.valueMissing).toBe(true);
  expect(latest).toBe(source);
  await fill(picker,"image");
  expect(parse(latest).services[0].artifact).toBeUndefined();
  expect(parse(latest).services[0].image).toBe("");
  expect(input("镜像").required).toBe(true);
});
it("limits Native to artifacts and preserves dual-source v2 definitions across engines", async () => {
  await act(async () => root.render(<Harness mode="native" />));
  let picker=host.querySelector<HTMLSelectElement>('select[aria-label="运行来源"]')!;
  expect([...picker.options].filter(option=>!option.disabled).map(option=>option.value)).toEqual(["artifact"]);
  expect(picker.value).toBe("");
  expect(latest).toBe(fixture);
  await fill(picker,"artifact");
  await fill(input("应用定义版本"),"ferry/v2");
  await fill(picker,"both");
  const expected=latest;
  for(const mode of ["docker","compose","stack","native"]) {
    await act(async()=>root.render(<Harness mode={mode} />));
    picker=host.querySelector<HTMLSelectElement>('select[aria-label="运行来源"]')!;
    expect(picker.value).toBe("both");
    expect([...picker.options].filter(option=>!option.disabled).map(option=>option.value)).toEqual(["both",mode==="native"?"artifact":"image"]);
    expect(latest).toBe(expected);
  }
});
it("shows an incompatible source accurately in read-only mode without rewriting it", async () => {
  const source=fixture.replace('    image: nginx:alpine # keep image comment','    artifact: {source: pipeline, pipeline: build, path: bin/web}');
  await act(async()=>root.render(<Harness mode="compose" source={source} readOnly />));
  const picker=host.querySelector<HTMLSelectElement>('select[aria-label="运行来源"]')!;
  expect(picker.disabled).toBe(true);
  expect(picker.value).toBe("artifact");
  expect(picker.selectedOptions[0]!.textContent).toBe(hostRepositoryText("manifest.035"));
  expect(latest).toBe(source);
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

it("v2 preserves both sources when the target engine or editor mode changes and blocks destructive downgrade", async () => {
  await act(async () => root.render(<Harness />));
  await fill(input("应用定义版本"), "ferry/v2");
  await fill(input(hostRepositoryText("manifest.033")), "both");
  await fill(input(hostRepositoryText("manifest.042")), "build");
  await fill(input(hostRepositoryText("manifest.043")), "bin/web");
  const expected = parse(latest);
  expect(expected.services[0]).toMatchObject({ image: "nginx:alpine", artifact: { source: "pipeline", pipeline: "build", path: "bin/web" } });
  await act(async () => root.render(<Harness mode="native" />));
  expect(parse(latest)).toEqual(expected);
  expect(input("镜像").value).toBe("nginx:alpine");
  expect(input(hostRepositoryText("manifest.043")).value).toBe("bin/web");
  await click("源码编辑"); await click("表单编辑");
  expect(parse(latest)).toEqual(expected);
  await fill(input("应用定义版本"), "ferry/v1");
  expect(parse(latest)).toEqual(expected);
  expect(host.textContent).toContain("请先移除双运行来源或放置约束");
  expect(latest).toContain("# original application");
});
it("edits typed Swarm placement and preserves unsupported constraints when changing engines", async () => {
  await act(async () => root.render(<Harness source={fixture.replace("ferry/v1", "ferry/v2")} mode="stack" />));
  await fill(input("节点约束"), "node.role == worker\nnode.labels.zone != zone-a");
  await fill(input("每节点副本上限"), "1");
  expect(parse(latest).services[0].placement).toEqual({ constraints: ["node.role == worker", "node.labels.zone != zone-a"], maxReplicasPerNode: 1 });
  const expected = latest;
  await act(async () => root.render(<Harness mode="compose" />));
  expect(latest).toBe(expected);
  expect(host.textContent).toContain("当前部署方式不支持放置约束");
  await click("清除放置配置");
  expect(parse(latest).services[0].placement).toBeUndefined();
});
it("capability checks use current unsaved YAML and clear obsolete results after editing", async () => {
  const fetch = vi.fn(async (_url: string, _options?: RequestInit) => ({ ok: true, status: 200, json: async () => ({ checks: [{ mode: "compose", supported: true }, { mode: "native", supported: false, reason: "Native requires artifact" }] }) }));
  vi.stubGlobal("fetch", fetch);
  await act(async () => root.render(<Harness workspace="ops" />));
  await click("检查部署兼容性");
  expect(fetch.mock.calls[0]?.[0]).toBe("/api/workspaces/ops/deployment-capabilities");
  expect(host.textContent).toContain("配置可渲染");
  expect(host.textContent).toContain("Native requires artifact");
  await fill(input("镜像"), "nginx:stable");
  expect(host.textContent).not.toContain("Native requires artifact");
});
