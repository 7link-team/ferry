import { useEffect, useRef, useState } from "react";
import { Dialog } from "radix-ui";
import { Check, File, Folder, Loader2, Upload, X } from "lucide-react";
import { Button } from "../../shared/ui/button";
import { api, errorText, localizedFetch } from "../../shared/api/api";
import { localeHeaders } from "../../shared/i18n/locale";
import { hostRepositoryText as t } from "../../shared/i18n/i18n";
import {
  droppedFiles,
  selectedFiles,
  transferSize,
  uploadPercent,
  type UploadSelection,
} from "./host-upload";

type Transfer = UploadSelection & {
  state:
    | "waiting"
    | "uploading"
    | "confirming"
    | "succeeded"
    | "failed"
    | "cancelled";
  loaded: number;
  speed: number;
  error?: string;
};
function Bar({ value, label }: { value: number; label: string }) {
  return (
    <div
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={value}
      className="h-2 w-full overflow-hidden rounded-full bg-muted"
    >
      <div
        className="h-full rounded-full bg-primary transition-[width] duration-150"
        style={{ width: `${value}%` }}
      />
    </div>
  );
}
export function HostUpload({
  endpoint,
  directory,
  initialSelection,
  onClose,
  onComplete,
}: {
  endpoint: string;
  directory: string;
  initialSelection?: Promise<UploadSelection[]>;
  onClose: () => void;
  onComplete: () => void;
}) {
  const [rows, setRows] = useState<Transfer[]>([]),
    [running, setRunning] = useState(false),
    [preparing, setPreparing] = useState(false),
    [error, setError] = useState("");
  const [page, setPage] = useState(0),
    [started, setStarted] = useState(false);
  const picker = useRef<HTMLInputElement>(null),
    folder = useRef<HTMLInputElement>(null),
    request = useRef<XMLHttpRequest | null>(null);
  const cancelled = useRef(false),
    mounted = useRef(true),
    choosing = useRef(0),
    queue = useRef<Transfer[]>([]),
    active = useRef(false);
  queue.current = rows;
  const patch = (index: number, values: Partial<Transfer>) => {
    if (mounted.current)
      setRows((current) =>
        current.map((row, i) => (i === index ? { ...row, ...values } : row)),
      );
  };
  const failure = (cause: unknown) =>
    cause instanceof Error && cause.message.startsWith("host-files.")
      ? t(cause.message)
      : errorText(cause);
  const choose = async (
    selection: Promise<UploadSelection[]> | UploadSelection[],
  ) => {
    const sequence = ++choosing.current;
    setPreparing(true);
    setError("");
    try {
      const items = await selection;
      if (!mounted.current || sequence !== choosing.current) return;
      const combined = new Map(queue.current.map((row) => [row.path, row]));
      for (const item of items)
        combined.set(item.path, {
          ...item,
          state: "waiting",
          loaded: 0,
          speed: 0,
        });
      if (combined.size > 10000) throw Error("host-files.uploadLimit");
      queue.current = [...combined.values()];
      setRows(queue.current);
      setPage(0);
    } catch (cause) {
      if (mounted.current) setError(failure(cause));
    } finally {
      if (mounted.current && sequence === choosing.current) setPreparing(false);
    }
  };
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      cancelled.current = true;
      request.current?.abort();
    };
  }, []);
  useEffect(() => {
    if (initialSelection) void choose(initialSelection);
  }, [initialSelection]);
  useEffect(() => {
    if (!running) return;
    const guard = (event: BeforeUnloadEvent) => {
      event.preventDefault();
      event.returnValue = "";
    };
    window.addEventListener("beforeunload", guard);
    return () => window.removeEventListener("beforeunload", guard);
  }, [running]);
  const path = (relative: string) =>
    `${directory.endsWith("/") ? directory : directory + "/"}${relative}`;
  const ensureDirectory = async (relative: string, known: Set<string>) => {
    let part = "";
    for (const name of relative.split("/").filter(Boolean)) {
      part = part ? part + "/" + name : name;
      if (known.has(part)) continue;
      if (cancelled.current) throw Error(t("host-files.uploadCancelled"));
      try {
        await api(endpoint.replace(/^\/api/, ""), {
          method: "POST",
          body: JSON.stringify({ path: path(part), kind: "directory" }),
        });
      } catch (cause) {
        // Existing directories are usable; do not mask inaccessible directories.
        const response = await localizedFetch(
          `${endpoint}?path=${encodeURIComponent(path(part))}&show_hidden=true`,
          { headers: { "X-Ferry-Request": "1" } },
        );
        if (!response.ok) throw cause;
      }
      known.add(part);
    }
  };
  const upload = (row: Transfer, index: number) =>
    new Promise<void>((resolve, reject) => {
      const xhr = new XMLHttpRequest();
      request.current = xhr;
      xhr.open(
        "PUT",
        `${endpoint}/content?path=${encodeURIComponent(path(row.path))}`,
      );
      for (const [name, value] of Object.entries(
        localeHeaders({
          "X-Ferry-Request": "1",
          "Content-Type": "application/octet-stream",
        }),
      ))
        xhr.setRequestHeader(name, value);
      const start = performance.now();
      let last = start,
        previous = 0,
        speed = 0;
      xhr.upload.onprogress = (event) => {
        const now = performance.now(),
          elapsed = now - last;
        if (elapsed >= 200) {
          speed = ((event.loaded - previous) * 1000) / elapsed;
          last = now;
          previous = event.loaded;
        }
        patch(index, {
          loaded: Math.min(row.file!.size, event.loaded),
          speed,
          state: event.loaded >= row.file!.size ? "confirming" : "uploading",
        });
      };
      xhr.onload = () => {
        request.current = null;
        if (xhr.status >= 200 && xhr.status < 300) {
          patch(index, {
            state: "succeeded",
            loaded: row.file!.size,
            speed:
              (row.file!.size * 1000) / Math.max(1, performance.now() - start),
          });
          resolve();
        } else {
          let message = t("host-files.006", { value1: xhr.status });
          try {
            message = JSON.parse(xhr.responseText).error || message;
          } catch {
            /* Non-JSON proxy response. */
          }
          reject(Error(message));
        }
      };
      xhr.onerror = () => {
        request.current = null;
        reject(Error(t("host-files.uploadNetwork")));
      };
      xhr.onabort = () => {
        request.current = null;
        reject(Error(t("host-files.uploadCancelled")));
      };
      // Stream the original File; never read the whole object or build multipart data.
      xhr.send(row.file!);
    });
  const start = async () => {
    if (active.current || preparing || !rows.length) return;
    active.current = true;
    cancelled.current = false;
    setRunning(true);
    setStarted(true);
    setError("");
    const known = new Set<string>();
    for (let index = 0; index < rows.length; index++) {
      const row = rows[index]!;
      if (row.state === "succeeded") continue;
      if (cancelled.current) {
        patch(index, { state: "cancelled", speed: 0 });
        continue;
      }
      patch(index, { state: "uploading", loaded: 0, speed: 0, error: "" });
      try {
        await ensureDirectory(
          row.file ? row.path.split("/").slice(0, -1).join("/") : row.path,
          known,
        );
        if (cancelled.current) throw Error(t("host-files.uploadCancelled"));
        if (row.file) await upload(row, index);
        else patch(index, { state: "succeeded" });
      } catch (cause) {
        patch(index, {
          state: cancelled.current ? "cancelled" : "failed",
          error: failure(cause),
          speed: 0,
        });
      }
    }
    active.current = false;
    if (mounted.current) {
      setRunning(false);
      onComplete();
    }
  };
  const total = rows.reduce((n, row) => n + (row.file?.size ?? 0), 0),
    loaded = rows.reduce((n, row) => n + row.loaded, 0);
  const finished = rows.filter((row) => row.state === "succeeded").length,
    failed = rows.filter((row) => row.state === "failed").length;
  const percent = uploadPercent(
      loaded,
      total,
      !!rows.length && finished === rows.length,
    ),
    speed = rows.reduce(
      (n, row) => n + (row.state === "uploading" ? row.speed : 0),
      0,
    );
  return (
    <Dialog.Root
      open
      onOpenChange={(open) => {
        if (!open && !running && !preparing) onClose();
      }}
    >
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-50 bg-black/50" />
        <Dialog.Content
          aria-describedby="host-upload-description"
          className="dialog-shell fixed left-1/2 top-1/2 z-50 flex max-h-[min(90dvh,52rem)] w-[min(48rem,calc(100vw-1.5rem))] -translate-x-1/2 -translate-y-1/2 flex-col overflow-hidden rounded-xl border bg-background shadow-xl"
          onEscapeKeyDown={(event) => {
            if (running || preparing) event.preventDefault();
          }}
          onPointerDownOutside={(event) => {
            if (running || preparing) event.preventDefault();
          }}
        >
          <div className="shrink-0 border-b p-4">
            <Dialog.Title className="flex items-center gap-2 font-semibold">
              <Upload className="size-5" />
              {t("host-files.045")}
            </Dialog.Title>
            <Dialog.Description
              id="host-upload-description"
              className="mt-1 break-all text-sm text-muted-foreground"
            >
              {t("host-files.047")} {directory}
            </Dialog.Description>
          </div>
          <div className="min-h-0 flex-1 space-y-4 overflow-y-auto p-4">
            {!started && (
              <div
                className="rounded-lg border-2 border-dashed p-5 text-center"
                onDragOver={(e) => e.preventDefault()}
                onDrop={(e) => {
                  e.preventDefault();
                  void choose(droppedFiles(e.dataTransfer));
                }}
              >
                <Upload className="mx-auto mb-2 size-7 text-muted-foreground" />
                <p className="text-sm">{t("host-files.uploadDrop")}</p>
                <div className="mt-3 flex flex-wrap justify-center gap-2">
                  <Button
                    variant="outline"
                    disabled={preparing}
                    onClick={() => picker.current?.click()}
                  >
                    {t("host-files.uploadFiles")}
                  </Button>
                  <Button
                    variant="outline"
                    disabled={preparing}
                    onClick={() => folder.current?.click()}
                  >
                    <Folder className="size-4" />
                    {t("host-files.uploadFolder")}
                  </Button>
                </div>
                <input
                  ref={picker}
                  aria-label={t("host-files.uploadFiles")}
                  type="file"
                  multiple
                  className="hidden"
                  onChange={(e) => {
                    try {
                      void choose(selectedFiles(e.target.files ?? []));
                    } catch (cause) {
                      setError(failure(cause));
                    }
                    e.target.value = "";
                  }}
                />
                <input
                  ref={folder}
                  aria-label={t("host-files.uploadFolder")}
                  type="file"
                  multiple
                  {...{ webkitdirectory: "" }}
                  className="hidden"
                  onChange={(e) => {
                    try {
                      void choose(selectedFiles(e.target.files ?? []));
                    } catch (cause) {
                      setError(failure(cause));
                    }
                    e.target.value = "";
                  }}
                />
                <p className="mt-3 text-xs text-muted-foreground">
                  {t("host-files.uploadConfirmHelp")}
                </p>
              </div>
            )}
            {preparing && (
              <p role="status" className="flex items-center gap-2 text-sm">
                <Loader2 className="size-4 animate-spin" />
                {t("host-files.uploadPreparing")}
              </p>
            )}
            {error && (
              <p role="alert" className="text-sm text-destructive">
                {error}
              </p>
            )}
            {!!rows.length && (
              <section
                className="space-y-2 rounded-lg border bg-muted/40 p-3"
                aria-label={t("host-files.uploadOverall")}
              >
                <div className="flex justify-between gap-2 text-sm font-semibold">
                  <span>
                    {t("host-files.uploadOverall")} · {finished}/{rows.length}
                  </span>
                  <span>{percent}%</span>
                </div>
                <Bar label={t("host-files.uploadOverall")} value={percent} />
                <div className="flex flex-wrap justify-between gap-2 text-xs tabular-nums">
                  <span>
                    {transferSize(loaded)} / {transferSize(total)}
                  </span>
                  <span>{transferSize(speed)}/s</span>
                </div>
                {failed > 0 && (
                  <p role="alert" className="text-sm text-destructive">
                    {t("host-files.uploadFailedCount", { count: failed })}
                  </p>
                )}
              </section>
            )}
            <ul className="space-y-2">
              {rows.slice(page * 100, (page + 1) * 100).map((row, offset) => {
                const index = page * 100 + offset,
                  value = uploadPercent(
                    row.loaded,
                    row.file?.size ?? 0,
                    row.state === "succeeded",
                  );
                return (
                  <li
                    key={row.path}
                    className={`min-w-0 space-y-2 rounded-lg border p-3 ${row.state === "failed" ? "border-destructive/50 bg-destructive/5" : "bg-card"}`}
                  >
                    <div className="flex items-start gap-2 text-sm">
                      {row.file ? (
                        <File className="mt-0.5 size-4 shrink-0" />
                      ) : (
                        <Folder className="mt-0.5 size-4 shrink-0" />
                      )}
                      <span className="min-w-0 flex-1 break-all font-medium">
                        {row.path}
                      </span>
                      {row.state === "succeeded" ? (
                        <Check className="size-4 shrink-0 text-primary" />
                      ) : (
                        !started && (
                          <Button
                            size="icon"
                            variant="ghost"
                            className="size-7 shrink-0"
                            aria-label={t("host-files.uploadRemove", {
                              name: row.path,
                            })}
                            onClick={() =>
                              setRows((current) =>
                                current.filter((_, i) => i !== index),
                              )
                            }
                          >
                            <X className="size-4" />
                          </Button>
                        )
                      )}
                    </div>
                    <Bar label={row.path} value={value} />
                    <div className="flex flex-wrap items-center justify-between gap-2 text-xs tabular-nums">
                      <span>
                        {transferSize(row.loaded)} /{" "}
                        {transferSize(row.file?.size ?? 0)}
                      </span>
                      <span>
                        {transferSize(row.speed)}/s · {value}%
                      </span>
                    </div>
                    <p
                      role={row.state === "failed" ? "alert" : undefined}
                      className={`break-words text-xs ${row.state === "failed" ? "text-destructive" : "text-muted-foreground"}`}
                    >
                      {t(`host-files.uploadState.${row.state}`)}
                      {row.error ? ` · ${row.error}` : ""}
                    </p>
                  </li>
                );
              })}
            </ul>
            {rows.length > 100 && (
              <div className="flex items-center justify-between gap-2 text-sm">
                <Button
                  variant="outline"
                  disabled={!page}
                  onClick={() => setPage((n) => n - 1)}
                >
                  ←
                </Button>
                <span>
                  {page + 1} / {Math.ceil(rows.length / 100)}
                </span>
                <Button
                  variant="outline"
                  disabled={(page + 1) * 100 >= rows.length}
                  onClick={() => setPage((n) => n + 1)}
                >
                  →
                </Button>
              </div>
            )}
          </div>
          <div className="flex shrink-0 gap-3 border-t p-4 pb-[max(1rem,env(safe-area-inset-bottom))]">
            {running ? (
              <Button
                variant="outline"
                className="min-h-11 flex-1"
                onClick={() => {
                  cancelled.current = true;
                  request.current?.abort();
                }}
              >
                {t("host-files.uploadStop")}
              </Button>
            ) : (
              <>
                <Button
                  variant="outline"
                  className="min-h-11 flex-1"
                  disabled={preparing}
                  onClick={onClose}
                >
                  {t("host-files.uploadClose")}
                </Button>
                {finished < rows.length && (
                  <Button
                    className="min-h-11 flex-1"
                    disabled={preparing || !rows.length}
                    onClick={() => void start()}
                  >
                    {started
                      ? t("host-files.uploadRetry")
                      : t("host-files.uploadConfirm")}
                  </Button>
                )}
              </>
            )}
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
