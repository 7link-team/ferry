import { describe, expect, it } from "vitest";

import {
  autoTableViewportStyle,
  buildTableCsv,
  clampPageToTotal,
  getActionColumnWidth,
  getSequenceNumber,
  getPageSizeOptions,
  getTableBodyState,
  getVisibleColumns,
  resolveTableImage,
  syncExternalFilter,
} from "./auto-table-state";
import type { ColumnConfig } from "./types";

interface Row {
  id: string;
  name: string;
  note: string;
}

const columns: ColumnConfig<Row>[] = [
  { field: "id", name: "ID" },
  { field: "name", name: "Name" },
  { field: "note", name: "Note" },
];

describe("AutoTable compatibility state", () => {
  it("sizes the sticky action column from the resource action set", () => {
    expect(getActionColumnWidth(1)).toBe(64);
    expect(getActionColumnWidth(2)).toBe(88);
    expect(getActionColumnWidth(4)).toBe(144);
    expect(getActionColumnWidth(20)).toBe(240);
  });
  it("fills its parent in list-page mode instead of capping the table to viewport units", () => {
    expect(autoTableViewportStyle(true, "62vh")).toEqual({ height: "100%" });
    expect(autoTableViewportStyle(false, "62vh")).toEqual({
      maxHeight: "62vh",
    });
  });
  it("clamps local pagination when filtered data no longer fills the current page", () => {
    expect(clampPageToTotal(2, 20, 1)).toBe(1);
    expect(clampPageToTotal(3, 20, 45)).toBe(3);
    expect(clampPageToTotal(4, 20, 0)).toBe(1);
  });
  it("uses the same visible columns for rendering and export", () => {
    expect(getVisibleColumns(columns, new Set(["note"]))).toEqual(
      columns.slice(0, 2),
    );
  });

  it("numbers rows continuously across one-based pages", () => {
    expect(getSequenceNumber(3, 20, 0)).toBe(41);
    expect(getSequenceNumber(3, 20, 4)).toBe(45);
  });

  it("escapes quotes and preserves commas and line breaks in CSV", () => {
    const csv = buildTableCsv(columns, [
      { id: "1", name: "Rice, Noodles", note: 'He said "yes"\nnext' },
    ]);

    expect(csv).toBe(
      '"ID","Name","Note"\r\n"1","Rice, Noodles","He said ""yes""\nnext"',
    );
  });

  it("resolves server-relative image paths through the configured asset resolver", () => {
    expect(
      resolveTableImage("/uploads/a.png", (path) => `https://cdn.test${path}`),
    ).toBe("https://cdn.test/uploads/a.png");
    expect(resolveTableImage("https://images.test/a.png")).toBe(
      "https://images.test/a.png",
    );
  });

  it("keeps request errors distinct from loading and empty states", () => {
    expect(getTableBodyState(true, new Error("old"), [])).toBe("loading");
    expect(getTableBodyState(false, new Error("network"), [])).toBe("error");
    expect(getTableBodyState(false, null, [])).toBe("empty");
    expect(getTableBodyState(false, null, [{ id: "1" }])).toBe("ready");
  });

  it("preserves a configured page size outside the default selector choices", () => {
    expect(getPageSizeOptions(999, [10, 20, 50, 100])).toEqual([
      10, 20, 50, 100, 999,
    ]);
  });

  it("synchronizes changed external filters and resets pagination", () => {
    const query = { page: 4, pageSize: 20, filter: { status: "active" } };
    expect(syncExternalFilter(query, { status: "disabled" })).toEqual({
      page: 1,
      pageSize: 20,
      filter: { status: "disabled" },
    });
    expect(syncExternalFilter(query, { status: "active" })).toBe(query);
  });
});
