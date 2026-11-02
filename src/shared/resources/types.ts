/**
 * The configuration contract for the two shared components. This file is the
 * authority on what a resource page can express; `ResourceTable` and
 * `ResourceForm` are its only consumers.
 *
 * Extending a resource's needs means adding a field here, never forking a
 * component (CLAUDE.md §12).
 */
import type { ReactNode } from "react";
import type { Envelope } from "../api/resource";
import type { Route } from "../routing/types";

export interface Option {
  value: string;
  label: string;
  /** Explicit message key; raw labels remain stable action/configuration identities. */
  labelKey?: string | undefined;
  group?: string;
  groupKey?: string;
  requires?: string[];
  disabled?: boolean;
}
export type Options = Option[] | (() => Promise<Option[]>);

export interface Filter {
  key: string;
  label: string;
  /** Explicit message key; raw labels remain stable action/configuration identities. */
  labelKey?: string | undefined;
  type: "text" | "select";
  options?: Options;
  placeholder?: string;
  placeholderKey?: string;
}

export interface Column<T> {
  key: string;
  label: string;
  /** Explicit message key; raw labels remain stable action/configuration identities. */
  labelKey?: string | undefined;
  /** Fixed px width. Leave one column unset so it absorbs the spare space. */
  width?: number;
  align?: "left" | "right";
  sortable?: boolean;
  render?: (row: T) => ReactNode;
}

export interface RowAction<T> {
  label: string;
  /** Explicit message key; raw labels remain stable action/configuration identities. */
  labelKey?: string | undefined;
  icon?: ReactNode;
  newTab?: boolean;
  permission?: string;
  /** Navigating action: becomes a real link. */
  to?: (row: T) => Partial<Route>;
  query?: (row: T) => Record<string, string>;
  /** Executing action: destructive ones confirm first. */
  action?: (row: T, values?: Record<string, unknown>) => Promise<void>;
  /** Shared action form, for operations needing explicit input before execution. */
  parameters?: (row: T) => FormField<Record<string, unknown>>[];
  initialValues?: (row: T) => Record<string, unknown>;
  description?: string;
  descriptionKey?: string;
  destructive?: boolean;
  when?: (row: T) => boolean;
  /** Keep unavailable actions discoverable and explain why. */
  disabled?: (row: T) => boolean;
  disabledReason?: (row: T) => string;
}

export interface BulkAction {
  label: string;
  /** Explicit message key; raw labels remain stable action/configuration identities. */
  labelKey?: string | undefined;
  permission?: string;
  destructive?: boolean;
  action: (ids: string[]) => Promise<void>;
}

export interface ListConfig<T> {
  /** Isolate URL state when several lists share a detail page. */
  queryPrefix?: string;
  filterPresets?: { label: string; labelKey?: string | undefined; presets: { label: string; labelKey?: string | undefined; values: Record<string, string> | (() => Record<string, string>) }[] }[];
  /** A report summary shares the table's committed filters, loading and refresh. */
  summary?: (data: Envelope<T>) => ReactNode;
  permissionResource?: string;
  /** REST resource name; the URL is /api/workspaces/{ws}/{resource}. */
  resource: string;
  /** Explicit API path for global resources (without /api). */
  endpoint?: string;
  defaultPageSize?: number;
  /** Remote providers expose a next page without a reliable total count. */
  pagination?: "next";
  defaultRefresh?: number;
  rowKey: (row: T) => string;
  /** Optional namespace directory browser. An empty key places a row at the root. */
  grouping?: { label: string; labelKey?: string | undefined; key: (row: T) => string };
  filters?: Filter[];
  columns: Column<T>[];
  actions?: RowAction<T>[];
  bulk?: BulkAction[];
  toolbar?: {
    content?: { permission?: string; render: (reload: () => void) => ReactNode };
    create?: boolean | { label: string; labelKey?: string | undefined };
    import?: boolean;
    export?: boolean;
    refresh?: readonly number[];
  };
  /** Conditions applied on every request but not shown as filters — this is
   *  how one resource serves several menu entries (cron-jobs scope, …). */
  fixed?: Record<string, string>;
  empty?: string;
  emptyKey?: string;
}

export type FieldType =
  | "connection-repositories"
  | "author-aliases"
  | "repository-source"
  | "marketplace-repository"
  | "actions-editor"
  | "cron"
  | "text"
  | "textarea"
  | "number"
  | "select"
  | "switch"
  | "password"
  | "secret"
  | "code"
  | "manifest"
  | "multiselect"
  | "custom"
  /** Read-only list of records (e.g. commits) described by `record`. */
  | "records";

export interface FormField<T> {
  key: string;
  label: string;
  /** Explicit message key; raw labels remain stable action/configuration identities. */
  labelKey?: string | undefined;
  type: FieldType;
  required?: boolean;
  /** Inclusive lower bound for a numeric input. */
  min?: number;
  max?: number;
  disabled?: (draft: Partial<T>) => boolean;
  render?: (props: { workspace: string; draft: Partial<T>; readOnly: boolean; onPatch: (patch: Partial<T>) => void; onValidityChange: (blocked: boolean) => void }) => ReactNode;
  createOption?: { label: string; labelKey?: string | undefined; permission: string; fields: FormField<Record<string, unknown>>[]; create: (values: Record<string, unknown>) => Promise<Option> };
  help?: string;
  helpKey?: string;
  placeholder?: string;
  placeholderKey?: string;
  options?: Options;
  /** Conditional display, e.g. address/port/user only for an SSH host. */
  /** Legacy values meaning no selection for this field; never applied to numeric metrics. */
  emptyValues?: readonly (string | number)[];
  emptyLabel?: string;
  emptyLabelKey?: string;
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
  /** Read-only text shown instead of the raw value, e.g. a formatted time. */
  display?: (row: Partial<T>) => string;
  /** Render an http(s) value as an external link on detail pages. */
  link?: boolean;
  /** How each item of a `records` field is shown. */
  record?: {
    title: (item: Record<string, unknown>) => string;
    meta?: (item: Record<string, unknown>) => string;
    body?: (item: Record<string, unknown>) => string;
    href?: (item: Record<string, unknown>) => string;
  };
}

export interface FormConfig<T> {
  width?: "full";
  permissionResource?: string;
  resource: string;
  /** Explicit API path for global resources (without /api). */
  endpoint?: string;
  /** `group` places a section under a side menu entry of that name; sections of
   *  one group that set `tab` are shown as tabs. One save covers every group. */
  sections: { title?: string; titleKey?: string; fullWidth?: boolean; group?: string; groupKey?: string; tab?: string; tabKey?: string; fields: FormField<T>[] }[];
  /** Heading for the detail and edit pages. */
  heading: (row: T) => string;
  /** Values a create form starts from. */
  blank: Partial<T>;
  readOnly?: (row: Partial<T>) => boolean;
  copy?: (row: T) => Partial<T>;
  viewActions?: (RowAction<T> & { section?: string; iconOnly?: boolean })[];
  pollMs?: (row: Partial<T>) => number;
  generator?: {
    options: Options;
    load: (id: string) => Promise<{ name: string; version: number; fields: FormField<Record<string, unknown>>[]; values: Record<string, unknown> }>;
    generate: (id: string, version: number, values: Record<string, unknown>) => Promise<Partial<T>>;
  };
}
