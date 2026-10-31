import type { ComponentType, ReactNode } from "react";

import type { Evaluable } from "../auto-form/types";
import type { StickyConfig } from "./table-sticky";

export type { Evaluable } from "../auto-form/types";
export type { StickyConfig } from "./table-sticky";

interface ColumnConfigBase<T = unknown> {
  field: keyof T | string;
  name: Evaluable<ReactNode>;
  type?: "text" | "image" | "icon" | "qrcode" | "area" | "actions" | "custom";
  format?: (value: unknown, row: T) => ReactNode;
  color?: string | ((value: unknown, row: T) => string | undefined);
  sortable?: boolean;
  width?: string;
  align?: "left" | "center" | "right";
  onclick?: (row: T, event: React.MouseEvent) => false | void;
  render?: (
    value: unknown,
    row: T,
    ctx: { column: ColumnConfig<T> },
  ) => ReactNode;
}

export interface TableAction<T = unknown> {
  name: Evaluable<ReactNode>;
  icon?: ComponentType<{ className?: string }>;
  color?: string;
  onclick?: (row: T) => void;
  disabled?: (row: T) => boolean;
  show?: (row: T) => boolean;
}

export interface TableQuery {
  page: number;
  pageSize: number;
  sort?: { field: string; direction: "asc" | "desc" } | undefined;
  filter?: Record<string, unknown>;
}

interface TableOptionsBase<T = unknown> {
  data?: T[];
  fetch?: (query: TableQuery) => Promise<{ rows: T[]; total: number }>;
  columns: ColumnConfig<T>[];
  actions?: TableAction<T>[];
  rowKey?: keyof T | ((row: T) => string);
  selectable?: boolean;
  selection?: "single" | "multiple";
  pageSize?: number;
  pageSizeOptions?: number[];
  defaultSort?: { field: string; direction: "asc" | "desc" };
  /** Initial state restored from a shareable list URL. */
  initialQuery?: Partial<TableQuery>;
  filter?: Record<string, unknown>;
  title?: ReactNode;
  onRowClick?: (row: T) => void;
  onSelectionChange?: (rows: T[]) => void;
  onQueryChange?: (query: TableQuery) => void;
  className?: string;
  loading?: boolean;
  /** Ferry resource adapters own REST/URL state; file lists have no pagination. */
  state?: {
    rows: T[];
    total: number;
    query: TableQuery;
    loading?: boolean;
    error?: string | undefined;
    selected?: ReadonlySet<string>;
  };
  onPageChange?: (page: number) => void;
  onPageSizeChange?: (size: number) => void;
  onSortChange?: (field: string) => void;
  onToggleSelect?: (row: T) => void;
  onSelectAll?: () => void;
  onRetry?: () => void;
  pagination?: boolean | "next";
  hasMore?: boolean;
  empty?: ReactNode;
  actionsContent?: (row: T) => ReactNode;
  actionWidth?: number;
  footerContent?: ReactNode;
  groupBy?: (row: T) => string;
  rowClickable?: (row: T) => boolean;
  scrollRef?: React.Ref<HTMLDivElement>;
  minWidth?: number;
  /** Resource lists size columns to content; file lists reserve fixed metadata widths. */
  layout?: "auto" | "fixed";
  rowClassName?: (row: T) => string;
  /** Show a one-based row sequence that remains continuous across pages. */
  showSequence?: boolean;
  /** Columns hidden by the page-level column selector. */
  hiddenFields?: ReadonlySet<string>;
  /** Resolves server-relative image paths. */
  resolveAsset?: (path: string) => string;
  /** Sticky columns/header/pagination config. */
  sticky?: StickyConfig;
  /** Fill the parent height, keeping header/footer fixed and scrolling only the table viewport. */
  fill?: boolean;
}

export interface AutoTableHandle {
  refresh: () => void;
  exportCsv: (filename?: string) => void;
}

export interface UseAutoTableReturn<T> {
  rows: T[];
  total: number;
  loading: boolean;
  error: Error | null;
  query: TableQuery;
  selected: T[];
  setPage: (page: number) => void;
  setPageSize: (size: number) => void;
  setSort: (field: string) => void;
  setFilter: (filter: Record<string, unknown>) => void;
  toggleSelect: (row: T) => void;
  selectAll: (rows: T[]) => void;
  refresh: () => void;
}

export function getRowKey<T>(
  row: T,
  rowKey?: keyof T | ((row: T) => string),
): string {
  if (!rowKey) return (row as { id?: string })?.id ?? String(row);
  if (typeof rowKey === "function") return rowKey(row);
  return String(row[rowKey]);
}

export function formatCellValue<T>(row: T, column: ColumnConfig<T>): ReactNode {
  const raw = (row as Record<string, unknown>)[column.field as string];
  if (column.format) return column.format(raw, row);
  return raw as ReactNode;
}

type OptionalUndefined<T> = {
  [K in keyof T]: {} extends Pick<T, K> ? T[K] | undefined : T[K];
};
export type ColumnConfig<T = unknown> = OptionalUndefined<ColumnConfigBase<T>>;
export type TableOptions<T = unknown> = OptionalUndefined<TableOptionsBase<T>>;
