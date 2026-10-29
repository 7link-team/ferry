import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { beforeEach, afterEach, expect, it, vi } from "vitest";
import { ResourceForm } from "./ResourceForm";
import { formConfigs } from "../resources";
import { DEFAULT_ACTIONS } from "../actions-workflow";
let root: Root, host: HTMLDivElement, requests: { url: string; method: string | undefined; body: string | undefined }[];
let stored: Record<string, unknown>;
const perms = new Set(["variables.read", "variables.update", "variables.reveal"]);
const mount = async (permissions = perms) => {
  await act(async () => root.render(<ResourceForm config={formConfigs("ops").variables!} workspace="ops" permissions={permissions} />));
};
const click = async (text: string) => {
  const button = [...host.querySelectorAll("button")].find(button => button.textContent === text);
  expect(button).toBeDefined();
  await act(async () => button!.click());
};
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  history.replaceState({}, "", "/ops/deploy/variables/10");
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  requests = [];
  stored = { id: "10", name: "db-url", kind: "secret", scope: "global", scope_ref: "0", has_value: true, version: 1 };
  vi.stubGlobal("fetch", vi.fn(async (url: string, init: RequestInit = {}) => {
    requests.push({ url, method: init.method, body: init.body as string });
    return { ok: true, status: 200, json: async () => url.endsWith("/reveal") ? { value: "secret-value\n$literal" } : stored };
  }));
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); });
it("secret detail only fetches metadata; reveal is explicit, audited endpoint and can hide", async () => {
  await mount();
  expect(host.textContent).toContain("已设置");
  expect(requests).toHaveLength(1);
  expect(host.textContent).not.toContain("secret-value");
  await click("查看明文（记录审计）");
  expect(requests.at(-1)).toMatchObject({ url: "/api/workspaces/ops/variables/10/actions/reveal", method: "POST" });
  expect(host.querySelector('[aria-label="变量明文"]')?.textContent).toBe("secret-value\n$literal");
  await click("隐藏明文");
  expect(host.textContent).not.toContain("secret-value");
});
it("read permission never offers reveal", async () => {
  await mount(new Set(["variables.read"]));
  expect(host.textContent).not.toContain("查看明文");
  expect(requests).toHaveLength(1);
});
it("editing a secret omits its value unless replacement is explicitly enabled", async () => {
  history.replaceState({}, "", "/ops/deploy/variables/10/edit");
  await mount();
  expect(host.querySelectorAll("textarea")).toHaveLength(1); // description only
  await act(async () => host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
  const unchanged = requests.find(r => r.method === "PATCH");
  expect(JSON.parse(unchanged!.body!)).not.toHaveProperty("value");
});
it("an explicitly selected empty replacement sends an empty string", async () => {
  history.replaceState({}, "", "/ops/deploy/variables/10/edit");
  await mount();
  await act(async () => (host.querySelector('[role="checkbox"]') as HTMLElement).click());
  expect(host.querySelectorAll("textarea")).toHaveLength(2);
  await act(async () => host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
  expect(JSON.parse(requests.find(r => r.method === "PATCH")!.body!)).toHaveProperty("value", "");
});
it("unmounting a revealed detail does not carry plaintext into another detail", async () => {
  await mount(); await click("查看明文（记录审计）");
  await act(async () => root.render(<div />));
  await mount();
  expect(host.textContent).not.toContain("secret-value");
  expect(requests.filter(r => r.url.endsWith("/reveal"))).toHaveLength(1);
});
it("pipeline creation uses a wide form and keeps repository/host settings out of basic fields", async () => {
  history.replaceState({}, "", "/ops/build/pipelines/new");
  vi.stubGlobal("fetch", vi.fn(async () => ({ok:true,status:200,json:async()=>({items:[],total:0,page:1,page_size:200})})));
  const permissions = new Set(["pipelines.create","hosts.read","tags.read","tags.create","pipeline-environments.read","pipeline-environments.create"]);
  await act(async () => root.render(<ResourceForm config={formConfigs("ops", permissions).pipelines!} workspace="ops" permissions={permissions}/>));
  expect(host.querySelector("form")?.className).not.toContain("max-w-3xl");
  expect(host.textContent).not.toContain("构建代码来源");
  expect(host.textContent).not.toContain("目标类型");
  expect(host.textContent).toContain("新建标签");
  expect(host.textContent).not.toContain("执行环境");
});
it("opens a template list directly in the pipeline import dialog", async () => {
  vi.stubGlobal("fetch", vi.fn(async () => ({ok:true,status:200,json:async()=>({items:[],total:0,page:1,page_size:200})})));
  const permissions = new Set(["pipelines.create", "pipeline-templates.read", "deployments.create", "app-templates.read"]);
  history.replaceState({}, "", "/ops/build/pipelines/new");
  await act(async () => root.render(<ResourceForm config={formConfigs("ops", permissions).pipelines!} workspace="ops" permissions={permissions} pipelineWorkspace />));
  expect(host.querySelector('section[aria-label="从模板生成"]')).toBeNull();
  await click("从模板导入");
  expect(document.querySelector('[role="dialog"] [role="listbox"]')).not.toBeNull();
  expect(document.querySelector('[role="dialog"] [data-slot="select-trigger"]')).toBeNull();
  await act(async () => (document.querySelector('[role="dialog"] button[aria-label="Close"]') as HTMLButtonElement | null)?.click());

  history.replaceState({}, "", "/ops/deploy/deployments/new");
  await act(async () => root.render(<ResourceForm config={formConfigs("ops", permissions).deployments!} workspace="ops" permissions={permissions}/>));
  expect(host.querySelector('section[aria-label="从模板生成"]')?.className).toContain("border");
});
it("exports the current Actions workflow and executor bindings as a pipeline template", async () => {
  history.replaceState({}, "", "/ops/build/pipelines/new");
  const permissions = new Set(["pipelines.create", "pipeline-templates.create"]);
  vi.stubGlobal("fetch", vi.fn(async (url: string, init: RequestInit = {}) => {
    requests.push({ url, method: init.method, body: init.body as string });
    return { ok: true, status: init.method === "POST" ? 201 : 200, json: async () => init.method === "POST" ? { id: "template-1" } : { items: [], total: 0, page: 1, page_size: 200 } };
  }));
  await act(async () => root.render(<ResourceForm config={formConfigs("ops", permissions).pipelines!} workspace="ops" permissions={permissions} pipelineWorkspace />));
  await click("保存为模板");
  const name = document.querySelector<HTMLInputElement>('[role="dialog"] input')!;
  await act(async () => { Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(name, "Build template"); name.dispatchEvent(new Event("input", { bubbles: true })); });
  await act(async () => [...document.querySelectorAll<HTMLButtonElement>('[role="dialog"] button')].find(button => button.textContent === "保存模板")!.click());
  const payload = JSON.parse(requests.find(row => row.url.endsWith("/pipeline-templates") && row.method === "POST")!.body!);
  expect(JSON.parse(payload.content)).toMatchObject({ engine: "github-actions", workflow: DEFAULT_ACTIONS, executor_bindings: [] });
});
it("imports an Actions template into the visual editor with task bindings", async () => {
  const permissions = new Set(["pipeline-templates.read"]);
  const config = formConfigs("ops", permissions).pipelines!;
  const generator = config.generator!;
  vi.stubGlobal("fetch", vi.fn(async (url: string) => ({ ok: true, status: 200, json: async () => url.endsWith("/actions/render")
    ? { definition: JSON.stringify({ engine: "github-actions", workflow: DEFAULT_ACTIONS, executor_bindings: [{ job_id: "build", target_kind: "host", target_ref: "1", image: "host" }], checkout_bindings: [], graph_layout: {} }) }
    : { name: "Build template", version: 1, parameters: "[]" } })));
  const imported = await generator.generate("template-1", 1, {});
  expect(imported).toMatchObject({ definition_format: "github-actions", definition: DEFAULT_ACTIONS, executor_bindings: [{ job_id: "build", target_ref: "1" }] });
});
it("saves and runs a pipeline from the full-page action bar", async () => {
  history.replaceState({}, "", "/ops/build/pipelines/new");
  const permissions = new Set(["pipelines.create", "pipelines.run"]);
  vi.stubGlobal("fetch", vi.fn(async (url: string, init: RequestInit = {}) => {
    requests.push({ url, method: init.method, body: init.body as string });
    const result = url.endsWith("/pipelines") && init.method === "POST" ? { id: "pipeline-1" }
      : url.endsWith("/pipelines/pipeline-1") ? { id: "pipeline-1", name: "Build", environments: [], version: 1 }
      : url.endsWith("/pipelines/pipeline-1/actions/run") ? { id: "run-1" }
      : { items: [], total: 0, page: 1, page_size: 200 };
    return { ok: true, status: init.method === "POST" ? 201 : 200, json: async () => result };
  }));
  const config = formConfigs("ops", permissions).pipelines!;
  config.blank = { ...config.blank, executor_bindings: [{ job_id: "build", target_kind: "host", target_ref: "1", image: "host" }] };
  await act(async () => root.render(<ResourceForm config={config} workspace="ops" permissions={permissions} pipelineWorkspace />));
  const name = host.querySelector('input[aria-label="流水线名称"]')! as HTMLInputElement;
  await act(async () => { Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(name, "Build"); name.dispatchEvent(new Event("input", { bubbles: true })); });
  await act(async () => host.querySelector<HTMLButtonElement>('button[value="save-run"]')!.click());
  expect(requests.filter(row => row.method === "POST").map(row => row.url)).toEqual([
    "/api/workspaces/ops/pipelines",
    "/api/workspaces/ops/pipelines/pipeline-1/actions/run",
  ]);
  expect(location.pathname).toBe("/ops/build/runs/run-1");
});
it("asks for a bound environment after saving before starting a pipeline", async () => {
  history.replaceState({}, "", "/ops/build/pipelines/new");
  const permissions = new Set(["pipelines.create", "pipelines.run"]);
  vi.stubGlobal("fetch", vi.fn(async (url: string, init: RequestInit = {}) => {
    requests.push({ url, method: init.method, body: init.body as string });
    const result = url.endsWith("/pipelines") && init.method === "POST" ? { id: "pipeline-1" }
      : url.endsWith("/pipelines/pipeline-1") ? { id: "pipeline-1", name: "Build", environments: [{ id: "env-1", name: "Production" }], version: 1 }
      : { items: [], total: 0, page: 1, page_size: 200 };
    return { ok: true, status: init.method === "POST" ? 201 : 200, json: async () => result };
  }));
  const config = formConfigs("ops", permissions).pipelines!;
  config.blank = { ...config.blank, executor_bindings: [{ job_id: "build", target_kind: "host", target_ref: "1", image: "host" }] };
  await act(async () => root.render(<ResourceForm config={config} workspace="ops" permissions={permissions} pipelineWorkspace />));
  const name = host.querySelector('input[aria-label="流水线名称"]')! as HTMLInputElement;
  await act(async () => { Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(name, "Build"); name.dispatchEvent(new Event("input", { bubbles: true })); });
  await act(async () => host.querySelector<HTMLButtonElement>('button[value="save-run"]')!.click());
  expect(document.querySelector('[role="dialog"]')?.textContent).toContain("执行环境");
  expect(requests.some(row => row.url.endsWith("/actions/run"))).toBe(false);
  await act(async () => (Array.from(document.querySelectorAll<HTMLButtonElement>('[role="dialog"] button')).find(button => button.textContent === "取消")!).click());
  expect(location.pathname).toBe("/ops/build/pipelines/pipeline-1/edit");
});
it("shows an install command immediately in the runner host mode", async () => {
  history.replaceState({}, "", "/ops/host/hosts/new");
  const permissions = new Set(["hosts.create", "hosts.enroll", "credentials.create", "credentials.read"]);
  vi.stubGlobal("fetch", vi.fn(async (url: string, init: RequestInit = {}) => {
    requests.push({ url, method: init.method, body: init.body as string });
    const result = url.endsWith("/runner-tokens") && init.method === "POST" ? { id: "token-1", token: "fixture-token", server: "https://ops.example", expires_at: Date.now() + 900000 }
      : url.endsWith("/runner-tokens/token-1") ? { id: "token-1", state: "pending", host_state: "", host_id: "", used_at: 0 }
      : { items: [], total: 0, page: 1, page_size: 200 };
    return { ok: true, status: init.method === "POST" ? 201 : 200, json: async () => result };
  }));
  await act(async () => root.render(<ResourceForm config={formConfigs("ops", permissions).hosts!} workspace="ops" permissions={permissions} />));
  expect(JSON.parse(requests.find(row => row.url.endsWith("/runner-tokens") && row.method === "POST")!.body!)).toMatchObject({ host_id: "", ttl_minutes: 15 });
  expect(requests.some(row => row.url.endsWith("/hosts") && row.method === "POST")).toBe(false);
  expect(host.querySelector<HTMLTextAreaElement>('textarea[aria-label="安装命令"]')?.value).toContain("fixture-token");
  await act(async () => host.querySelector<HTMLButtonElement>('button[role="tab"][aria-selected="false"]')!.click());
  expect(host.textContent).toContain("添加主机");
  expect(host.querySelector('[role="tablist"][aria-label="接入方式"]')).not.toBeNull();
  expect(host.querySelector('[role="tablist"][aria-label="接入方式"]')?.className).toContain("sticky");
  expect(host.querySelector<HTMLButtonElement>('button[role="tab"][aria-selected="true"]')?.textContent).toBe("SSH");
  expect(host.textContent).toContain("主机地址");
  expect(host.textContent).toContain("添加登录凭据");
  expect(host.textContent).not.toContain("接入方式*");
  await act(async () => host.querySelector<HTMLButtonElement>('button[role="tab"][aria-selected="false"]')!.click());
  expect(host.querySelector<HTMLButtonElement>('button[role="tab"][aria-selected="true"]')?.textContent).toBe("代理 Runner");
  expect(host.querySelector<HTMLTextAreaElement>('textarea[aria-label="安装命令"]')?.value).toContain("fixture-token");
});
it("shows empty host tags as read-only data without creation instructions", async () => {
  history.replaceState({}, "", "/ops/host/hosts/10");
  vi.stubGlobal("fetch", vi.fn(async (url: string) => ({ ok: true, status: 200, json: async () => url.endsWith("/hosts/10")
    ? { id: "10", name: "web-01", connection: "runner", group_id: "0", tag_ids: [], description: "", version: 1 }
    : { items: [], total: 0, page: 1, page_size: 200 } })));
  await act(async () => root.render(<ResourceForm config={formConfigs("ops").hosts!} workspace="ops" permissions={new Set(["hosts.read"])} />));
  expect(host.textContent).toContain("标签");
  expect(host.textContent).not.toContain("暂无可选标签");
  expect(host.textContent).not.toContain("取消全部选择会清空关联");
});
it("creates a classification tag in place and selects it without losing the pipeline draft", async () => {
 history.replaceState({}, "", "/ops/build/pipelines/new");
 const tags:{id:string;name:string}[]=[];
 vi.stubGlobal("fetch",vi.fn(async(url:string,init:RequestInit={})=>{
   requests.push({url,method:init.method,body:init.body as string});
   if(url.endsWith('/tags')&&init.method==='POST'){const body=JSON.parse(String(init.body));tags.push({id:'tag-created',name:body.name});return {ok:true,status:201,json:async()=>({id:'tag-created'})};}
   return {ok:true,status:200,json:async()=>({items:url.includes('/tags?')?tags:[],total:url.includes('/tags?')?tags.length:0,page:1,page_size:200})};
 }));
 const permissions=new Set(['pipelines.create','tags.read','tags.create']);
 await act(async()=>root.render(<ResourceForm config={formConfigs('ops',permissions).pipelines!} workspace="ops" permissions={permissions}/>));
 const name=host.querySelector('input')!;
 await act(async()=>{Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value')!.set!.call(name,'Draft kept');name.dispatchEvent(new Event('input',{bubbles:true}));});
 await click('新建标签');
 const dialog=document.querySelector('[role="dialog"]')!;
 const input=dialog.querySelector('input')!;
 await act(async()=>{Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value')!.set!.call(input,'Release');input.dispatchEvent(new Event('input',{bubbles:true}));});
 await act(async()=>dialog.querySelector('form')!.dispatchEvent(new Event('submit',{bubbles:true,cancelable:true})));
 expect(requests.filter(row=>row.method==='POST')).toHaveLength(1);
 expect(host.textContent).toContain('Release');
 expect(host.querySelector('[role="checkbox"][data-state="checked"]')).not.toBeNull();
 expect(name.value).toBe('Draft kept');
});
