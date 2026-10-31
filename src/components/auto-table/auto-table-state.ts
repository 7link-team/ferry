import type { CSSProperties } from "react";

import type { ColumnConfig } from "./types";
import type { TableQuery } from "./types";

export function autoTableViewportStyle(
  fill: boolean | undefined,
  maxHeight: string | number | undefined,
): CSSProperties {
  return fill ? { height: "100%" } : { maxHeight };
}

export function getActionColumnWidth(actionCount: number): number {
  return Math.min(240, Math.max(64, 32 + Math.max(0, actionCount) * 28));
}

export function clampPageToTotal(
  page: number,
  pageSize: number,
  total: number,
): number {
  const lastPage = Math.max(1, Math.ceil(total / pageSize));
  return Math.min(Math.max(1, page), lastPage);
}

export function getPageSizeOptions(
  pageSize: number,
  pageSizeOptions: number[],
): number[] {
  return pageSizeOptions.includes(pageSize)
    ? pageSizeOptions
    : [...pageSizeOptions, pageSize];
}

export function syncExternalFilter(
  query: TableQuery,
  filter: Record<string, unknown>,
): TableQuery {
  if (JSON.stringify(query.filter ?? {}) === JSON.stringify(filter))
    return query;
  return { ...query, page: 1, filter };
}

export function getVisibleColumns<T>(
  columns: ColumnConfig<T>[],
  hiddenFields?: ReadonlySet<string>,
): ColumnConfig<T>[] {
  return columns.filter((column) => !hiddenFields?.has(String(column.field)));
}

export function getSequenceNumber(
  page: number,
  pageSize: number,
  index: number,
): number {
  return (page - 1) * pageSize + index + 1;
}

function csvValue(value: unknown): string {
  const text = value == null ? "" : String(value);
  return `"${text.replaceAll('"', '""')}"`;
}

export function buildTableCsv<T>(
  columns: ColumnConfig<T>[],
  rows: T[],
): string {
  const headers = columns.map((column) =>
    csvValue(typeof column.name === "function" ? column.name({}) : column.name),
  );
  const lines = rows.map((row) =>
    columns
      .map((column) => {
        const value = (row as Record<string, unknown>)[column.field as string];
        const formatted = column.format?.(value, row);
        return csvValue(
          typeof formatted === "string" || typeof formatted === "number"
            ? formatted
            : value,
        );
      })
      .join(","),
  );
  return [headers.join(","), ...lines].join("\r\n");
}

export function resolveTableImage(
  path: string,
  resolveAsset?: (path: string) => string,
): string {
  return resolveAsset ? resolveAsset(path) : path;
}

export function getTableBodyState<T>(
  loading: boolean,
  error: Error | null,
  rows: T[],
): "loading" | "error" | "empty" | "ready" {
  if (loading) return "loading";
  if (error) return "error";
  if (rows.length === 0) return "empty";
  return "ready";
}
