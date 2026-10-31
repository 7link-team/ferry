import { AutoTable } from "./auto-table";
import { AutoForm, AutoField } from "./auto-form";
/**
 * Two-way file transfer against a host, over whichever channel that host uses.
 *
 * Downloads and uploads both stream through the server without being buffered
 * there, so this deliberately uses a plain `<a download>` for the download and
 * a raw `fetch(PUT)` with the File as the body — not FormData, which would
 * force the whole file into memory on both ends.
 */
import { lazy, Suspense, useCallback, useEffect, useRef, useState } from "react";
import {
  ArrowLeft,
  ArrowRight,
  ArrowUp,
  Download,
  Eye,
  EyeOff,
  HardDrive,
  File as FileIcon,
  FileArchive,
  FileAudio,
  FileCode2,
  FileImage,
  FileJson2,
  FilePlus2,
  FileSpreadsheet,
  FileText,
  FileVideo,
  Folder,
  FolderPlus,
  Link as LinkIcon,
  Loader2,
  RefreshCw,
  Save,
  SquareTerminal,
  Trash2,
  Upload,
  X,
} from "lucide-react";
import { Dialog } from "radix-ui";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { api, errorText } from "../api";
import { base } from "../resource";
import { DeleteConfirmDialog } from "./DeleteConfirmDialog";

interface Entry {
  name: string;
  kind: string;
  size: number;
  mode: number;
  modified_at: number;
  path?: string;
}
type FileSystem = { name: string; path: string; total: number; used: number };
type FileSystems = { platform: string; items: FileSystem[] };
const imageTypes = /\.(png|jpe?g|gif|webp|bmp)$/i;
const textTypes = /\.(txt|log|md|markdown|json|jsonc|ya?ml|toml|xml|csv|ini|conf|config|env|properties|sh|bash|zsh|ps1|bat|cmd|sql|py|js|jsx|ts|tsx|css|scss|html?|go|rs|java|c|cc|cpp|h|hpp|rb|php|vue|svelte|dockerfile)$/i;
const textNames = /^(Dockerfile|Makefile|CMakeLists\.txt|\.gitignore|\.gitattributes|\.env(?:\..+)?|LICENSE|README)$/i;
function fileIcon(name: string) {
  if (imageTypes.test(name)) return FileImage;
  if (/\.jsonc?$/i.test(name)) return FileJson2;
  if (/\.(csv|xlsx?|ods)$/i.test(name)) return FileSpreadsheet;
  if (/\.(zip|tar|gz|tgz|7z|rar)$/i.test(name)) return FileArchive;
  if (/\.(mp3|wav|ogg|flac)$/i.test(name)) return FileAudio;
  if (/\.(mp4|webm|mov|avi)$/i.test(name)) return FileVideo;
  if (/\.(md|markdown|txt|log|pdf)$/i.test(name)) return FileText;
  if (textTypes.test(name) || textNames.test(name)) return FileCode2;
  return FileIcon;
}
function FileKindIcon({ name }: { name: string }) {
  const Icon = fileIcon(name);
  return <Icon className="size-4 shrink-0 text-muted-foreground" />;
}
type Preview = { name: string; path: string; kind: "text" | "image"; value: string; original: string; url?: string };
const HostTerminal = lazy(() => import("./HostTerminal").then(module => ({ default: module.HostTerminal })));
let nextPaneId = 0;

const units = ["B", "KB", "MB", "GB", "TB"];
function size(bytes: number) {
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${unit === 0 ? value : value.toFixed(1)} ${units[unit]}`;
}

/** `/a/b` + `c` -> `/a/b/c`, and `/` + `c` -> `/c`. */
export function join(dir: string, name: string) {
  const normalized = normalizePath(dir);
  return `${normalized === "/" ? "" : normalized.replace(/\/+$/, "")}/${name}`;
}
export function normalizePath(path: string) {
  return /^[a-z]:[\\/]/i.test(path) || path.startsWith("\\\\") ? path.replaceAll("\\", "/") : path;
}
/** `/a/b/c` -> `/a/b`; `/a` -> `/`; `/` -> `/`. */
export function parent(dir: string) {
  dir = normalizePath(dir);
  if (dir === "/") return "/";
  if (/^[a-z]:\/*$/i.test(dir) || /^\/\/[^/]+\/[^/]+\/*$/.test(dir)) return "/";
  dir = dir.replace(/\/+$/, "");
  const cut = dir.lastIndexOf("/");
  if (cut === 2 && /^[a-z]:/i.test(dir)) return dir.slice(0, 3);
  return cut <= 0 ? "/" : dir.slice(0, cut);
}

export function HostFiles({ workspace, hostId, canTerminal = false, initialPath = "/", onOpenTerminal }: { workspace: string; hostId: string; canTerminal?: boolean; initialPath?: string; onOpenTerminal?: (directory: string) => void }) {
  const paneId = useRef(++nextPaneId);
  const pagePath = useRef(location.pathname);
  const currentPath = useRef(initialPath);
  const [path, setPath] = useState(initialPath);
  const [trail, setTrail] = useState({ paths: [initialPath], index: 0 });
  const [draft, setDraft] = useState(initialPath);
  const [entries, setEntries] = useState<Entry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState("");
  const [terminalDirectory, setTerminalDirectory] = useState<string>();
  const [preview, setPreview] = useState<Preview>();
  const [previewing, setPreviewing] = useState("");
  const [pendingDelete, setPendingDelete] = useState<Entry>();
  const [creating, setCreating] = useState<"file" | "directory">();
  const [newName, setNewName] = useState("");
  const [showHidden, setShowHidden] = useState(false);
  const showHiddenRef = useRef(false);
  const [fileSystems, setFileSystems] = useState<FileSystems>();
  const fileSystemsRef = useRef<FileSystems | undefined>(undefined);
  const [fileSystemsError, setFileSystemsError] = useState("");
  const loadSequence = useRef(0);
  const virtualRoot = fileSystems?.platform === "windows" && path === "/";
  const entryPath = (entry: Entry) => entry.path ?? join(path, entry.name);
  useEffect(() => () => { if (preview?.url) URL.revokeObjectURL(preview.url); }, [preview?.url]);
  const picker = useRef<HTMLInputElement>(null);
  const openTerminal = (directory: string) => {
    if (/[\x00-\x1f\x7f]/.test(directory)) { setError("目录路径包含控制字符，无法打开终端。"); return; }
    if (onOpenTerminal) onOpenTerminal(directory);
    else setTerminalDirectory(directory);
  };

  const endpoint = `/api${base(workspace, "hosts")}/${encodeURIComponent(hostId)}/files`;

  const load = useCallback(
    async (target: string, remember = false): Promise<boolean> => {
      target = normalizePath(target);
      const sequence = ++loadSequence.current;
      setLoading(true);
      setError("");
      try {
        let body: { path?: string; items: Entry[] };
        if (fileSystemsRef.current?.platform === "windows" && target === "/") {
          body = { path: "/", items: fileSystemsRef.current.items.map(disk => ({ name: disk.path, path: disk.path, kind: "directory", size: disk.total, mode: 0, modified_at: 0 })) };
        } else {
        const response = await fetch(`${endpoint}?path=${encodeURIComponent(target)}&show_hidden=${showHiddenRef.current}`, {
          headers: { "X-Ferry-Request": "1" },
        });
        const result = await response.json();
        if (!response.ok) throw new Error(result.error ?? "读取目录失败。");
        body = result;
        }
        if (sequence !== loadSequence.current) return false;
        target = body.path ?? target;
        setEntries(body.items.filter(entry => showHiddenRef.current || !entry.name.startsWith('.')));
        const changed = target !== currentPath.current;
        currentPath.current = target;
        setPath(target);
        setDraft(fileSystemsRef.current?.platform === "windows" && target === "/" ? "" : target);
        if (remember) setTrail(current => current.paths[current.index] === target ? current : {
          paths: [...current.paths.slice(0, current.index + 1), target], index: current.index + 1,
        });
        if (remember && changed) {
          const state = history.state ?? {};
          history.pushState({ ...state, ferryFilePaths: { ...state.ferryFilePaths, [paneId.current]: target } }, "", location.href);
        }
        return true;
      } catch (cause) {
        if (sequence !== loadSequence.current) return false;
        setError(errorText(cause));
        setEntries([]);
        return false;
      } finally {
        if (sequence === loadSequence.current) setLoading(false);
      }
    },
    [endpoint],
  );

  useEffect(() => {
    let cancelled = false;
    setTrail({ paths: [initialPath], index: 0 });
    fileSystemsRef.current = undefined;
    setFileSystems(undefined);
    setFileSystemsError("");
    // Partition discovery can take seconds; never let its completion replace a
    // directory the user has already opened in the meantime.
    const started = loadSequence.current;
    void api<FileSystems>(endpoint.replace(/^\/api/, "").replace(/\/files$/, "/file-systems"))
      .then(value => {
        if (!Array.isArray(value.items) || !value.items.every(disk => typeof disk.path === "string" && disk.path.length > 0)) throw new Error("磁盘分区接口返回了无效数据。");
        if (!cancelled) { fileSystemsRef.current = value; setFileSystems(value); }
      })
      .catch(cause => { if (!cancelled) setFileSystemsError(`磁盘分区读取失败：${errorText(cause)}`); })
      .finally(() => { if (!cancelled && loadSequence.current === started) void load(initialPath); });
    return () => { cancelled = true; ++loadSequence.current; };
  }, [load, initialPath]);
  useEffect(() => {
    const restore = (event: PopStateEvent) => {
      if (location.pathname !== pagePath.current) return;
      const target = event.state?.ferryFilePaths?.[paneId.current] ?? initialPath;
      void load(target);
      setTrail(current => {
        const index = current.paths.indexOf(target);
        return index < 0 ? { paths: [target], index: 0 } : { ...current, index };
      });
    };
    window.addEventListener("popstate", restore);
    return () => window.removeEventListener("popstate", restore);
  }, [initialPath, load]);
  const moveHistory = async (offset: number) => {
    const index = trail.index + offset;
    const target = trail.paths[index];
    if (target) history.go(offset);
  };

  const upload = async (file: File) => {
    setBusy(`上传 ${file.name}…`);
    setError("");
    try {
      // The File is the body directly: the browser streams it, and the server
      // streams it on to the host. FormData would buffer it twice.
      const response = await fetch(
        `${endpoint}/content?path=${encodeURIComponent(join(path, file.name))}`,
        { method: "PUT", headers: { "X-Ferry-Request": "1" }, body: file },
      );
      if (!response.ok) {
        const body = await response.json().catch(() => ({}));
        throw new Error(body.error ?? `上传失败（${response.status}）。`);
      }
      await load(path);
    } catch (cause) {
      setError(errorText(cause));
    } finally {
      setBusy("");
    }
  };

  const remove = async (entry: Entry) => {
    setBusy(`删除 ${entry.name}…`);
    try {
      const response = await fetch(
        `${endpoint}?path=${encodeURIComponent(join(path, entry.name))}`,
        { method: "DELETE", headers: { "X-Ferry-Request": "1" } },
      );
      if (!response.ok) {
        const body = await response.json().catch(() => ({}));
        throw new Error(body.error ?? "删除失败。");
      }
      await load(path);
    } finally {
      setBusy("");
    }
  };

  const createEntry = async () => {
    const name = newName.trim();
    if (!name || name === "." || name === ".." || /[\\/\x00-\x1f\x7f]/.test(name)) {
      setError("请输入不含路径分隔符的文件或文件夹名称。");
      return;
    }
    setBusy(`创建 ${name}…`); setError("");
    try {
      const response = await fetch(endpoint, {
        method: "POST", headers: { "X-Ferry-Request": "1", "Content-Type": "application/json" },
        body: JSON.stringify({ path: join(path, name), kind: creating }),
      });
      if (!response.ok) {
        const body = await response.json().catch(() => ({}));
        throw new Error(body.error ?? `创建失败（${response.status}）。`);
      }
      setCreating(undefined); setNewName("");
      await load(path);
    } catch (cause) { setError(errorText(cause)); }
    finally { setBusy(""); }
  };

  const closePreview = () => {
    if (preview?.kind === "text" && preview.value !== preview.original && !window.confirm("放弃未保存的修改？")) return;
    setPreview(undefined);
  };
  const openPreview = async (entry: Entry) => {
    const image = imageTypes.test(entry.name);
    if (!image && !textTypes.test(entry.name) && !textNames.test(entry.name)) {
      setError(`暂不支持预览“${entry.name}”；请下载后使用对应程序打开。`);
      return;
    }
    const limit = image ? 12 * 1024 * 1024 : 2 * 1024 * 1024;
    if (entry.size > limit) { setError(`文件超过${image ? "12" : "2"} MB，请下载后查看。`); return; }
    const target = join(path, entry.name);
    setPreviewing(entry.name); setError("");
    try {
      const response = await fetch(`${endpoint}/content?path=${encodeURIComponent(target)}`, { headers: { "X-Ferry-Request": "1" } });
      if (!response.ok) throw new Error(`读取文件失败（${response.status}）。`);
      if (Number(response.headers.get("content-length") ?? 0) > limit || !response.body) throw new Error("文件过大，请下载后查看。");
      const reader = response.body.getReader();
      const chunks: Uint8Array[] = [];
      let length = 0;
      try {
        for (;;) {
          const { done, value } = await reader.read();
          if (done) break;
          length += value.byteLength;
          if (length > limit) throw new Error("文件过大，请下载后查看。");
          chunks.push(value);
        }
      } finally { await reader.cancel().catch(() => {}); reader.releaseLock(); }
      const bytes = new Uint8Array(length);
      let offset = 0;
      for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.byteLength; }
      if (image) {
        const type = entry.name.toLowerCase().endsWith(".png") ? "image/png" : entry.name.toLowerCase().endsWith(".webp") ? "image/webp" : entry.name.toLowerCase().endsWith(".gif") ? "image/gif" : entry.name.toLowerCase().endsWith(".bmp") ? "image/bmp" : "image/jpeg";
        setPreview({ name: entry.name, path: target, kind: "image", value: "", original: "", url: URL.createObjectURL(new Blob([bytes], { type })) });
      } else {
        const value = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
        if (value.includes("\0")) throw new Error("二进制文件无法作为文本预览，请下载后查看。");
        setPreview({ name: entry.name, path: target, kind: "text", value, original: value });
      }
    } catch (cause) { setError(errorText(cause)); }
    finally { setPreviewing(""); }
  };
  const savePreview = async () => {
    if (!preview || preview.kind !== "text") return;
    const saved = preview;
    setBusy(`保存 ${preview.name}…`); setError("");
    try {
      const response = await fetch(`${endpoint}/content?path=${encodeURIComponent(preview.path)}`, {
        method: "PUT", headers: { "X-Ferry-Request": "1" }, body: new Blob([preview.value], { type: "text/plain;charset=utf-8" }),
      });
      if (!response.ok) {
        const body = await response.json().catch(() => ({}));
        throw new Error(body.error ?? `保存失败（${response.status}）。`);
      }
      setPreview(current => current?.path === saved.path ? { ...current, original: saved.value } : current);
      await load(path);
    } catch (cause) { setError(errorText(cause)); }
    finally { setBusy(""); }
  };

  return (
    <div className="flex h-full min-h-0 min-w-0 gap-3 max-lg:flex-col">
    {pendingDelete && <DeleteConfirmDialog target={join(path, pendingDelete.name)} onClose={() => setPendingDelete(undefined)} onConfirm={() => remove(pendingDelete)} />}
    <Dialog.Root open={!!creating} onOpenChange={open => { if (!open && !busy) { setCreating(undefined); setError(""); } }}>
      <Dialog.Portal><Dialog.Overlay className="fixed inset-0 z-50 bg-black/50" />
        <Dialog.Content className="fixed left-1/2 top-1/2 z-50 w-[min(28rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 rounded-md border bg-background p-5 shadow-lg">
          <Dialog.Title className="text-base font-semibold">新建{creating === "directory" ? "文件夹" : "文件"}</Dialog.Title>
          <Dialog.Description className="mt-1 truncate text-sm text-muted-foreground" title={path}>位置：{path}</Dialog.Description>
          <AutoForm className="mt-4 space-y-4" onSubmit={event => { event.preventDefault(); void createEntry(); }}>
            <AutoField label="名称" required><Input autoFocus name="name" required aria-label="名称" value={newName} onChange={event => setNewName(event.target.value)} /></AutoField>
            {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
            <div className="flex justify-end gap-2"><Button type="button" variant="outline" disabled={!!busy} onClick={() => setCreating(undefined)}>取消</Button><Button type="submit" disabled={!!busy || !newName.trim()}>{busy ? "创建中…" : "创建"}</Button></div>
          </AutoForm>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
    <div
      className="grid min-h-0 min-w-0 flex-1 grid-rows-[auto_auto_1fr] gap-3"
      onDragOver={(event) => event.preventDefault()}
      onDrop={(event) => {
        event.preventDefault();
        const file = event.dataTransfer.files[0];
        if (file) void upload(file);
      }}
    >
      <AutoForm
        className="flex flex-wrap gap-2"
        onSubmit={(event) => {
          event.preventDefault();
          void load(draft.trim() || "/", true);
        }}
      >
        <Button type="button" size="icon" variant="outline" onClick={() => void moveHistory(-1)} disabled={trail.index === 0} aria-label="后退" title="后退"><ArrowLeft /></Button>
        <Button type="button" size="icon" variant="outline" onClick={() => void moveHistory(1)} disabled={trail.index >= trail.paths.length - 1} aria-label="前进" title="前进"><ArrowRight /></Button>
        <Button type="button" size="icon" variant="outline" onClick={() => void load(parent(path), true)}
                disabled={path === "/"} aria-label="上级目录">
          <ArrowUp />
        </Button>
        {!!fileSystems?.items.length && <AutoField label="磁盘分区" hideLabel className="min-w-36 max-w-64">
          <select aria-label="磁盘分区" className="h-9 w-full rounded-md border border-input bg-background px-2 text-sm"
            value={virtualRoot ? "__computer__" : [...fileSystems.items].sort((a,b) => b.path.length-a.path.length).find(disk => path === disk.path || path.startsWith(disk.path.endsWith('/') ? disk.path : `${disk.path}/`))?.path ?? "__none__"}
            onChange={event => void load(event.target.value === "__computer__" ? "/" : event.target.value, true)}>
            <option value="__none__" disabled>选择磁盘分区</option>
            {fileSystems.platform === "windows" && <option value="__computer__">此电脑 · 所有磁盘</option>}
            {fileSystems.items.map(disk => <option key={disk.path} value={disk.path}>{disk.path}{disk.name && disk.name !== disk.path ? ` · ${disk.name}` : ''}</option>)}
          </select>
        </AutoField>}
        <AutoField label="目录路径" hideLabel className="min-w-32 flex-1"><Input aria-label="目录路径" placeholder={virtualRoot ? "此电脑 · 请选择磁盘分区" : "完整目录路径"} value={draft} onChange={(event) => setDraft(event.target.value)} className="w-full font-mono" /></AutoField>
        <Button type="submit" variant="outline" size="icon" aria-label="转到目录" title="转到目录"><ArrowUp className="rotate-90" /></Button>
        <Button type="button" variant="outline" size="icon" onClick={() => void load(path)} aria-label="刷新">
          <RefreshCw className={loading ? "animate-spin" : ""} />
        </Button>
        <Button type="button" variant={showHidden ? "secondary" : "outline"} size="icon" aria-label="显示隐藏文件" aria-pressed={showHidden} title={showHidden ? "隐藏文件：显示中" : "隐藏文件：已隐藏"} onClick={() => { const value = !showHidden; showHiddenRef.current = value; setShowHidden(value); void load(path); }}>{showHidden ? <Eye /> : <EyeOff />}</Button>
        {canTerminal && <Button type="button" variant="outline" size="icon" disabled={virtualRoot} aria-label="在当前目录打开终端" title={`在 ${path} 打开终端`} onClick={() => openTerminal(path)}><SquareTerminal /></Button>}
        <Button type="button" variant="outline" size="icon" disabled={virtualRoot} aria-label="新建文件夹" title="新建文件夹" onClick={() => { setCreating("directory"); setNewName(""); setError(""); }}><FolderPlus /></Button>
        <Button type="button" variant="outline" size="icon" disabled={virtualRoot} aria-label="新建文件" title="新建文件" onClick={() => { setCreating("file"); setNewName(""); setError(""); }}><FilePlus2 /></Button>
        <Button type="button" variant="outline" size="icon" aria-label="上传文件" title="上传文件" onClick={() => picker.current?.click()} disabled={!!busy || virtualRoot}><Upload /></Button>
        <input
          ref={picker}
          type="file"
          className="hidden"
          onChange={(event) => {
            const file = event.target.files?.[0];
            if (file) void upload(file);
            event.target.value = "";
          }}
        />
      </AutoForm>
      <div className="min-w-0 space-y-1">
      <div className="flex min-w-0 flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted-foreground">
        <span className="min-w-0 truncate font-mono" title={virtualRoot ? "此电脑" : path}>当前位置：{virtualRoot ? "此电脑" : path}</span>
        {fileSystemsError && <span role="status" className="text-destructive">{fileSystemsError}</span>}
      </div>

      {(error || busy) && <div className="min-h-5 text-sm">
        {error && <span className="text-destructive">{error}</span>}
        {!error && busy && (
          <span className="text-muted-foreground">
            <Loader2 className="mr-1 inline size-3.5 animate-spin" />
            {busy}
          </span>
        )}
      </div>}
      </div>

      <AutoTable<Entry> fill minWidth={680} pagination={false} rowKey="name" sticky={{ header: true, right: 1 }}
        state={{ rows: entries, total: entries.length, query: { page: 1, pageSize: Math.max(1, entries.length) }, loading }}
        empty={error ? "" : "这个目录是空的。"} onRowClick={entry => { if (entry.kind === "directory") void load(entryPath(entry), true); else if (entry.kind === "file") void openPreview(entry); }}
        rowClickable={entry => entry.kind === "directory" || entry.kind === "file"}
        columns={[
{ field: 'name', name: '名称', render: (_value, entry) => <>
                  {entry.kind === "directory" ? (
                    <button
                      type="button"
                      className="flex w-full min-w-0 items-center gap-2 text-left hover:underline"
                      onClick={() => void load(entryPath(entry), true)}
                    >
                      {virtualRoot ? <HardDrive className="size-4 shrink-0 text-muted-foreground" /> : <Folder className="size-4 shrink-0 text-muted-foreground" />}
                      <span className="min-w-0 truncate" title={entry.name}>{entry.name}</span>
                    </button>
                  ) : (
                    <button type="button" className="flex w-full min-w-0 items-center gap-2 text-left hover:underline" onClick={() => void openPreview(entry)} disabled={entry.kind !== "file"}>
                      {entry.kind === "symlink" ? <LinkIcon className="size-4 shrink-0 text-muted-foreground" /> : <FileKindIcon name={entry.name} />}
                      <span className="min-w-0 truncate" title={entry.name}>{entry.name}</span>
                    </button>
                  )}
                </> },
{ field: 'size', name: '大小', width: "96px", render: (_value, entry) => <>
                  {entry.kind === "directory" && !virtualRoot ? "—" : size(entry.size)}
                </> },
{ field: 'mode', name: '权限', width: "80px", render: (_value, entry) => <>
                  {(entry.mode & 0o777).toString(8).padStart(3, "0")}
                </> },
{ field: 'modified_at', name: '修改时间', width: "176px", render: (_value, entry) => <>
                  {entry.modified_at ? new Date(entry.modified_at).toLocaleString("zh-CN") : "—"}
                </> }
        ]} actionWidth={virtualRoot ? 60 : 144} actionsContent={entry => <>
                  {canTerminal && entry.kind === "directory" && <Button type="button" variant="ghost" size="icon" className="size-7" aria-label={`在 ${entry.name} 打开终端`} title={virtualRoot ? "在此盘符打开终端" : "在此目录打开终端"} onClick={() => openTerminal(entryPath(entry))}><SquareTerminal className="size-4" /></Button>}
                  {!virtualRoot && <>
                  {entry.kind === "file" && <Button type="button" variant="ghost" size="icon" className="size-7" aria-label={`预览 ${entry.name}`} title="预览和编辑" disabled={!!previewing} onClick={() => void openPreview(entry)}>{previewing === entry.name ? <Loader2 className="size-4 animate-spin" /> : <Eye className="size-4" />}</Button>}
                  {entry.kind !== "directory" && (
                    // A real link so the browser streams the download itself.
                    <a
                      className="mr-1 inline-flex size-7 items-center justify-center text-primary hover:bg-accent"
                      href={`${endpoint}/content?path=${encodeURIComponent(join(path, entry.name))}`}
                      download={entry.name}
                      aria-label={`下载 ${entry.name}`}
                      title={`下载 ${entry.name}`}
                    >
                      <Download className="size-4" />
                    </a>
                  )}
                  <button
                    type="button"
                    className="inline-flex size-7 items-center justify-center text-destructive hover:bg-accent"
                    aria-label={`删除 ${entry.name}`}
                    title={`删除 ${entry.name}`}
                    onClick={() => setPendingDelete(entry)}
                  >
                    <Trash2 className="inline size-3.5" />
                  </button>
                  </>}
                </>} />
    </div>
    {terminalDirectory && !onOpenTerminal && <aside aria-label="目录终端" className="flex min-h-[360px] min-w-0 flex-col border-t pt-3 lg:min-h-0 lg:w-[min(42vw,40rem)] lg:shrink-0 lg:border-t-0 lg:border-l lg:pt-0 lg:pl-3">
      <div className="mb-2 flex items-center gap-2"><SquareTerminal className="size-4 shrink-0" /><span className="min-w-0 flex-1 truncate font-mono text-sm" title={terminalDirectory}>{terminalDirectory}</span><Button type="button" variant="ghost" size="icon" aria-label="关闭目录终端" onClick={() => setTerminalDirectory(undefined)}><X className="size-4" /></Button></div>
      <div className="min-h-0 flex-1"><Suspense fallback={<p role="status">正在加载终端…</p>}><HostTerminal workspace={workspace} hostId={hostId} directory={terminalDirectory} /></Suspense></div>
    </aside>}
    <Dialog.Root open={!!preview} onOpenChange={open => { if (!open) closePreview(); }}>
      <Dialog.Portal><Dialog.Overlay className="fixed inset-0 z-50 bg-black/50" />
        <Dialog.Content className="fixed inset-4 z-50 flex flex-col border bg-background shadow-xl sm:inset-8">
          <div className="flex h-12 shrink-0 items-center gap-2 border-b px-3">
            <Dialog.Title className="min-w-0 flex-1 truncate text-sm font-semibold" title={preview?.path}>{preview?.name}</Dialog.Title>
            <Dialog.Description className="sr-only">文件预览与文本编辑</Dialog.Description>
            {preview?.kind === "text" && <Button size="sm" onClick={() => void savePreview()} disabled={!!busy || preview.value === preview.original}><Save className="size-4" />保存</Button>}
            <Button variant="ghost" size="icon" aria-label="关闭预览" onClick={closePreview}><X className="size-4" /></Button>
          </div>
          {error && <p role="alert" className="border-b px-3 py-2 text-sm text-destructive">{error}</p>}
          {preview?.kind === "image" ? <div className="grid min-h-0 flex-1 place-items-center overflow-auto p-4"><img src={preview.url} alt={preview.name} className="max-h-full max-w-full object-contain" /></div> :
            <textarea aria-label="文件内容" spellCheck={false} value={preview?.value ?? ""} onChange={event => setPreview(current => current ? { ...current, value: event.target.value } : current)} className="min-h-0 flex-1 resize-none bg-background p-4 font-mono text-sm leading-relaxed outline-none" />}
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
    </div>
  );
}
