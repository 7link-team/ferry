import { act } from "react";
import { createRoot,type Root } from "react-dom/client";
import { afterEach,beforeEach,expect,it,vi } from "vitest";
import { DEFAULT_ACTIONS } from "../../features/pipelines/actions-workflow";
import { i18n } from "../../shared/i18n/i18n";
import { formConfigs } from "../resources";
import { ResourceActionDialog, ResourceForm } from "./ResourceForm";
vi.mock("../../features/pipelines/YamlEditor", () => ({
  YamlEditor: ({ value, readOnly, onChange }: { value: string; readOnly: boolean; onChange: (value: string) => void }) =>
    <textarea aria-label="GitHub Actions YAML" value={value} readOnly={readOnly} onChange={event => onChange(event.target.value)} />,
}));
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
  void i18n.changeLanguage("zh-CN");
  localStorage.setItem("ui.locale", "zh-CN");
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  history.replaceState({}, "", "/ops/deploy/variables/10");
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  requests = [];
  stored = { id: "10", name: "db-url", kind: "secret", scope: "global", scope_ref: "0", has_value: true, version: 1 };
  vi.stubGlobal("fetch", vi.fn(async (url: string, init: RequestInit = {}) => {
    requests.push({ url, method: init.method, body: init.body as string });
    return { ok: true, status: 200, json: async () => url.endsWith("/disclosures") ? { value: "secret-value\n$literal" } : stored };
  }));
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); });
it("resets dependent arrays as arrays and scalar fields as strings before creating a resource", async () => {
  history.replaceState({}, "", "/zh-CN/workspaces/ops/variables/new");
  const config = {
    resource: "variables", heading: () => "Report",
    blank: { source: "", author_keys: ["old"], applications: [], branch: "main" },
    sections: [{ fields: [{ key: "source", label: "Source", type: "text" as const, resetOnChange: ["author_keys", "applications", "branch"] }] }],
  };
  await act(async () => root.render(<ResourceForm config={config} workspace="ops" permissions={new Set(["variables.create"])} />));
  const input = host.querySelector<HTMLInputElement>('input[name="source"]')!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "new-source");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await act(async () => host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
  const saved = requests.find((request) => request.method === "POST");
  expect(saved).toBeDefined();
  expect(JSON.parse(saved!.body!)).toMatchObject({ source: "new-source", author_keys: [], applications: [], branch: "" });
});
it("merges custom action field patches without nesting them or dropping sibling values", async () => {
  const action = vi.fn();
  await act(async () => root.render(<ResourceActionDialog workspace="ops" permissions={perms} row={{}} onClose={() => {}} onDone={() => {}} action={{
    label: "保存渠道", action, initialValues: () => ({ enabled: true, channels: ["Operations"] }),
    parameters: () => [{ key: "channels", label: "渠道", type: "custom", render: ({ draft, onPatch }) => <button type="button" onClick={() => onPatch({ channels: [...(draft.channels as string[]), "Development"] })}>{(draft.channels as string[]).join(",")}</button> }],
  }} />));
  await act(async () => document.querySelector<HTMLButtonElement>('[role="dialog"] form button[type="button"]')!.click());
  expect(document.querySelector('[role="dialog"]')!.textContent).toContain("Operations,Development");
  await act(async () => document.querySelector<HTMLFormElement>('[role="dialog"] form')!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
  expect(action).toHaveBeenCalledWith({}, { enabled: true, channels: ["Operations", "Development"] });
});
it("copies an immutable public field and disables copying an empty value", async () => {
  const writeText = vi.fn(async () => {});
  vi.stubGlobal("navigator", { clipboard: { writeText } });
  const config = { resource: "variables", heading: () => "配置", blank: {}, sections: [{ fields: [{ key: "name", label: "回调地址", type: "text" as const, immutable: true, copyable: true }] }] };
  history.replaceState({}, "", "/ops/deploy/variables/10/edit");
  await act(async () => root.render(<ResourceForm config={config} workspace="ops" permissions={perms} />));
  const copy = host.querySelector<HTMLButtonElement>('button[aria-label="复制 回调地址"]')!;
  expect(copy.disabled).toBe(false);
  await act(async () => copy.click());
  expect(writeText).toHaveBeenCalledWith("db-url");
  await act(async () => root.render(<div />));
  stored.name = "";
  await act(async () => root.render(<ResourceForm config={config} workspace="ops" permissions={perms} />));
  expect(host.querySelector<HTMLButtonElement>('button[aria-label="复制 回调地址"]')!.disabled).toBe(true);
});
it("offers dictionary suggestions on a text field while preserving arbitrary custom names", async () => {
  const action = vi.fn(async () => {});
  await act(async () => root.render(<ResourceActionDialog workspace="ops" permissions={new Set()} row={{}} onClose={() => {}} onDone={() => {}} action={{
    label: "Create environment", action, parameters: () => [{ key: "name", label: "Environment name", type: "text", required: true, options: [{ value: "dev", label: "Development" }] }],
  }} />));
  const input = document.querySelector<HTMLInputElement>('input[aria-label="Environment name"]')!;
  expect(input.list?.options[0]!.value).toBe("dev");
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "custom-production-region");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await act(async () => document.querySelector<HTMLFormElement>('[role="dialog"] form')!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
  expect(action).toHaveBeenCalledWith({}, { name: "custom-production-region" });
});
it("keeps manual environment names editable when optional dictionary suggestions cannot load", async () => {
  const action = vi.fn(async () => {});
  await act(async () => root.render(<ResourceActionDialog workspace="ops" permissions={new Set()} row={{}} onClose={() => {}} onDone={() => {}} action={{
    label: "Create environment", initialValues: () => ({ name: "custom-name" }), action,
    parameters: () => [{ key: "name", label: "Environment name", type: "text", required: true, options: async () => { throw new Error("suggestions unavailable"); } }],
  }} />));
  expect(document.querySelector<HTMLInputElement>('input[aria-label="Environment name"]')!.disabled).toBe(false);
  expect(document.querySelector('[role="alert"]')?.textContent).toContain("suggestions unavailable");
  await act(async () => document.querySelector<HTMLFormElement>('[role="dialog"] form')!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
  expect(action).toHaveBeenCalledWith({}, { name: "custom-name" });
});
it("secret detail only fetches metadata; reveal is explicit, audited endpoint and can hide", async () => {
  await mount();
  expect(host.textContent).toContain("已设置");
  expect(requests).toHaveLength(1);
  expect(host.textContent).not.toContain("secret-value");
  await click("查看明文（记录审计）");
  expect(requests.at(-1)).toMatchObject({ url: "/api/workspaces/ops/variables/10/disclosures", method: "POST" });
  expect(host.querySelector('[aria-label="变量明文"]')?.textContent).toBe("secret-value\n$literal");
  await click("隐藏明文");
  expect(host.textContent).not.toContain("secret-value");
});
it("renders a header view action once, not again in untitled read-only sections", async () => {
  history.replaceState({}, "", "/ops/deploy/alert-events/7");
  stored = { id: "7", rule_name: "CPU high", state: "firing", message: "cpu > 90", trigger_delivery: "已投递", recovery_delivery: "" };
  await act(async () => root.render(<ResourceForm config={formConfigs("ops", new Set(["alerts.read"]))["alert-events"]!} workspace="ops" permissions={new Set(["alerts.read"])} />));
  expect(host.textContent).toContain("CPU high");
  expect(host.querySelectorAll('a[aria-label="返回规则"]')).toHaveLength(1);
});
it("lists deployment modes from native through Docker, Compose and Swarm", () => {
  const field = formConfigs("ops").deployments!.sections.flatMap(section => section.fields).find(field => field.key === "mode")!;
  expect((field.options as { value: string }[]).map(option => option.value)).toEqual(["native", "docker", "compose", "stack"]);
});
it("shows semantic empty labels for zero host references and resolves real group and jump names", async () => {
  history.replaceState({}, "", "/ops/deploy/hosts/17");
  stored = { id: "17", name: "SSH target", connection: "ssh", group_id: "0", jump_host_id: 0, credential_id: "8", address: "target.example.invalid", port: 22, username: "root", tag_ids: [] };
  vi.stubGlobal("fetch", vi.fn(async (url: string) => ({ ok: true, status: 200, json: async () => url.includes("/nodes/17") ? stored : { items: url.includes("node-groups") ? [{ id: "2", name: "Production group" }] : url.includes("credentials") ? [{ id: "8", name: "SSH login", kind: "password" }] : url.includes("/nodes?") ? [{ id: "3", name: "Jump server", jump_host_id: "0" }] : [], total: 1, page: 1, page_size: 200 } })));
  const config = formConfigs("ops").hosts!;
  const render = () => root.render(<ResourceForm config={config} workspace="ops" permissions={new Set(["hosts.read"])} />);
  await act(async () => render());
  expect(host.textContent).toContain("未分组");
  expect(host.textContent).toContain("不使用跳板主机");
  expect(host.textContent).toContain("SSH login");
  expect(host.textContent).not.toContain("选项不可用（0）");
  expect(stored.group_id).toBe("0"); expect(stored.jump_host_id).toBe(0);
  await act(async () => root.unmount());
  root = createRoot(host);
  stored = { ...stored, group_id: "2", jump_host_id: "3" };
  await act(async () => render());
  expect(host.textContent).toContain("Production group");
  expect(host.textContent).toContain("Jump server");
});
it("shows a repository event's commits, links and times, and omits an unknown operator", async () => {
  history.replaceState({}, "", "/ops/build/repository-events/8");
  stored = {
    id: "8", connection_id: "3", event: "push", repository: "team/app", reference: "main", actor: "", action: "轮询检测：分支提交发生变化",
    created_at: Date.UTC(2026, 10, 1, 2, 0), delivery: "已投递企业微信", source: "polling", before: "b0", after: "a1", total_commits: 2,
    compare_url: "https://code.example/compare", note: "最近的提交中未找到推送前的版本，可能是强制推送",
    commits: [
      { id: "a1b2c3d4e5", short_id: "a1b2c3d4", message: "feat: login\n\nadds SSO", author: "Ann", author_username: "ann", time: "2026-11-01T02:00:00Z", url: "https://code.example/c/a1" },
      { id: "f0", short_id: "f0", message: "fix: typo", author: "Bo", url: "javascript:alert(1)" },
    ],
  };
  const permissions = new Set(["repositories.read"]);
  await act(async () => root.render(<ResourceForm config={formConfigs("ops", permissions)["repository-events"]!} workspace="ops" permissions={permissions} />));
  expect(requests[0]!.url).toBe("/api/workspaces/ops/repository-events/8");
  expect(formConfigs("ops", permissions)["repository-events"]!.heading(stored)).toBe("代码推送 · team/app");
  expect(host.textContent).not.toContain("操作人");
  expect(host.textContent).not.toContain("未知");
  expect(host.textContent).toContain(new Date(Number(stored.created_at)).toLocaleString("zh-CN"));
  const list = host.querySelector('ol[aria-label="提交列表"]')!;
  expect(list.querySelectorAll("li")).toHaveLength(2);
  expect(list.textContent).toContain("a1b2c3d4");
  expect(list.textContent).toContain("Ann（ann）");
  expect(list.textContent).toContain("feat: login\n\nadds SSO");
  const links = [...list.querySelectorAll("a")].map(a => a.getAttribute("href"));
  expect(links).toEqual(["https://code.example/c/a1"]);
  expect(host.querySelector('a[href="https://code.example/compare"]')).not.toBeNull();
  expect(host.textContent).toContain("可能是强制推送");
  expect(host.querySelector('section[aria-label="合并请求"]')).toBeNull();
  expect(host.querySelector('a[aria-label="返回仓库连接"]')).toBeNull();
  const back = host.querySelector('a[aria-label="返回列表"]');
  expect(back?.getAttribute("href")).toContain("repository-events");

  history.replaceState({}, "", "/ops/build/repository-events/8?from_connection=3&page=2");
  await act(async () => root.render(<ResourceForm config={formConfigs("ops", permissions)["repository-events"]!} workspace="ops" permissions={permissions} />));
  const connectionBack = host.querySelector('a[aria-label="返回仓库连接"]')?.getAttribute("href");
  expect(connectionBack).toContain("tab=events");
  expect(connectionBack).toContain("page=2");
  expect(connectionBack).not.toContain("from_connection");
});
it("read permission never offers reveal", async () => {
  await mount(new Set(["variables.read"]));
  expect(host.textContent).not.toContain("查看明文");
  expect(requests).toHaveLength(1);
});
it("keeps the selected host when creating an alert rule from its detail page", async () => {
  history.replaceState({}, "", "/ops/deploy/alerts/new?target_kind=host&target_ref=42");
  vi.stubGlobal("fetch", vi.fn(async (url: string, init: RequestInit = {}) => {
    requests.push({ url, method: init.method, body: init.body as string });
    return { ok: true, status: 200, json: async () => url.includes("/nodes")
      ? { items: [{ id: "42", name: "Node 42" }], total: 1, page: 1, page_size: 200 }
      : { id: "99" } };
  }));
  const permissions = new Set(["alerts.create", "hosts.read"]);
  await act(async () => root.render(<ResourceForm config={formConfigs("ops", permissions)["alert-rules"]!} workspace="ops" permissions={permissions} />));
  expect(host.textContent).toContain("Node 42");
  await act(async () => {
    const name = host.querySelector<HTMLInputElement>('input[name="name"]')!;
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(name, "Node 42 offline");
    name.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await act(async () => host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
  const request = requests.find(row => row.method === "POST" && row.url.endsWith("/alert-rules"));
  expect(JSON.parse(request!.body!)).toMatchObject({ target_kind: "host", target_ref: "42" });
});
it("editing a secret omits its value unless replacement is explicitly enabled", async () => {
  history.replaceState({}, "", "/ops/deploy/variables/10/edit");
  await mount();
  expect(host.querySelectorAll("textarea")).toHaveLength(1); // description only
  await act(async () => host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
  const unchanged = requests.find(r => r.method === "PATCH");
  expect(JSON.parse(unchanged!.body!)).not.toHaveProperty("value");
});
it("shows an unset secret input immediately without sending a replacement until edited", async () => {
  stored.has_value = false;
  history.replaceState({}, "", "/ops/deploy/variables/10/edit");
  await mount();
  const input = host.querySelector<HTMLTextAreaElement>('textarea[autocomplete="off"]');
  expect(input).not.toBeNull();
  expect(input!.value).toBe("");
  await act(async () => host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
  expect(JSON.parse(requests.find(r => r.method === "PATCH")!.body!)).not.toHaveProperty("value");
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
  expect(requests.filter(r => r.url.endsWith("/disclosures"))).toHaveLength(1);
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
  vi.stubGlobal("fetch", vi.fn(async (url: string) => ({ ok: true, status: 200, json: async () => url.endsWith("/renderings")
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
      : url.endsWith("/pipelines/pipeline-1/runs") ? { id: "run-1" }
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
    "/api/workspaces/ops/pipelines/pipeline-1/runs",
  ]);
  expect(location.pathname).toBe("/zh-CN/workspaces/ops/pipeline-runs/run-1");
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
  expect(requests.some(row => row.url.endsWith("/runs"))).toBe(false);
  await act(async () => (Array.from(document.querySelectorAll<HTMLButtonElement>('[role="dialog"] button')).find(button => button.textContent === "取消")!).click());
  expect(location.pathname).toBe("/zh-CN/workspaces/ops/pipelines/pipeline-1/edit");
});
it("shows an install command immediately in the runner host mode", async () => {
  history.replaceState({}, "", "/ops/host/nodes/new");
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
  expect(requests.some(row => row.url.endsWith("/nodes") && row.method === "POST")).toBe(false);
  expect(host.querySelector<HTMLTextAreaElement>('textarea[aria-label="安装命令"]')?.value).toContain("fixture-token");
  await act(async () => host.querySelector<HTMLButtonElement>('button[role="tab"][aria-selected="false"]')!.click());
  expect(host.textContent).toContain("添加主机");
  expect(host.querySelector('[role="tablist"][aria-label="接入方式"]')).not.toBeNull();
  expect(host.querySelector('[role="tablist"][aria-label="接入方式"]')?.className).toContain("sticky");
  expect(host.querySelector<HTMLButtonElement>('button[role="tab"][aria-selected="true"]')?.textContent).toBe("SSH");
  expect(host.textContent).toContain("主机地址");
  expect(host.textContent).toContain("跳板主机");
  expect(host.textContent).toContain("多个目标主机可共用它");
  expect(host.textContent).toContain("添加登录凭据");
  expect(host.textContent).not.toContain("接入方式*");
  await act(async () => host.querySelector<HTMLButtonElement>('button[role="tab"][aria-selected="false"]')!.click());
  expect(host.querySelector<HTMLButtonElement>('button[role="tab"][aria-selected="true"]')?.textContent).toBe("主机代理");
  expect(host.querySelector<HTMLTextAreaElement>('textarea[aria-label="安装命令"]')?.value).toContain("fixture-token");
});
it("shows empty host tags as read-only data without creation instructions", async () => {
  history.replaceState({}, "", "/ops/host/nodes/10");
  vi.stubGlobal("fetch", vi.fn(async (url: string) => ({ ok: true, status: 200, json: async () => url.endsWith("/nodes/10")
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
