import { ChevronDown,Download,Loader2,Upload,X } from "lucide-react";
import { cn } from "cn";
import { Dialog } from "radix-ui";
import { useRef,useState } from "react";
import { toast } from "sonner";
import { api,ApiError,errorText,localizedFetch } from "../../shared/api/api";
import { base,queryString } from "../../shared/api/resource";
import { AutoField } from "../../shared/forms/auto-form";
import { commonText,useCommonTranslation } from "../../shared/i18n/i18n";
import { AutoTable } from "../../shared/tables/auto-table";
import { Button } from "../../shared/ui/button";
import { DropdownMenu,DropdownMenuContent,DropdownMenuItem,DropdownMenuTrigger } from "../../shared/ui/dropdown-menu";
import { Input } from "../../shared/ui/input";

export const importResources = new Set(["hosts", "host-groups", "tags", "credentials", "repositories", "pipelines", "pipeline-groups", "pipeline-environments", "deployments", "variables", "cron-jobs", "alert-rules", "dictionaries", "roles", "marketplace-sources", "app-templates", "pipeline-templates", "config-templates"]);
export const exportResources = new Set([...importResources, "activity", "members", "settings", "marketplace", "alert-events", "cron-runs", "pipeline-runs", "pipeline-artifacts", "deployment-revisions"]);
interface Preview { id: string | null; valid: boolean; count: number; rows: { row: number; values: Record<string, unknown>; error: string | null }[] }

async function download(path: string, filename: string) {
  const response = await localizedFetch(`/api${path}`, { credentials: "same-origin", headers: { "X-Ferry-Request": "1" } });
  if (!response.ok) {
    const data = await response.json().catch(() => ({}));
    throw new ApiError(data.error ?? commonText("transfer.downloadFailed"), response.status);
  }
  const url = URL.createObjectURL(await response.blob());
  const link = document.createElement("a"); link.href = url; link.download = filename;
  document.body.append(link); link.click(); link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 30_000);
}

export function ResourceTransfer({ workspace, resource, params, columns, canImport, canExport, onChanged }: {
  workspace: string; resource: string; params: Record<string, string | number | undefined>;
  columns?: { key: string; label: string }[];
  canImport: boolean; canExport: boolean; onChanged: () => void;
}) {
  useCommonTranslation();
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [preview, setPreview] = useState<Preview>();
  const [file, setFile] = useState<File>();
  const [dragging, setDragging] = useState(false);
  const fileInput = useRef<HTMLInputElement>(null);
  const endpoint = base(workspace, resource);
  const run = async (action: () => Promise<void>) => {
    setBusy(true); setError("");
    try { await action(); } catch (e) { setError(errorText(e)); toast.error(errorText(e)); }
    finally { setBusy(false); }
  };
  const discard = async () => {
    if (preview?.id) await api(`${endpoint}/imports/${preview.id}`, { method: "DELETE" });
    setPreview(undefined);
  };
  const close = () => { if (!busy) void run(async () => { await discard(); setFile(undefined); setOpen(false); }); };
  const previewColumns = [...new Set(preview?.rows.flatMap(row => Object.keys(row.values)) ?? [])];
  const exportParams = { ...params, ...(columns?.length ? { columns: columns.map(column => column.key).join(','), headers: JSON.stringify(columns.map(column => column.label)) } : {}) };
  return <>
    {canImport && <Dialog.Root open={open} onOpenChange={value => value ? setOpen(true) : close()}>
      <Dialog.Trigger asChild><Button size="sm" variant="outline"><Upload /> {commonText("transfer.import")}</Button></Dialog.Trigger>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-50 bg-black/40" />
        <Dialog.Content className="fixed left-1/2 top-1/2 z-50 flex max-h-[90dvh] w-[min(64rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 flex-col gap-4 rounded-lg border bg-background p-6 shadow-lg">
          <Dialog.Title className="text-lg font-semibold">{commonText("transfer.importTitle")}</Dialog.Title>
          <Dialog.Description className="text-sm text-muted-foreground">{commonText("transfer.importHelp")}</Dialog.Description>
          <Dialog.Close asChild><Button size="icon" variant="ghost" disabled={busy} className="absolute right-3 top-3" aria-label={commonText("transfer.close")}><X /></Button></Dialog.Close>
          <div className="flex flex-wrap gap-2">
            {(["xlsx", "csv"] as const).map(format => <Button key={format} variant="outline" size="sm" disabled={busy} onClick={() => void run(() => download(`${endpoint}/import-template?format=${format}`, `${resource}-template.${format}`))}>{commonText("transfer.downloadTemplate", { format: format.toUpperCase() })}</Button>)}
          </div>
          {!preview && <AutoField label={commonText("transfer.chooseFile")} className="grid gap-2 text-sm">
            <div role="button" tabIndex={busy ? -1 : 0} aria-disabled={busy} aria-label={commonText("transfer.chooseFile")} className={cn("flex cursor-pointer flex-col items-center gap-2 rounded-lg border-2 border-dashed p-8 transition-colors", dragging ? "border-primary bg-primary/5" : "border-muted-foreground/30 hover:border-primary/50", file && "border-primary/50 bg-primary/5", busy && "pointer-events-none opacity-50")}
              onClick={() => !busy && fileInput.current?.click()} onKeyDown={event => { if (!busy && (event.key === "Enter" || event.key === " ")) { event.preventDefault(); fileInput.current?.click(); } }} onDragOver={event => { event.preventDefault(); if (!busy) setDragging(true); }} onDragLeave={() => setDragging(false)} onDrop={event => { event.preventDefault(); setDragging(false); if (!busy) { setFile(event.dataTransfer.files[0]); setError(""); } }}>
              <Upload aria-hidden="true" className="size-8 text-muted-foreground" />
              <p className={cn("break-all text-center text-sm", file ? "font-medium text-primary" : "text-muted-foreground")}>{file?.name ?? commonText("transfer.dropzone")}</p>
            </div>
            <Input ref={fileInput} type="file" className="sr-only" accept=".csv,.xlsx" disabled={busy} onChange={e => { setFile(e.target.files?.[0]); setError(""); }} />
          </AutoField>}
          {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
          {busy && <p role="status" className="flex items-center gap-2 text-sm"><Loader2 className="size-4 animate-spin" /> {commonText("transfer.processing")}</p>}
          {preview && <>
            <p role="status" className="text-sm">{preview.valid ? commonText("transfer.valid", { count: preview.count }) : commonText("transfer.invalid")}</p>
            <AutoTable pagination={false} fill rowKey={row => String(row.row)} minWidth={600} sticky={{ header: true, left: 1 }}
              state={{ rows: preview.rows, total: preview.rows.length, query: { page: 1, pageSize: Math.max(1, preview.rows.length) } }}
              columns={[{ field: "row", name: commonText("transfer.row"), width: "88px" }, { field: "error", name: commonText("transfer.validation"), width: "200px", render: (_value, row) => <span className={row.error ? "text-destructive" : ""}>{row.error ?? commonText("transfer.passed")}</span> }, ...previewColumns.map(key => ({ field: key, name: columns?.find(column => column.key === key)?.label ?? key, width: "220px", render: (_value: unknown, row: Preview["rows"][number]) => <span className="whitespace-pre-wrap break-words">{typeof row.values[key] === "string" ? String(row.values[key]) : JSON.stringify(row.values[key])}</span> }))]} />
          </>}
          <div className="flex flex-wrap justify-end gap-2">
            <Button variant="outline" disabled={busy} onClick={close}>{commonText("common.cancel")}</Button>
            {preview ? <Button variant="outline" disabled={busy} onClick={() => void run(discard)}>{commonText("transfer.chooseAgain")}</Button> : <Button disabled={busy || !file} onClick={() => void run(async () => {
              if (!file) return;
              if (file.size > 8 * 1024 * 1024) throw new Error(commonText("transfer.tooLarge"));
              if (!/\.(csv|xlsx)$/i.test(file.name)) throw new Error(commonText("transfer.invalidFormat"));
              const form = new FormData(); form.append("file", file);
              setPreview(await api<Preview>(`${endpoint}/imports`, { method: "POST", body: form }));
            })}>{commonText("transfer.preview")}</Button>}
            {preview?.valid && <Button disabled={busy} onClick={() => void run(async () => {
              const result = await api<{items: unknown[]}>(`${endpoint}/imports/${preview.id}/commits`, { method: "POST" });
              setPreview(undefined); setFile(undefined); setOpen(false); onChanged(); toast.success(commonText("transfer.created", { count: result.items.length }));
            })}>{commonText("transfer.confirm", { count: preview.count })}</Button>}
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>}
    {canExport && <DropdownMenu><DropdownMenuTrigger asChild><Button size="sm" variant="outline" disabled={busy}>{busy && !open ? <Loader2 className="animate-spin" /> : <Download />} {commonText("transfer.export")}<ChevronDown className="size-4" /></Button></DropdownMenuTrigger>
      <DropdownMenuContent align="end">{(["xlsx", "csv"] as const).map(format => <DropdownMenuItem key={format} onSelect={() => void run(() => download(`${endpoint}/exports${queryString({ ...exportParams, format })}`, `${resource}.${format}`))}>{commonText("transfer.exportFormat", { format: format.toUpperCase() })}</DropdownMenuItem>)}</DropdownMenuContent>
    </DropdownMenu>}
  </>;
}
