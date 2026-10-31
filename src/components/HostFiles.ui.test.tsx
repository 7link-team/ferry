import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { HostFiles } from "./HostFiles";

vi.mock("./HostTerminal", () => ({ HostTerminal: ({ directory }: { directory: string }) => <div data-testid="terminal-directory">{directory}</div> }));

let root: Root;
let host: HTMLDivElement;
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  vi.stubGlobal("fetch", vi.fn(async (url: string) => ({ ok: true, json: async () => url.endsWith('/file-systems') ? { platform: "unix", items: [{ path: "/", name: "root", total: 1024, used: 128 }] } : { items: [{ name: "logs", kind: "directory", size: 0, mode: 493, modified_at: 0 }] } })));
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); });

it("opens current and selected directories in a closable terminal panel", async () => {
  await act(async () => root.render(<HostFiles workspace="ops" hostId="1" canTerminal />));
  await act(async () => host.querySelector<HTMLButtonElement>('button[aria-label="在当前目录打开终端"]')!.click());
  expect(host.querySelector('[aria-label="目录终端"]')?.textContent).toContain("/");
  await act(async () => host.querySelector<HTMLButtonElement>('button[aria-label="在 logs 打开终端"]')!.click());
  expect(host.querySelector('[data-testid="terminal-directory"]')?.textContent).toBe("/logs");
  await act(async () => host.querySelector<HTMLButtonElement>('button[aria-label="关闭目录终端"]')!.click());
  expect(host.querySelector('[aria-label="目录终端"]')).toBeNull();
});

it("keeps a directory the user opened while partitions were still loading", async () => {
  let partitions!: () => void;
  vi.stubGlobal("fetch", vi.fn((url: string) => url.endsWith("/file-systems")
    ? new Promise(done => { partitions = () => done({ ok: true, json: async () => ({ platform: "unix", items: [{ path: "/", name: "root", total: 1, used: 0 }] }) }); })
    : Promise.resolve({ ok: true, json: async () => ({ path: new URL(url, "http://x").searchParams.get("path"), items: [{ name: url.includes("srv") ? "app.log" : "etc", kind: "file", size: 1, mode: 420, modified_at: 0 }] }) })));
  await act(async () => root.render(<HostFiles workspace="ops" hostId="1" />));
  const input = host.querySelector<HTMLInputElement>('input[aria-label="目录路径"]')!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "/srv");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await act(async () => input.form!.requestSubmit());
  expect(host.textContent).toContain("app.log");
  // The late partition response must not reset the view to the initial directory.
  await act(async () => partitions());
  expect(input.value).toBe("/srv");
  expect(host.textContent).toContain("app.log");
  expect(host.textContent).not.toContain("etc");
});

it("hides terminal actions without terminal permission", async () => {
  await act(async () => root.render(<HostFiles workspace="ops" hostId="1" />));
  expect(host.querySelector('button[aria-label="在当前目录打开终端"]')).toBeNull();
  expect(host.querySelector('button[aria-label="在 logs 打开终端"]')).toBeNull();
});

it("creates entries and confirms a named file deletion", async () => {
  const requests = vi.mocked(fetch);
  await act(async () => root.render(<HostFiles workspace="ops" hostId="1" />));
  await act(async () => host.querySelector<HTMLButtonElement>('button[aria-label="新建文件夹"]')!.click());
  await act(async () => {
    const name = document.body.querySelector<HTMLInputElement>('input[aria-label="名称"]')!;
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(name, "archive");
    name.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await act(async () => document.body.querySelector<HTMLButtonElement>('[role="dialog"] button[type="submit"]')!.click());
  const create = requests.mock.calls.find(([, init]) => init?.method === "POST");
  expect(create?.[1]?.body).toBe(JSON.stringify({ path: "/archive", kind: "directory" }));

  await act(async () => host.querySelector<HTMLButtonElement>('button[aria-label="删除 logs"]')!.click());
  expect(document.body.textContent).toContain('确认删除“/logs”？');
  expect(requests.mock.calls.some(([, init]) => init?.method === "DELETE")).toBe(false);
  await act(async () => [...document.body.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "确认删除")!.click());
  expect(requests.mock.calls.some(([, init]) => init?.method === "DELETE")).toBe(true);
});

it("creates a new file without uploading or replacing an existing file", async () => {
  const requests = vi.mocked(fetch);
  await act(async () => root.render(<HostFiles workspace="ops" hostId="1" />));
  await act(async () => host.querySelector<HTMLButtonElement>('button[aria-label="新建文件"]')!.click());
  await act(async () => {
    const name = document.body.querySelector<HTMLInputElement>('input[aria-label="名称"]')!;
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(name, "notes.txt");
    name.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await act(async () => document.body.querySelector<HTMLButtonElement>('[role="dialog"] button[type="submit"]')!.click());
  const create = requests.mock.calls.find(([, init]) => init?.method === "POST");
  expect(create?.[1]?.body).toBe(JSON.stringify({ path: "/notes.txt", kind: "file" }));
  expect(requests.mock.calls.some(([, init]) => init?.method === "PUT")).toBe(false);
  expect(host.querySelector('[aria-label="文件内容"]')).toBeNull();
});
