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
  vi.stubGlobal("fetch", vi.fn(async () => ({ ok: true, json: async () => ({ items: [{ name: "logs", kind: "directory", size: 0, mode: 493, modified_at: 0 }] }) })));
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

it("hides terminal actions without terminal permission", async () => {
  await act(async () => root.render(<HostFiles workspace="ops" hostId="1" />));
  expect(host.querySelector('button[aria-label="在当前目录打开终端"]')).toBeNull();
  expect(host.querySelector('button[aria-label="在 logs 打开终端"]')).toBeNull();
});
