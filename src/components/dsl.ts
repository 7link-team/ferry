/**
 * The configuration contract for the two shared components. This file is the
 * authority on what a resource page can express; `ResourceTable` and
 * `ResourceForm` are its only consumers.
 *
 * Extending a resource's needs means adding a field here, never forking a
 * component (CLAUDE.md §12).
 */
import type { ReactNode } from "react";
import type { Route } from "../router";

export interface Option {
  value: string;
  label: string;
  group?: string;
  requires?: string[];
  disabled?: boolean;
}
export type Options = Option[] | (() => Promise<Option[]>);

export interface Filter {
  key: string;
  label: string;
  type: "text" | "select";
  options?: Options;
  placeholder?: string;
}

export interface Column<T> {
  key: string;
  label: string;
  /** Fixed px width. Leave one column unset so it absorbs the spare space. */
  width?: number;
  align?: "left" | "right";
  sortable?: boolean;
  render?: (row: T) => ReactNode;
}

export interface RowAction<T> {
  label: string;
  permission?: string;
  /** Navigating action: becomes a real link. */
  to?: (row: T) => Partial<Route>;
  query?: (row: T) => Record<string, string>;
  /** Executing action: destructive ones confirm first. */
  action?: (row: T) => Promise<void>;
  destructive?: boolean;
  when?: (row: T) => boolean;
  /** Keep unavailable actions discoverable and explain why. */
  disabled?: (row: T) => boolean;
  disabledReason?: (row: T) => string;
}

export interface BulkAction {
  label: string;
  permission?: string;
  destructive?: boolean;
  action: (ids: string[]) => Promise<void>;
}

export interface ListConfig<T> {
  permissionResource?: string;
  /** REST resource name; the URL is /api/workspaces/{ws}/{resource}. */
  resource: string;
  /** Explicit API path for global resources (without /api). */
  endpoint?: string;
  defaultPageSize?: number;
  rowKey: (row: T) => string;
  filters?: Filter[];
  columns: Column<T>[];
  actions?: RowAction<T>[];
  bulk?: BulkAction[];
  toolbar?: {
    create?: boolean | { label: string };
    import?: boolean;
    export?: boolean;
    refresh?: readonly number[];
  };
  /** Conditions applied on every request but not shown as filters — this is
   *  how one resource serves several menu entries (cron-jobs scope, …). */
  fixed?: Record<string, string>;
  empty?: string;
}

export type FieldType =
  | "repository-source"
  | "text"
  | "textarea"
  | "number"
  | "select"
  | "switch"
  | "password"
  | "secret"
  | "code"
  | "manifest"
  | "multiselect";

export interface FormField<T> {
  key: string;
  label: string;
  type: FieldType;
  required?: boolean;
  help?: string;
  placeholder?: string;
  options?: Options;
  /** Conditional display, e.g. address/port/user only for an SSH host. */
  when?: (draft: Partial<T>) => boolean;
  /** Settable on create, fixed afterwards. */
  immutable?: boolean;
  /** Clear dependent selections when this field changes. */
  resetOnChange?: string[];
  /** Metadata key for a stored secret's presence (never its value). */
  stored?: string;
  /** Explicitly opt into replacing an existing write-only value, including with empty text. */
  replaceOnEdit?: boolean;
  /** Optional audited plaintext action, only offered on the detail page. */
  reveal?: { permission: string; action: (row: Partial<T>) => Promise<string> };
  /** 2 makes the field span the whole two-column grid. */
  span?: 1 | 2;
}

export interface FormConfig<T> {
  permissionResource?: string;
  resource: string;
  /** Explicit API path for global resources (without /api). */
  endpoint?: string;
  sections: { title?: string; fields: FormField<T>[] }[];
  /** Heading for the detail and edit pages. */
  heading: (row: T) => string;
  /** Values a create form starts from. */
  blank: Partial<T>;
  readOnly?: (row: Partial<T>) => boolean;
  copy?: (row: T) => Partial<T>;
  viewActions?: RowAction<T>[];
  pollMs?: (row: Partial<T>) => number;
  generator?: {
    options: Options;
    load: (id: string) => Promise<{ name: string; version: number; fields: FormField<Record<string, unknown>>[]; values: Record<string, unknown> }>;
    generate: (id: string, version: number, values: Record<string, unknown>) => Promise<Partial<T>>;
  };
}
