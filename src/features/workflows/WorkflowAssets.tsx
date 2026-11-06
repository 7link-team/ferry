import { FieldCaption } from "../../shared/forms/FieldHelp";
import { createContext, useContext, useEffect, useRef, useState } from "react";
import { api, errorText } from "../../shared/api/api";
import { workflowText as t } from "../../shared/i18n/i18n";
import { Input } from "../../shared/ui/input";

export interface WorkflowAsset {
  asset: string;
  mime: string;
  name: string;
  size: number;
}
/** Files in a new editor stay local until its first save, scoped to that form. */
export function createWorkflowDraftAssets() {
  const files = new Map<string, { file: File; url: string }>();
  return {
    files,
    async stage(file: File): Promise<WorkflowAsset> {
      const digest = await crypto.subtle.digest("SHA-256", await file.arrayBuffer());
      const asset = `sha256:${Array.from(new Uint8Array(digest), byte => byte.toString(16).padStart(2, "0")).join("")}`;
      if (!files.has(asset)) files.set(asset, { file, url: URL.createObjectURL(file) });
      return { asset, mime: file.type || "application/octet-stream", name: file.name, size: file.size };
    },
    clear() { for (const item of files.values()) URL.revokeObjectURL(item.url); files.clear(); },
  };
}
export const WorkflowDraftAssetsContext = createContext<ReturnType<typeof createWorkflowDraftAssets> | null>(null);

export async function uploadWorkflowDraftAssets(workspace: string, workflow: string, definition: string, draft: ReturnType<typeof createWorkflowDraftAssets>) {
  const assets = new Map<string, WorkflowAsset>();
  for (const [hash, { file }] of draft.files) {
    if (!definition.includes(hash)) continue;
    const body = new FormData(); body.append("file", file);
    assets.set(hash, await api<WorkflowAsset>(`/workspaces/${workspace}/workflows/${workflow}/assets`, { method: "POST", body }));
  }
  // Deduplicated server assets may have a canonical MIME type from a previous upload.
  return JSON.stringify(JSON.parse(definition), (_key, value: unknown) => {
    if (value && typeof value === "object" && "asset" in value && typeof value.asset === "string") {
      const uploaded = assets.get(value.asset);
      if (uploaded) return { ...value, mime: uploaded.mime, size: uploaded.size };
    }
    return value;
  });
}
export const WorkflowAssetContext = createContext({
  workspace: "",
  workflow: "",
});
export function WorkflowAssetPreview({ file }: { file: WorkflowAsset }) {
  const context = useContext(WorkflowAssetContext);
  const draft = useContext(WorkflowDraftAssetsContext);
  const hash = file.asset?.replace(/^sha256:/, "");
  const local = draft?.files.get(file.asset)?.url;
  if (!/^[a-f0-9]{64}$/.test(hash) || /^0+$/.test(hash) || (!context.workflow && !local))
    return null;
  const url = local ?? `/api/workspaces/${context.workspace}/workflows/${context.workflow}/assets/${hash}`;
  return (
    <div className="min-w-0 space-y-2 rounded-lg border p-3">
      {file.mime.startsWith("image/") && (
        <img
          src={url}
          alt={file.name}
          className="max-h-64 w-full rounded object-contain"
        />
      )}
      {file.mime.startsWith("audio/") && (
        <audio src={url} controls preload="metadata" className="w-full" />
      )}
      {file.mime.startsWith("video/") && (
        <video
          src={url}
          controls
          preload="metadata"
          className="max-h-80 w-full rounded"
        />
      )}
      <a
        href={url}
        download={file.name}
        className="block break-all text-sm text-primary underline"
      >
        {file.name || t("downloadFile")}
      </a>
      <span className="text-xs text-muted-foreground">
        {file.mime} · {Math.ceil(file.size / 1024)} KB
      </span>
    </div>
  );
}
export function WorkflowAssetUpload({
  file,
  kind,
  readOnly,
  change,
  busyChange,
}: {
  file: WorkflowAsset;
  kind: string;
  readOnly: boolean;
  change: (file: WorkflowAsset) => void;
  busyChange?: (busy: boolean) => void;
}) {
  const context = useContext(WorkflowAssetContext);
  const draft = useContext(WorkflowDraftAssetsContext);
  const [busy, setBusy] = useState(false),
    [error, setError] = useState("");
  const alive = useRef(true), report = useRef(busyChange);
  report.current = busyChange;
  useEffect(() => { alive.current = true; return () => { alive.current = false; report.current?.(false); }; }, []);
  return (
    <div className="space-y-3">
      <WorkflowAssetPreview file={file} />
      {!readOnly && (
        <label className="block space-y-2 text-sm">
          <span><FieldCaption label={<>{t("uploadFile")}</>} /></span>
          <Input
            type="file"
            aria-label={t("uploadFile")}
            accept={kind === "file" ? undefined : `${kind}/*`}
            disabled={busy || (!context.workflow && !draft)}
            onChange={async (event) => {
              const selected = event.target.files?.[0];
              event.target.value = "";
              if (!selected) return;
              if (selected.size > 32 * 1024 * 1024) {
                setError(t("fileTooLarge"));
                return;
              }
              setBusy(true);
              report.current?.(true);
              setError("");
              try {
                if (!context.workflow && draft) {
                  const file = await draft.stage(selected);
                  if (alive.current) change(file);
                  return;
                }
                const body = new FormData();
                body.append("file", selected);
                const file = await api<WorkflowAsset>(
                    `/workspaces/${context.workspace}/workflows/${context.workflow}/assets`,
                    { method: "POST", body },
                  );
                if (alive.current) change(file);
              } catch (error) {
                if (alive.current) setError(errorText(error));
              } finally {
                if (alive.current) setBusy(false);
                report.current?.(false);
              }
            }}
          />
          <span className="block text-xs text-muted-foreground">
            {busy
              ? t("uploading")
              : !context.workflow && draft
                ? t("localUploadHelp")
              : context.workflow
                ? t("uploadLimit")
                : t("saveBeforeUpload")}
          </span>
        </label>
      )}
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
    </div>
  );
}

interface Output {
  state?: string;
  end_reason?: string;
  ports: {
    name: string;
    index: number;
    items: {
      json: Record<string, unknown>;
      binary?: Record<string, WorkflowAsset>;
    }[];
  }[];
}
export function WorkflowOutputPreview({ output }: { output: Output }) {
  return (
    <div className="space-y-4">
      {output.state && <p role="status" className={output.state === "failed" ? "text-sm text-destructive" : "text-sm text-muted-foreground"}>{t("state")}: {output.state}{output.end_reason ? ` · ${output.end_reason}` : ""}</p>}
      {output.ports.map((port) => (
        <section key={`${port.name}:${port.index}`} className="space-y-3">
          <h3 className="font-medium">
            {port.name === "main" ? t("mainPort") : port.name} · {port.items.length} {t("inspector.items")}
          </h3>
          {port.name === "main" && <p className="text-xs text-muted-foreground">{t("defaultOutputHelp")}</p>}
          {port.items.slice(0, 20).map((item, index) => (
            <div key={index} className="space-y-3 rounded-lg border p-3">
              <dl className="space-y-2">
                {!Object.keys(item.json).length && <p className="text-xs text-muted-foreground"><code>{"{}"}</code> · {t("emptyObject")}</p>}
                {Object.entries(item.json).map(([key, value]) => (
                  <div key={key} className="grid min-w-0 gap-1">
                    <dt className="text-xs text-muted-foreground">{key}</dt>
                    <dd className="whitespace-pre-wrap break-words text-sm">
                      {typeof value === "string"
                        ? value
                        : JSON.stringify(value, null, 2)}
                    </dd>
                  </div>
                ))}
              </dl>
              {Object.entries(item.binary ?? {}).map(([field, file]) => (
                <div key={field}>
                  <p className="mb-1 text-xs text-muted-foreground">{field}</p>
                  <WorkflowAssetPreview file={file} />
                </div>
              ))}
            </div>
          ))}
          {port.items.length > 20 && (
            <p className="text-xs text-muted-foreground">{t("previewLimit")}</p>
          )}
        </section>
      ))}
    </div>
  );
}
