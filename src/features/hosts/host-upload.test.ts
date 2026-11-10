import { expect, it } from "vitest";
import {
  droppedFiles,
  selectedFiles,
  transferSize,
  uploadPath,
  uploadPercent,
} from "./host-upload";

it("preserves folder paths and rejects traversal, absolute and excessive selections", () => {
  const file = new File(["hello"], "file.txt");
  Object.defineProperty(file, "webkitRelativePath", {
    value: "folder/nested/file.txt",
  });
  expect(selectedFiles([file])[0]).toEqual({
    path: "folder/nested/file.txt",
    file,
  });
  for (const path of [
    "/etc/passwd",
    "../file",
    "a/../b",
    "a//b",
    "a\\b",
    "a/./b",
    "a\u0000b",
    "a/".repeat(65) + "b",
  ])
    expect(() => uploadPath(path)).toThrow();
  expect(() => selectedFiles(Array(10001).fill(file))).toThrow(
    "host-files.uploadLimit",
  );
});

it("reads every directory batch and retains empty dragged directories", async () => {
  const file = new File(["x"], "child.txt");
  const child = {
    name: file.name,
    isFile: true,
    file: (resolve: (f: File) => void) => resolve(file),
  };
  const empty = {
    name: "empty",
    isDirectory: true,
    createReader: () => ({
      readEntries: (resolve: (v: unknown[]) => void) => resolve([]),
    }),
  };
  const batches = [[child], [empty], []];
  const root = {
    name: "root",
    isDirectory: true,
    createReader: () => ({
      readEntries: (resolve: (v: unknown[]) => void) =>
        resolve(batches.shift()!),
    }),
  };
  const result = await droppedFiles({
    items: [{ webkitGetAsEntry: () => root }],
  } as unknown as DataTransfer);
  expect(result.map((v) => v.path)).toEqual([
    "root",
    "root/child.txt",
    "root/empty",
  ]);
});

it("handles unknown totals, empty files and transfer size units", () => {
  expect(uploadPercent(0, 0)).toBe(0);
  expect(uploadPercent(0, 0, true)).toBe(100);
  expect(uploadPercent(11, 10)).toBe(100);
  expect(uploadPercent(3, 10)).toBe(30);
  expect(transferSize(0)).toBe("0 B");
  expect(transferSize(1048576)).toBe("1.0 MB");
});
