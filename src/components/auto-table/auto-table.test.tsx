import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { AutoTable } from "./auto-table";

let root: Root, host: HTMLDivElement;
beforeEach(() => {
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
