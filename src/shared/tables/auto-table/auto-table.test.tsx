import { act } from "react";
import { createRoot,type Root } from "react-dom/client";
import { afterEach,beforeEach,expect,it,vi } from "vitest";
import { i18n } from "../../i18n/i18n";
import { AutoTable } from "./auto-table";

let root: Root, host: HTMLDivElement;
beforeEach(() => {
  void i18n.changeLanguage("zh-CN");
  localStorage.setItem("ui.locale", "zh-CN");
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
});
const rows = [
  { id: "1", name: "Alpha" },
  { id: "2", name: "Beta" },
];
it("measures current action controls after equal-count row replacement and resizing", async () => {
  let actionHeight = 44;
  const callbacks: (() => void)[] = [];
  vi.stubGlobal("ResizeObserver", class {
    active = true;
    constructor(callback: ResizeObserverCallback) {
      callbacks.push(() => { if (this.active) callback([], this as unknown as ResizeObserver); });
    }
    observe() {}
    disconnect() { this.active = false; }
  });
  const bounds = vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
    const height = this.tagName === "THEAD" ? 60 : this.hasAttribute("data-measured-action") && this.isConnected ? actionHeight : 0;
    return { x: 0, y: 0, width: 100, height, top: 0, left: 0, right: 100, bottom: height, toJSON: () => ({}) };
  });
  try {
    const render = (items: typeof rows) => act(async () => root.render(<AutoTable
      columns={[{field:"name",name:"Name"}]} rowKey={row => row.id}
      state={{rows:items,total:items.length,query:{page:1,pageSize:20,filter:{}}}}
      minDataHeight="row" actionWidth={80}
      actionsContent={row => <button data-measured-action>{row.id}</button>}
    />));
    await render(rows.slice(0, 1));
    expect(host.querySelector("table")!.parentElement!.style.minHeight).toBe("104px");
    await render(rows.slice(1));
    actionHeight = 88;
    await act(async () => callbacks.forEach(callback => callback()));
    expect(host.querySelector("table")!.parentElement!.style.minHeight).toBe("148px");
  } finally {
    bounds.mockRestore();
    vi.unstubAllGlobals();
  }
});
it("illustrates empty data and filtered results while preserving custom messages and errors", async () => {
  const render = async (filter: Record<string, unknown>, extra: Record<string, unknown> = {}) => act(async () => root.render(<AutoTable columns={[{field:"name",name:"Name"}]} state={{rows:[],total:0,query:{page:1,pageSize:20,filter}}} {...extra} />));
  await render({});
  expect(host.textContent).toContain("暂无记录");
  expect(host.querySelector('[role="status"] svg[aria-hidden="true"]')).not.toBeNull();
  await render({name:"missing"});
  expect(host.textContent).toContain("没有符合当前筛选条件的记录");
  await render({}, {empty:"No complete commit facts"});
  expect(host.textContent).toContain("No complete commit facts");
  await act(async () => root.render(<AutoTable columns={[{field:"name",name:"Name"}]} state={{rows:[],total:0,query:{page:1,pageSize:20},error:"offline"}} />));
  expect(host.querySelector('[role="alert"]')?.textContent).toContain("offline");
  expect(host.querySelector('[role="status"]')).toBeNull();
  // A load error is the shared alert with a Retry action wired to the table's retry.
  const onRetry = vi.fn();
  await act(async () => root.render(<AutoTable columns={[{field:"name",name:"Name"}]} onRetry={onRetry} state={{rows:[],total:0,query:{page:1,pageSize:20},error:"加载失败"}} />));
  const alert = host.querySelector('[role="alert"]')!;
  expect(alert.className).toContain("border-destructive/40");
  await act(async () => [...alert.querySelectorAll("button")].find(button => button.textContent === "重试")!.click());
  expect(onRetry).toHaveBeenCalledOnce();
});
it("opens rows by mouse and keyboard while giving embedded controls and columns priority", async () => {
  const open = vi.fn(),
    action = vi.fn(),
    column = vi.fn();
  await act(async () =>
    root.render(
      <AutoTable
        pagination={false}
        state={{ rows, total: 2, query: { page: 1, pageSize: 20 } }}
        onRowClick={open}
        columns={[
          { field: "name", name: "名称" },
          {
            field: "control",
            name: "列操作",
            onclick: column,
            render: () => "Column",
          },
        ]}
        actionsContent={(row) => (
          <button onClick={() => action(row)}>操作</button>
        )}
      />,
    ),
  );
  await act(async () =>
    host.querySelector<HTMLTableCellElement>("tbody td")!.click(),
  );
  expect(open).toHaveBeenCalledTimes(1);
  await act(async () =>
    host.querySelector<HTMLButtonElement>("tbody button")!.click(),
  );
  expect(action).toHaveBeenCalledTimes(1);
  expect(open).toHaveBeenCalledTimes(1);
  await act(async () =>
    host
      .querySelector<HTMLTableRowElement>("tbody tr")!
      .dispatchEvent(
        new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
      ),
  );
  expect(open).toHaveBeenCalledTimes(2);
  await act(async () =>
    host.querySelectorAll<HTMLTableCellElement>("tbody td")[1]!.click(),
  );
  expect(column).toHaveBeenCalledTimes(1);
  expect(open).toHaveBeenCalledTimes(2);
});
it("retains explicit remote pagination without slicing data or inventing a total", async () => {
  const next = vi.fn();
  await act(async () =>
    root.render(
      <AutoTable
        pagination="next"
        hasMore
        state={{ rows, total: 0, query: { page: 4, pageSize: 50 } }}
        onPageChange={next}
        columns={[{ field: "name", name: "名称" }]}
      />,
    ),
  );
  expect(host.querySelectorAll("tbody tr")).toHaveLength(2);
  expect(host.textContent).not.toContain("共 0 条");
  await act(async () =>
    host.querySelector<HTMLButtonElement>('[aria-label="下一页"]')!.click(),
  );
  expect(next).toHaveBeenCalledWith(5);
});

it.each([false, true])("places selection before every column, with sequence=%s", async (showSequence) => {
  const open = vi.fn();
  await act(async () => root.render(<AutoTable selectable showSequence={showSequence} rowKey={row => row.id}
    state={{ rows, total: 2, query: { page: 1, pageSize: 20 } }}
    sticky={{ header: true, left: 1 }} onRowClick={open}
    columns={[{ field: "name", name: "Name" }]} />));
  expect(host.querySelector('thead th:first-child [role="checkbox"]')).not.toBeNull();
  expect(host.querySelector('tbody td:first-child [role="checkbox"]')).not.toBeNull();
  expect(host.querySelector<HTMLTableCellElement>('tbody td:first-child')?.style.left).toBe("0px");
  if (showSequence) expect(host.querySelector('tbody td:nth-child(2)')?.textContent).toBe("1");
  await act(async () => host.querySelector<HTMLButtonElement>('tbody td:first-child [role="checkbox"]')!.click());
  expect(open).not.toHaveBeenCalled();
});
