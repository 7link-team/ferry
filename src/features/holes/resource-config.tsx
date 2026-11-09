import { Ban, Check, Copy, ExternalLink, Eye, Pencil, Radar, Trash2, X } from "lucide-react";
import { toast } from "sonner";
import { api, errorText } from "../../shared/api/api";
import { base, deleteResource, named } from "../../shared/api/resource";
import { AutoField } from "../../shared/forms/auto-form";
import { commonText } from "../../shared/i18n/i18n";
import {
  ago,
  cronDate,
  form,
  label,
  list,
  namedOptions,
  type AnyRow,
  type ResourceConfigContext,
} from "../../shared/resources/config";
import type { FormConfig, ListConfig, Option, RowAction } from "../../shared/resources/types";
import type { Navigation } from "../../shared/routing/types";
import { Badge } from "../../shared/ui/badge";
import { Button } from "../../shared/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "../../shared/ui/select";

export interface Hole {
  id: string;
  host_id: string;
  host_name: string;
  host_online: boolean;
  name: string;
  target_port: number;
  strip_prefix: boolean;
  /** Root-relative preview path, e.g. "/holes/<token>/". */
  url?: string | undefined;
  state: "active" | "expired" | "revoked";
  expires_at: number;
  revoked_at: number;
  probe_state: string;
  probe_error: string;
  probed_at: number;
  active_connections: number;
  created_at: number;
  created_by: string;
  created_by_name: string;
  updated_at: number;
  updated_by: string;
  version: number;
  /** Who owns the lifetime: a person, or the pipeline run / workflow execution that opened it. */
  source_kind: "manual" | "pipeline" | "workflow";
  /** Pipeline run id or workflow execution id; empty for manual tunnels. */
  source_id: string;
  /** Job key or node id inside the source. */
  source_ref: string;
  review_state: "none" | "waiting" | "approved" | "rejected" | "timed_out";
  reviewed_at: number;
  reviewed_by: string;
  /** Write-only: present on create, and on edit only to extend from now. */
  ttl_hours?: number | undefined;
}

const key = (value: string) => `holes:${value}`;
const text = (value: string, values?: Record<string, unknown>) =>
  commonText(key(value), values);

export const HOLE_STATES: Option[] = ["active", "expired", "revoked"].map(
  (value) => ({ value, label: value, labelKey: key(`state.${value}`) }),
);
export const PROBE_STATES: Option[] = [
  "unknown",
  "reachable",
  "unreachable",
  "runner_offline",
  "unsupported",
].map((value) => ({ value, label: value, labelKey: key(`probe.${value}`) }));
export const REVIEW_STATES: Option[] = [
  "none",
  "waiting",
  "approved",
  "rejected",
  "timed_out",
].map((value) => ({ value, label: value, labelKey: key(`review.${value}`) }));
export const TTL_HOURS = [1, 8, 24, 168] as const;
export const DEFAULT_TTL_HOURS = 24;

/** The server returns a root-relative path; the browser knows the public origin. */
export const previewLink = (row: Pick<Hole, "url">) =>
  row.url ? new URL(row.url, window.location.origin).href : "";

async function copyLink(row: Hole) {
  try {
    await navigator.clipboard.writeText(previewLink(row));
    toast.success(commonText("common.copied"));
  } catch {
    toast.error(commonText("common.copyFailed"));
  }
}

const hostLabel = (row: Partial<Hole>) =>
  `${named(row.host_name, row.host_id)} · ${text(row.host_online ? "online" : "offline")}`;

function HostCell({ row }: { row: Hole }) {
  const status = text(row.host_online ? "online" : "offline");
  return (
    <span className="inline-flex min-w-0 items-center gap-1.5" title={status}>
      <span
        aria-hidden="true"
        className={`size-2 shrink-0 rounded-full ${row.host_online ? "bg-success" : "bg-muted-foreground/40"}`}
      />
      <span className="truncate">{named(row.host_name, row.host_id)}</span>
      <span className="sr-only">{status}</span>
    </span>
  );
}

function StateBadge({ state }: { state: string }) {
  return (
    <Badge
      variant={
        state === "active"
          ? "default"
          : state === "revoked"
            ? "destructive"
            : "secondary"
      }
    >
      {label(HOLE_STATES, state)}
    </Badge>
  );
}

const probeSummary = (row: Partial<Hole>) =>
  row.probed_at
    ? `${label(PROBE_STATES, row.probe_state ?? "unknown")} · ${ago(row.probed_at)}`
    : label(PROBE_STATES, row.probe_state ?? "unknown");

function Reachability({ row }: { row: Hole }) {
  return (
    <div className="min-w-0 space-y-0.5">
      <div>{probeSummary(row)}</div>
      {row.probe_state === "unreachable" && row.probe_error && (
        <div className="line-clamp-2 break-words text-xs text-muted-foreground" title={row.probe_error}>
          {row.probe_error}
        </div>
      )}
    </div>
  );
}

/** Rows without a source predate automatic previews and are manual. */
const manual = (row: Partial<Hole>) => (row.source_kind ?? "manual") === "manual";

/** Opens the run or execution that owns an automatic preview. */
function SourceCell({
  row,
  workspace,
  href,
  navigate,
}: {
  row: Hole;
  workspace: string;
  href: Navigation["href"] | undefined;
  navigate: Navigation["navigate"];
}) {
  if (manual(row)) return <span>{text("source.manual")}</span>;
  const caption = text(`source.${row.source_kind}`, { ref: row.source_ref });
  const className = "line-clamp-2 break-all text-left text-primary underline underline-offset-4";
  if (row.source_kind === "pipeline") {
    const route = { workspace, group: "workflow", item: "runs", id: row.source_id, mode: "view" as const };
    return (
      <a
        className={className}
        href={href?.(route, {}) ?? "#"}
        title={caption}
        onClick={(event) => {
          if (event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
          event.preventDefault();
          navigate(route, {});
        }}
      >
        {caption}
      </a>
    );
  }
  // An execution is shown inside its workflow, which the preview does not record.
  return (
    <button
      type="button"
      className={className}
      title={caption}
      onClick={async () => {
        try {
          const execution = await api<{ workflow_id: string }>(
            `/workspaces/${encodeURIComponent(workspace)}/workflow-executions/${encodeURIComponent(row.source_id)}`,
          );
          navigate(
            { workspace, group: "workflow", item: "workflows", id: execution.workflow_id, mode: "view" },
            { execution: row.source_id },
          );
        } catch (cause) {
          toast.error(errorText(cause));
        }
      }}
    >
      {caption}
    </button>
  );
}

function ReviewBadge({ state }: { state: string }) {
  if (state === "none") return <span className="text-muted-foreground">{label(REVIEW_STATES, state)}</span>;
  return (
    <Badge
      variant={
        state === "approved"
          ? "default"
          : state === "waiting"
            ? "outline"
            : state === "rejected"
              ? "destructive"
              : "secondary"
      }
    >
      {label(REVIEW_STATES, state)}
    </Badge>
  );
}

/** A waiting review is decided by whoever may run the owning source. */
const reviewActions = (workspace: string): RowAction<Hole>[] =>
  (["pipeline", "workflow"] as const).flatMap((source) =>
    (["approved", "rejected"] as const).map((decision) => ({
      label: decision === "approved" ? "通过" : "驳回",
      labelKey: key(decision === "approved" ? "approve" : "reject"),
      icon: decision === "approved" ? <Check className="size-4" /> : <X className="size-4" />,
      permission: source === "pipeline" ? "pipelines.run" : "workflows.run",
      destructive: decision === "rejected",
      when: (row: Hole) => row.review_state === "waiting" && row.source_kind === source,
      action: async (row: Hole) => {
        await api(itemPath(workspace, row, "reviews"), {
          method: "POST",
          body: JSON.stringify({ decision }),
        });
      },
    })),
  );

/** Required on create (suggested 24h); optional on edit, where a choice extends from now. */
function Validity({
  draft,
  readOnly,
  onPatch,
  creating,
}: {
  draft: Partial<Hole>;
  readOnly: boolean;
  onPatch: (patch: Partial<Hole>) => void;
  creating: boolean;
}) {
  const keep = "keep";
  return (
    <AutoField label={text("ttl")} required={creating} help={text("ttlHelp")}>
      <Select
        value={draft.ttl_hours ? String(draft.ttl_hours) : creating ? "" : keep}
        disabled={readOnly}
        onValueChange={(value) =>
          onPatch({ ttl_hours: value === keep ? undefined : Number(value) })
        }
      >
        <SelectTrigger className="w-fit min-w-[min(100%,10rem)] max-w-full" aria-label={text("ttl")}>
          <SelectValue placeholder={commonText("common.select")} />
        </SelectTrigger>
        <SelectContent>
          {!creating && <SelectItem value={keep}>{text("ttlKeep")}</SelectItem>}
          {TTL_HOURS.map((hours) => (
            <SelectItem key={hours} value={String(hours)}>
              {text(`ttl.${hours}`)}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </AutoField>
  );
}

function PreviewLink({ draft }: { draft: Partial<Hole> }) {
  const link = previewLink(draft);
  return (
    <AutoField label={text("url")}>
      <div className="flex min-w-0 items-start gap-2">
        <a
          className="min-w-0 flex-1 break-all text-sm text-primary underline underline-offset-4"
          href={link}
          target="_blank"
          rel="noopener noreferrer"
        >
          {link}
        </a>
        <Button
          type="button"
          variant="ghost"
          size="icon"
          className="shrink-0"
          aria-label={text("copyLink")}
          title={text("copyLink")}
          onClick={() => void copyLink(draft as Hole)}
        >
          <Copy className="size-4" aria-hidden="true" />
        </Button>
      </div>
    </AutoField>
  );
}

function UsageNotes({ draft }: { draft: Partial<Hole> }) {
  const path = draft.url ?? "/holes/<token>/";
  return (
    <ul className="list-disc space-y-1.5 pl-5 text-sm leading-6 text-muted-foreground">
      <li>{text("usage.sandbox")}</li>
      <li>{text("usage.strip")}</li>
      <li className="break-words">{text("usage.keep", { path })}</li>
      <li>{text("usage.connections")}</li>
    </ul>
  );
}

/** Status fields belong to a persisted record; `editValues` drops `url` so they show on the detail page only. */
const persisted = (row: Partial<Hole>) => !!row.url;
const runnerHosts = (workspace: string) =>
  namedOptions(workspace, "hosts", undefined, { connection: "runner" });
const itemPath = (workspace: string, row: Hole, operation: string) =>
  `${base(workspace, "holes")}/${encodeURIComponent(row.id)}/${operation}`;

export function holesLists({
  workspace,
  href,
  navigate,
}: ResourceConfigContext): Record<string, ListConfig<AnyRow>> {
  return {
    holes: list<Hole>({
      resource: "holes",
      defaultRefresh: 5,
      rowKey: (row) => row.id,
      filters: [
        { key: "q", label: "Search", labelKey: key("search"), type: "text" },
        {
          key: "host_id",
          label: "Host",
          labelKey: key("host"),
          type: "select",
          options: runnerHosts(workspace),
        },
        {
          key: "state",
          label: "State",
          labelKey: key("state"),
          type: "select",
          options: HOLE_STATES,
        },
      ],
      columns: [
        { key: "name", label: "Name", labelKey: key("name"), sortable: true },
        {
          key: "source_kind",
          label: "Source",
          labelKey: key("source"),
          render: (row) => <SourceCell row={row} workspace={workspace} href={href} navigate={navigate} />,
        },
        {
          key: "host_name",
          label: "Host",
          labelKey: key("host"),
          render: (row) => <HostCell row={row} />,
        },
        {
          key: "target_port",
          label: "Port",
          labelKey: key("port"),
          width: 100,
          align: "right",
          render: (row) => String(row.target_port),
        },
        {
          key: "state",
          label: "State",
          labelKey: key("state"),
          width: 110,
          render: (row) => <StateBadge state={row.state} />,
        },
        {
          key: "probe_state",
          label: "Reachability",
          labelKey: key("reachability"),
          render: (row) => <Reachability row={row} />,
        },
        {
          key: "active_connections",
          label: "Active connections",
          labelKey: key("activeConnections"),
          width: 120,
          align: "right",
          render: (row) => String(row.active_connections ?? 0),
        },
        {
          key: "expires_at",
          label: "Expires",
          labelKey: key("expiresAt"),
          width: 180,
          sortable: true,
          render: (row) => cronDate(row.expires_at),
        },
        {
          key: "review_state",
          label: "Review",
          labelKey: key("review"),
          width: 120,
          render: (row) => <ReviewBadge state={row.review_state ?? "none"} />,
        },
        {
          key: "created_by_name",
          label: "Created by",
          labelKey: key("createdBy"),
          render: (row) => named(row.created_by_name, row.created_by),
        },
      ],
      actions: [
        {
          label: "复制链接",
          labelKey: key("copyLink"),
          icon: <Copy className="size-4" />,
          action: copyLink,
        },
        {
          label: "打开预览",
          labelKey: key("open"),
          icon: <ExternalLink className="size-4" />,
          action: async (row) => {
            window.open(previewLink(row), "_blank", "noopener,noreferrer");
          },
        },
        {
          label: "立即检测",
          labelKey: key("probe"),
          icon: <Radar className="size-4" />,
          permission: "holes.update",
          when: (row) => row.state === "active",
          action: async (row) => {
            await api(itemPath(workspace, row, "probes"), { method: "POST" });
          },
        },
        {
          label: "撤销",
          labelKey: key("revoke"),
          icon: <Ban className="size-4" />,
          permission: "holes.update",
          destructive: true,
          when: (row) => row.state !== "revoked",
          action: async (row) => {
            await api(itemPath(workspace, row, "revocations"), { method: "POST" });
          },
        },
        ...reviewActions(workspace),
        {
          label: "查看",
          labelKey: "common-domain:common.view",
          icon: <Eye className="size-4" />,
          permission: "holes.read",
          to: (row) => ({ id: row.id, mode: "view" }),
        },
        {
          label: "编辑",
          labelKey: "common-domain:common.edit",
          icon: <Pencil className="size-4" />,
          permission: "holes.update",
          // Automatic previews live as long as their source; the server rejects edits (holes.automatic).
          when: (row) => row.state !== "revoked" && manual(row),
          to: (row) => ({ id: row.id, mode: "edit" }),
        },
        {
          label: "删除",
          labelKey: "common-domain:common.delete",
          icon: <Trash2 className="size-4" />,
          permission: "holes.delete",
          destructive: true,
          action: (row) => deleteResource(workspace, "holes", row.id),
        },
      ],
      toolbar: {
        create: { label: "New preview tunnel", labelKey: key("create") },
        import: false,
        export: false,
      },
      empty: "",
      emptyKey: key("empty"),
    }),
  };
}

export function holesForms({
  workspace,
}: ResourceConfigContext): Record<string, FormConfig<AnyRow>> {
  return {
    holes: form<Hole>({
      resource: "holes",
      heading: (row) => row.name,
      blank: {
        name: "",
        host_id: "",
        strip_prefix: true,
        ttl_hours: DEFAULT_TTL_HOURS,
      },
      /** The host detail tab opens creation with its host preset and locked. */
      queryDefaults: ["host_id"],
      /** Edit sends name/strip_prefix/version; ttl_hours only when the user extends. */
      editValues: (row) => ({
        name: row.name,
        strip_prefix: row.strip_prefix,
        version: row.version,
        ttl_hours: undefined,
        url: undefined,
      }),
      /** Revoked is terminal and automatic previews belong to their source: the server rejects both edits. */
      readOnly: (row) => row.state === "revoked" || !manual(row),
      pollMs: () => 5000,
      sections: [
        {
          title: "Tunnel",
          titleKey: key("section.tunnel"),
          fields: [
            {
              key: "name",
              label: "Name",
              labelKey: key("name"),
              type: "text",
              required: true,
            },
            {
              key: "host_id",
              label: "Host",
              labelKey: key("host"),
              type: "select",
              required: true,
              immutable: true,
              options: runnerHosts(workspace),
              display: hostLabel,
              helpKey: key("hostHelp"),
            },
            {
              key: "target_port",
              label: "Local port",
              labelKey: key("port"),
              type: "number",
              required: true,
              immutable: true,
              min: 1,
              max: 65535,
              helpKey: key("portHelp"),
            },
            {
              key: "ttl_hours",
              label: "Validity",
              labelKey: key("ttl"),
              type: "custom",
              required: true,
              when: (draft) => !draft.id,
              render: ({ draft, readOnly, onPatch }) => (
                <Validity draft={draft} readOnly={readOnly} onPatch={onPatch} creating />
              ),
            },
            {
              key: "ttl_hours",
              label: "Validity",
              labelKey: key("ttl"),
              type: "custom",
              when: (draft) => !!draft.id && !persisted(draft),
              render: ({ draft, readOnly, onPatch }) => (
                <Validity draft={draft} readOnly={readOnly} onPatch={onPatch} creating={false} />
              ),
            },
          ],
        },
        {
          title: "Status",
          titleKey: key("section.status"),
          fields: [
            {
              key: "url",
              label: "Preview link",
              labelKey: key("url"),
              type: "custom",
              span: 2,
              when: persisted,
              render: ({ draft }) => <PreviewLink draft={draft} />,
            },
            {
              key: "state",
              label: "State",
              labelKey: key("state"),
              type: "select",
              options: HOLE_STATES,
              when: persisted,
            },
            {
              key: "probe_state",
              label: "Reachability",
              labelKey: key("reachability"),
              type: "text",
              when: persisted,
              display: probeSummary,
            },
            {
              key: "probe_error",
              label: "Check error",
              labelKey: key("probeError"),
              type: "text",
              span: 2,
              when: (draft) => persisted(draft) && !!draft.probe_error,
            },
            {
              key: "active_connections",
              label: "Active connections",
              labelKey: key("activeConnections"),
              type: "text",
              when: persisted,
              display: (row) => String(row.active_connections ?? 0),
            },
            {
              key: "expires_at",
              label: "Expires",
              labelKey: key("expiresAt"),
              type: "text",
              when: persisted,
              display: (row) => cronDate(row.expires_at ?? 0),
            },
            {
              key: "revoked_at",
              label: "Revoked at",
              labelKey: key("revokedAt"),
              type: "text",
              when: (draft) => persisted(draft) && !!draft.revoked_at,
              display: (row) => cronDate(row.revoked_at ?? 0),
            },
            {
              key: "created_by_name",
              label: "Created by",
              labelKey: key("createdBy"),
              type: "text",
              when: persisted,
              display: (row) => named(row.created_by_name, row.created_by),
            },
            {
              key: "created_at",
              label: "Created",
              labelKey: key("createdAt"),
              type: "text",
              when: persisted,
              display: (row) => cronDate(row.created_at ?? 0),
            },
            {
              key: "updated_at",
              label: "Updated",
              labelKey: key("updatedAt"),
              type: "text",
              when: persisted,
              display: (row) => cronDate(row.updated_at ?? 0),
            },
          ],
        },
        {
          title: "Usage notes",
          titleKey: key("section.usage"),
          fields: [
            {
              key: "usage",
              label: "Usage notes",
              labelKey: key("section.usage"),
              type: "custom",
              span: 2,
              when: persisted,
              render: ({ draft }) => <UsageNotes draft={draft} />,
            },
          ],
        },
      ],
    }),
  };
}
