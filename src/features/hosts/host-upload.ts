export type UploadSelection = { path: string; file?: File };
const limit = 10000;
export function uploadPath(path: string) {
  if (
    !path ||
    path.length > 4096 ||
    path.startsWith("/") ||
    /[\\\x00-\x1f\x7f]/.test(path) ||
    path.split("/").some((p) => !p || p === "." || p === "..") ||
    path.split("/").length > 64
  )
    throw Error("host-files.uploadInvalidPath");
  return path;
}
export function selectedFiles(files: FileList | File[]): UploadSelection[] {
  if (files.length > limit) throw Error("host-files.uploadLimit");
  return Array.from(files, (file) => ({
    path: uploadPath(file.webkitRelativePath || file.name),
    file,
  }));
}
/** Snapshot handles during drop, before the browser revokes DataTransfer access. */
export function droppedFiles(
  transfer: DataTransfer,
): Promise<UploadSelection[]> {
  const entries = Array.from(transfer.items ?? [])
    .map((item) => item.webkitGetAsEntry?.())
    .filter((e): e is FileSystemEntry => !!e);
  if (!entries.length) return Promise.resolve(selectedFiles(transfer.files));
  const result: UploadSelection[] = [];
  const visit = async (entry: FileSystemEntry, parent = ""): Promise<void> => {
    if (result.length >= limit) throw Error("host-files.uploadLimit");
    const path = uploadPath(parent + entry.name);
    if (entry.isFile) {
      const file = await new Promise<File>((resolve, reject) =>
        (entry as FileSystemFileEntry).file(resolve, reject),
      );
      result.push({ path, file });
    } else if (entry.isDirectory) {
      result.push({ path });
      const reader = (entry as FileSystemDirectoryEntry).createReader();
      for (;;) {
        const batch = await new Promise<FileSystemEntry[]>((resolve, reject) =>
          reader.readEntries(resolve, reject),
        );
        if (!batch.length) break;
        for (const child of batch) await visit(child, path + "/");
      }
    }
  };
  return (async () => {
    for (const entry of entries) await visit(entry);
    return result;
  })();
}
export function uploadPercent(loaded: number, total: number, complete = false) {
  return complete
    ? 100
    : total
      ? Math.min(100, Math.floor((loaded / total) * 100))
      : 0;
}
export function transferSize(bytes: number) {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  while (bytes >= 1024 && i < units.length - 1) {
    bytes /= 1024;
    i++;
  }
  return `${i ? bytes.toFixed(1) : Math.round(bytes)} ${units[i]}`;
}
