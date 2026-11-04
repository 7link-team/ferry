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
