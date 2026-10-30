import { useState } from "react";
import { Dialog } from "radix-ui";
import { Download, Loader2, Upload, X } from "lucide-react";
import { toast } from "sonner";
import { api, ApiError, errorText } from "../api";
import { base, queryString } from "../resource";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "./ui/dropdown-menu";

export const importResources = new Set(["hosts", "host-groups", "tags", "credentials", "repositories", "pipelines", "pipeline-groups", "pipeline-environments", "deployments", "variables", "cron-jobs", "alert-rules", "dictionaries", "roles", "marketplace-sources", "app-templates", "pipeline-templates", "config-templates"]);
export const exportResources = new Set([...importResources, "activity", "members", "settings", "marketplace", "alert-events", "cron-runs", "pipeline-runs", "pipeline-artifacts", "deployment-revisions"]);
interface Preview { id: string | null; valid: boolean; count: number; rows: { row: number; values: Record<string, unknown>; error: string | null }[] }

async function download(path: string, filename: string) {
  const response = await fetch(`/api${path}`, { credentials: "same-origin", headers: { "X-Ferry-Request": "1" } });
  if (!response.ok) {
    const data = await response.json().catch(() => ({}));
    throw new ApiError(data.error ?? "下载失败，请重试。", response.status);
  }
  const url = URL.createObjectURL(await response.blob());
  const link = document.createElement("a"); link.href = url; link.download = filename;
  document.body.append(link); link.click(); link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 30_000);
}

export function ResourceTransfer({ workspace, resource, params, canImport, canExport, onChanged }: {
  workspace: string; resource: string; params: Record<string, string | number | undefined>;
  canImport: boolean; canExport: boolean; onChanged: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [preview, setPreview] = useState<Preview>();
  const [file, setFile] = useState<File>();
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
  const columns = [...new Set(preview?.rows.flatMap(row => Object.keys(row.values)) ?? [])];
  return <>
    {canImport && <Dialog.Root open={open} onOpenChange={value => value ? setOpen(true) : close()}>
      <Dialog.Trigger asChild><Button size="sm" variant="outline"><Upload /> 导入</Button></Dialog.Trigger>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-50 bg-black/40" />
        <Dialog.Content className="fixed left-1/2 top-1/2 z-50 flex max-h-[90dvh] w-[min(64rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 flex-col gap-4 rounded-lg border bg-background p-6 shadow-lg">
          <Dialog.Title className="text-lg font-semibold">导入数据</Dialog.Title>
          <Dialog.Description className="text-sm text-muted-foreground">下载模板填写后预览，确认才新增记录。支持 CSV／XLSX，每次最多 1000 行、8MB；引用填写当前空间的 ID，密钥列不会回显。</Dialog.Description>
          <Dialog.Close asChild><Button size="icon" variant="ghost" disabled={busy} className="absolute right-3 top-3" aria-label="关闭导入"><X /></Button></Dialog.Close>
          <div className="flex flex-wrap gap-2">
            {(["xlsx", "csv"] as const).map(format => <Button key={format} variant="outline" size="sm" disabled={busy} onClick={() => void run(() => download(`${endpoint}/import-template?format=${format}`, `${resource}-template.${format}`))}>下载 {format.toUpperCase()} 模板</Button>)}
          </div>
          <label className="grid gap-2 text-sm">选择导入文件<Input type="file" accept=".csv,.xlsx" disabled={busy || !!preview} onChange={e => { setFile(e.target.files?.[0]); setError(""); }} /></label>
          {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
          {busy && <p role="status" className="flex items-center gap-2 text-sm"><Loader2 className="size-4 animate-spin" /> 正在处理，请稍候…</p>}
          {preview && <>
            <p role="status" className="text-sm">{preview.valid ? `校验通过，将新增 ${preview.count} 项。` : "校验未通过，未保存任何记录。请修正下列错误后重新上传。"}</p>
            <div className="min-h-0 overflow-auto rounded border">
              <table className="w-full text-left text-sm"><thead className="sticky top-0 bg-muted"><tr><th className="p-2">原始行号</th><th className="p-2">校验结果</th>{columns.map(key => <th className="p-2" key={key}>{key}</th>)}</tr></thead>
                <tbody>{preview.rows.map(row => <tr key={row.row} className="border-t"><td className="p-2">{row.row}</td><td className={`min-w-36 p-2 ${row.error ? "text-destructive" : ""}`}>{row.error ?? "通过"}</td>{columns.map(key => <td key={key} className="max-w-72 whitespace-pre-wrap break-words p-2">{typeof row.values[key] === "string" ? String(row.values[key]) : JSON.stringify(row.values[key])}</td>)}</tr>)}</tbody>
              </table>
            </div>
          </>}
          <div className="flex flex-wrap justify-end gap-2">
            <Button variant="outline" disabled={busy} onClick={close}>取消</Button>
            {preview ? <Button variant="outline" disabled={busy} onClick={() => void run(discard)}>重新选择文件</Button> : <Button disabled={busy || !file} onClick={() => void run(async () => {
              if (!file) return;
              if (file.size > 8 * 1024 * 1024) throw new Error("文件不能超过8MB。");
              const form = new FormData(); form.append("file", file);
              setPreview(await api<Preview>(`${endpoint}/imports`, { method: "POST", body: form }));
            })}>预览导入</Button>}
            {preview?.valid && <Button disabled={busy} onClick={() => void run(async () => {
              const result = await api<{items: unknown[]}>(`${endpoint}/imports/${preview.id}/commits`, { method: "POST" });
              setPreview(undefined); setFile(undefined); setOpen(false); onChanged(); toast.success(`已新增 ${result.items.length} 项`);
            })}>确认新增 {preview.count} 项</Button>}
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>}
    {canExport && <DropdownMenu><DropdownMenuTrigger asChild><Button size="sm" variant="outline" disabled={busy}>{busy && !open ? <Loader2 className="animate-spin" /> : <Download />} 导出</Button></DropdownMenuTrigger>
      <DropdownMenuContent align="end">{(["xlsx", "csv"] as const).map(format => <DropdownMenuItem key={format} onSelect={() => void run(() => download(`${endpoint}/exports${queryString({ ...params, format })}`, `${resource}.${format}`))}>导出 {format.toUpperCase()}（当前筛选全部记录）</DropdownMenuItem>)}</DropdownMenuContent>
    </DropdownMenu>}
  </>;
}
