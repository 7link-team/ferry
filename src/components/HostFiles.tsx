/**
 * Two-way file transfer against a host, over whichever channel that host uses.
 *
 * Downloads and uploads both stream through the server without being buffered
 * there, so this deliberately uses a plain `<a download>` for the download and
 * a raw `fetch(PUT)` with the File as the body — not FormData, which would
 * force the whole file into memory on both ends.
 */
import { useCallback, useEffect, useRef, useState } from "react";
import {
  ArrowUp,
  File as FileIcon,
  Folder,
  Link as LinkIcon,
  Loader2,
  RefreshCw,
  Trash2,
  Upload,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { errorText } from "../api";
import { base } from "../resource";

interface Entry {
  name: string;
  kind: string;
  size: number;
  mode: number;
  modified_at: number;
}

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
  return `${dir === "/" ? "" : dir}/${name}`;
}
/** `/a/b/c` -> `/a/b`; `/a` -> `/`; `/` -> `/`. */
export function parent(dir: string) {
  if (dir === "/") return "/";
  const cut = dir.lastIndexOf("/");
  return cut <= 0 ? "/" : dir.slice(0, cut);
}

export function HostFiles({ workspace, hostId }: { workspace: string; hostId: string }) {
  const [path, setPath] = useState("/");
  const [draft, setDraft] = useState("/");
  const [entries, setEntries] = useState<Entry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState("");
  const picker = useRef<HTMLInputElement>(null);

  const endpoint = `/api${base(workspace, "hosts")}/${encodeURIComponent(hostId)}/files`;

  const load = useCallback(
    async (target: string) => {
      setLoading(true);
      setError("");
      try {
        const response = await fetch(`${endpoint}?path=${encodeURIComponent(target)}`, {
          headers: { "X-Ferry-Request": "1" },
        });
        const body = await response.json();
        if (!response.ok) throw new Error(body.error ?? "读取目录失败。");
        setEntries(body.items as Entry[]);
        setPath(target);
        setDraft(target);
      } catch (cause) {
        setError(errorText(cause));
        setEntries([]);
      } finally {
        setLoading(false);
      }
    },
    [endpoint],
  );

  useEffect(() => {
    void load("/");
  }, [load]);

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
    if (!window.confirm(`确认删除 ${entry.name}？此操作无法撤销。`)) return;
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
    } catch (cause) {
      setError(errorText(cause));
    } finally {
      setBusy("");
    }
  };

  return (
    <div
      className="grid min-h-0 grid-rows-[auto_auto_1fr] gap-3"
      onDragOver={(event) => event.preventDefault()}
      onDrop={(event) => {
        event.preventDefault();
        const file = event.dataTransfer.files[0];
        if (file) void upload(file);
      }}
    >
      <form
        className="flex gap-2"
        onSubmit={(event) => {
          event.preventDefault();
          void load(draft.trim() || "/");
        }}
      >
        <Button type="button" size="icon" variant="outline" onClick={() => void load(parent(path))}
                disabled={path === "/"} aria-label="上级目录">
          <ArrowUp />
        </Button>
        <Input value={draft} onChange={(event) => setDraft(event.target.value)} className="font-mono" />
        <Button type="submit" variant="outline">转到</Button>
        <Button type="button" variant="outline" size="icon" onClick={() => void load(path)} aria-label="刷新">
          <RefreshCw className={loading ? "animate-spin" : ""} />
        </Button>
        <Button type="button" onClick={() => picker.current?.click()} disabled={!!busy}>
          <Upload /> 上传
        </Button>
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
      </form>

      <div className="min-h-5 text-sm">
        {error && <span className="text-destructive">{error}</span>}
        {!error && busy && (
          <span className="text-muted-foreground">
            <Loader2 className="mr-1 inline size-3.5 animate-spin" />
            {busy}
          </span>
        )}
        {!error && !busy && <span className="text-muted-foreground">可以把文件拖到此处上传</span>}
      </div>

      <div className="min-h-0 overflow-auto rounded-lg border border-border bg-card">
        <table className="w-full border-separate border-spacing-0 text-sm">
          <thead>
            <tr>
              {["名称", "大小", "权限", "修改时间", "操作"].map((label, index) => (
                <th
                  key={label}
                  className={`sticky top-0 z-10 bg-muted px-3 py-2 font-medium text-muted-foreground shadow-[inset_0_-1px_0_var(--border)] ${
                    index === 0 ? "text-left" : index === 4 ? "text-right" : "text-right"
                  }`}
                >
                  {label}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {entries.map((entry) => (
              <tr key={entry.name} className="group">
                <td className="border-b border-border px-3 py-1.5 group-hover:bg-accent">
                  {entry.kind === "directory" ? (
                    <button
                      type="button"
                      className="inline-flex items-center gap-2 hover:underline"
                      onClick={() => void load(join(path, entry.name))}
                    >
                      <Folder className="size-4 text-muted-foreground" />
                      {entry.name}
                    </button>
                  ) : (
                    <span className="inline-flex items-center gap-2">
                      {entry.kind === "symlink" ? (
                        <LinkIcon className="size-4 text-muted-foreground" />
                      ) : (
                        <FileIcon className="size-4 text-muted-foreground" />
                      )}
                      {entry.name}
                    </span>
                  )}
                </td>
                <td className="border-b border-border px-3 py-1.5 text-right text-muted-foreground group-hover:bg-accent">
                  {entry.kind === "directory" ? "—" : size(entry.size)}
                </td>
                <td className="border-b border-border px-3 py-1.5 text-right font-mono text-xs text-muted-foreground group-hover:bg-accent">
                  {(entry.mode & 0o777).toString(8).padStart(3, "0")}
                </td>
                <td className="border-b border-border px-3 py-1.5 text-right text-muted-foreground group-hover:bg-accent">
                  {entry.modified_at ? new Date(entry.modified_at).toLocaleString("zh-CN") : "—"}
                </td>
                <td className="border-b border-border px-3 py-1.5 text-right whitespace-nowrap group-hover:bg-accent">
                  {entry.kind !== "directory" && (
                    // A real link so the browser streams the download itself.
                    <a
                      className="mr-3 text-primary hover:underline"
                      href={`${endpoint}/content?path=${encodeURIComponent(join(path, entry.name))}`}
                      download={entry.name}
                    >
                      下载
                    </a>
                  )}
                  <button
                    type="button"
                    className="text-destructive hover:underline"
                    onClick={() => void remove(entry)}
                  >
                    <Trash2 className="inline size-3.5" />
                  </button>
                </td>
              </tr>
            ))}
            {!loading && !entries.length && (
              <tr>
                <td colSpan={5} className="px-3 py-12 text-center text-muted-foreground">
                  {error ? "" : "这个目录是空的。"}
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </div>
  );
}
