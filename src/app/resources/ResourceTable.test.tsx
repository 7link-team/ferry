import { act } from "react";
import { createRoot,type Root } from "react-dom/client";
import { afterEach,beforeEach,expect,it,vi } from "vitest";
import { listResource } from "../../shared/api/resource";
import { i18n } from "../../shared/i18n/i18n";
import type { ListConfig } from "../../shared/resources/types";
import { navigate } from "../router";
import { ResourceTable } from "./ResourceTable";

vi.mock("../router", () => ({
  useRoute: () => ({ workspace: "ops", group: "host", item: "hosts", mode: "list" }),
  useQueryValue: (_key: string, fallback: unknown) => [fallback, () => {}],
  useScrollSync: () => ({ current: null }),
  href: () => "/detail", navigate: vi.fn(),
}));
vi.mock("../../shared/api/resource", async original => ({ ...await original<typeof import("../../shared/api/resource")>(),
  listResource: vi.fn(async () => ({ items: [{ id: "1", name: "Alpha" }, { id: "2", name: "Beta" }], total: 2, page: 1, page_size: 20 })),
}));
let root: Root;
let element: HTMLDivElement;
beforeEach(() => {
  void i18n.changeLanguage("zh-CN");
  localStorage.setItem("ui.locale", "zh-CN");
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  element = document.createElement("div"); document.body.append(element); root = createRoot(element);
});
afterEach(async () => { await act(async () => root.unmount()); element.remove(); vi.clearAllMocks(); });
it("renders report metadata from the fetched envelope and hides stale charts after refresh failure", async () => {
  const envelope = { items: [{ id: "1", name: "Alpha" }], total: 4, page: 1, page_size: 1, coverage: { complete: false }, series: [{ id: "2", name: "Full series" }] };
  vi.mocked(listResource).mockResolvedValueOnce(envelope);
  const summary = vi.fn((data: typeof envelope) => <p>Report {data.series[0]!.name}</p>);
  const config: ListConfig<{ id: string; name: string }> = {
    resource: "report", rowKey: row => row.id, columns: [{ key: "name", label: "Name" }],
    summary: data => summary(data as typeof envelope), toolbar: { create: false },
  };
  await act(async () => root.render(<ResourceTable config={config} workspace="ops" permissions={new Set()} />));
  expect(summary).toHaveBeenLastCalledWith(envelope);
  expect(element.textContent).toContain("Report Full series");
  vi.mocked(listResource).mockRejectedValueOnce(new Error("Report unavailable"));
  await act(async () => [...element.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent?.trim() === "刷新")!.click());
  expect(element.textContent).toContain("Report unavailable");
  expect(element.textContent).not.toContain("Report Full series");
});
it("opens configured read-only details from the row or an explicit labeled action", async () => {
  const open = vi.fn();
  const config: ListConfig<{id:string;name:string}> = {
    resource:"executions", rowKey:row=>row.id, onRowClick:open,
    columns:[{key:"name",label:"名称"}], toolbar:{create:false},
    actions:[{label:"查看详情",showLabel:true,icon:<span data-eye />,action:async row=>{open(row);}}],
  };
  await act(async()=>root.render(<ResourceTable config={config} workspace="ops" permissions={new Set()} />));
  const details=element.querySelector<HTMLButtonElement>('button[aria-label="查看详情"]')!;
  expect(details.textContent).toBe("查看详情");expect(details.querySelector('[data-eye]')).not.toBeNull();
  await act(async()=>details.click());expect(open).toHaveBeenCalledTimes(1);
  await act(async()=>[...element.querySelectorAll('td')].find(cell=>cell.textContent==="Beta")!.click());
  expect(open).toHaveBeenLastCalledWith({id:"2",name:"Beta"});
});
it.each(["en-US", "zh-CN", "ja-JP"])("opens the permitted detail action from a row independently of labels in %s", async locale => {
  await i18n.changeLanguage(locale);
  const config: ListConfig<{id:string;name:string}> = {
    resource:"ai-agents", rowKey:row=>row.id,
    columns:[{key:"name",label:"Name"}], toolbar:{create:false},
    actions:[
      {label:"Edit",labelKey:"common-domain:common.edit",permission:"ai-agents.update",to:row=>({id:row.id,mode:"edit"})},
      {label:"View",labelKey:"common-domain:common.view",to:row=>({id:row.id,mode:"view"}),when:row=>row.id!=="2"},
    ],
  };
  vi.mocked(listResource).mockResolvedValueOnce({items:[{id:"1",name:"Alpha"},{id:"2",name:"Beta"}],total:2,page:1,page_size:20});
  await act(async()=>root.render(<ResourceTable config={config} workspace="ops" permissions={new Set()} />));
  await act(async()=>[...element.querySelectorAll('td')].find(cell=>cell.textContent==="Alpha")!.click());
  expect(navigate).toHaveBeenLastCalledWith(expect.objectContaining({id:"1",mode:"view"}),{});
  await act(async()=>[...element.querySelectorAll('td')].find(cell=>cell.textContent==="Beta")!.click());
  expect(navigate).toHaveBeenCalledTimes(1);
});
it("keeps the 15-second default selectable, spins only the refresh icon and resets after failure", async () => {
  vi.useFakeTimers();
  const visibility = vi.spyOn(document, "hidden", "get").mockReturnValue(false);
  localStorage.setItem("ferry.refresh.clock", "7");
  const config: ListConfig<{ id: string; name: string }> = {
    resource: "clock", defaultRefresh: 15, rowKey: row => row.id,
    columns: [{ key: "name", label: "Name" }], toolbar: { create: false },
  };
  try {
    await act(async () => root.render(<ResourceTable config={config} workspace="ops" permissions={new Set()} />));
    expect(element.querySelector('[role="timer"]')?.textContent).toBe("15s");
    await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
    expect(element.querySelector('[role="timer"]')?.textContent).toBe("14s");
    let fail!: (reason: Error) => void;
    vi.mocked(listResource).mockImplementationOnce(() => new Promise((_resolve, reject) => { fail = reject; }));
    await act(async () => { await vi.advanceTimersByTimeAsync(14000); });
    const countdown = element.querySelector('[role="timer"]');
    expect(countdown?.textContent).toBe("0s");
    expect(countdown?.querySelector('svg')).toBeNull();
    expect(element.querySelectorAll('svg.animate-spin')).toHaveLength(1);
    expect(listResource).toHaveBeenCalledTimes(2);
    await act(async () => { await vi.advanceTimersByTimeAsync(20000); });
    expect(listResource).toHaveBeenCalledTimes(2);
    await act(async () => fail(new Error("Refresh failed")));
    expect(element.querySelector('[role="timer"]')?.textContent).toBe("15s");
    await act(async () => [...element.querySelectorAll<HTMLButtonElement>('button')].find(button => button.textContent?.trim() === "刷新")!.click());
    expect(listResource).toHaveBeenCalledTimes(3);
    expect(element.querySelector('[role="timer"]')?.textContent).toBe("15s");
  } finally { localStorage.removeItem("ferry.refresh.clock"); visibility.mockRestore(); vi.useRealTimers(); }
});
it("shows refresh failures even when a remote page already contains rows, then recovers", async () => {
  const first = { items: [{ id: "1", name: "Repository" }], total: 0, page: 1, page_size: 50, has_more: true };
  vi.mocked(listResource).mockResolvedValueOnce(first);
  const config: ListConfig<{ id: string; name: string }> = {
    resource: "connection-repositories", pagination: "next", rowKey: row => row.id,
    columns: [{ key: "name", label: "仓库名称" }], toolbar: { create: false },
  };
  await act(async () => root.render(<ResourceTable config={config} workspace="ops" permissions={new Set()} />));
  const next = () => element.querySelector<HTMLButtonElement>('button[aria-label="下一页"]')!;
  expect(next().disabled).toBe(false);
  expect(element.textContent).not.toContain("共 0 条");
  const refresh = () => [...element.querySelectorAll<HTMLButtonElement>('button')].find(button => button.textContent?.trim() === "刷新")!;
  vi.mocked(listResource).mockRejectedValueOnce(new Error("Provider HTTP 403"));
  await act(async () => refresh().click());
  expect(element.querySelector('[role="alert"]')?.textContent).toBe("Provider HTTP 403");
  expect(next().disabled).toBe(true);
  vi.mocked(listResource).mockResolvedValueOnce({ ...first, has_more: false });
  await act(async () => refresh().click());
  expect(element.querySelector('[role="alert"]')).toBeNull();
  expect(next().disabled).toBe(true);
});
it("isolates pending actions to their own row/button and allows concurrent probes and navigation", async () => {
  const finish = new Map<string, () => void>();
  const action = vi.fn((row: { id: string }) => new Promise<void>(done => finish.set(row.id, done)));
  const config: ListConfig<{ id: string; name: string }> = {
    resource: "hosts", rowKey: row => row.id, columns: [{ key: "name", label: "名称" }],
    toolbar: { create: false, import: false, export: false },
    actions: [{ label: "测试连接", action }, { label: "查看", to: row => ({ id: row.id, mode: "view" }) }],
  };
  await act(async () => root.render(<ResourceTable config={config} workspace="ops" permissions={new Set()} />));
  const probes = () => [...element.querySelectorAll<HTMLButtonElement>('button[aria-label="测试连接"]')];
  await act(async () => probes()[0]!.click());
  expect(probes()[0]!.disabled).toBe(true);
  expect(probes()[0]!.getAttribute("aria-busy")).toBe("true");
  expect(probes()[1]!.disabled).toBe(false);
  expect(element.querySelectorAll('a[aria-label="查看"]')).toHaveLength(2);
  await act(async () => element.querySelector<HTMLElement>('a[aria-label="查看"]')!.click());
  expect(navigate).toHaveBeenCalledTimes(1);
  await act(async () => probes()[1]!.click());
  expect(action).toHaveBeenCalledTimes(2);
  await act(async () => finish.get("1")!());
  expect(probes()[0]!.disabled).toBe(false);
  expect(probes()[1]!.disabled).toBe(true);
  await act(async () => finish.get("2")!());
  expect(probes().every(button => !button.disabled)).toBe(true);
});
