import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { i18n } from "../../shared/i18n/i18n";
import { HostTerminal } from "./HostTerminal";

const terminals = vi.hoisted(() => [] as Array<{ dispose: ReturnType<typeof vi.fn> }>);
vi.mock("@xterm/xterm", () => ({ Terminal: class {
  cols = 80; rows = 24; options = {};
  dispose = vi.fn();
  constructor() { terminals.push(this); }
  loadAddon() {} open() {} write() {} onData() { return { dispose() {} }; }
} }));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: class { fit() {} } }));
vi.mock("../../shared/theme/theme", () => ({ useTheme: () => ({}), createThemeColorReader: () => (_key: string, fallback: string) => fallback }));
vi.mock("./HostTelemetry", () => ({ TerminalMetrics: () => null }));

class Socket {
  static OPEN = 1;
  static instances: Socket[] = [];
  readyState = 1;
  binaryType = "";
  onopen?: () => Promise<void>;
  onclose?: () => void;
  onerror?: () => void;
  onmessage?: (event: { data: unknown }) => void;
  send = vi.fn();
  close = vi.fn(() => { this.readyState = 3; this.onclose?.(); });
  constructor() { Socket.instances.push(this); }
}
let root: Root, element: HTMLDivElement;
beforeEach(async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  await i18n.changeLanguage("zh-CN");
  Socket.instances = []; terminals.length = 0;
  vi.stubGlobal("WebSocket", Socket);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  vi.stubGlobal("fetch", vi.fn(async () => ({ ok: true, json: async () => ({ os: "linux" }) })));
  element = document.createElement("div"); document.body.append(element); root = createRoot(element);
  await act(async () => root.render(<HostTerminal workspace="1" hostId="2" />));
});
afterEach(async () => { await act(async () => root.unmount()); element.remove(); vi.unstubAllGlobals(); await i18n.changeLanguage("en-US"); });

it("offers a fresh connection after disconnect without replaying shell input", async () => {
  const original = Socket.instances[0]!;
  await act(async () => original.onopen?.());
  await act(async () => original.close());
  const reconnect = element.querySelector("button");
  expect(reconnect, "a disconnected terminal needs a reconnect action").not.toBeNull();
  expect(reconnect!.textContent).toContain("重新连接");
  expect(reconnect!.title).toContain("新的终端会话");
  await act(async () => reconnect!.click());
  expect(Socket.instances).toHaveLength(2);
  expect(terminals[0]!.dispose).toHaveBeenCalledOnce();
  await act(async () => Socket.instances[1]!.onopen?.());
  expect(Socket.instances[1]!.send).not.toHaveBeenCalled();
  expect(element.textContent).toContain("已连接");
  expect(element.querySelector("button")).toBeNull();
});

it("keeps padding outside the element measured by FitAddon", () => {
  const frame = element.querySelector(".bg-card")!;
  expect(frame.children, "FitAddon must measure an unpadded child, otherwise its last row is clipped").toHaveLength(1);
  expect(frame.firstElementChild!.className).not.toMatch(/\bp-2\b/);
});

it("does not reconnect automatically after errors or shell exit", async () => {
  await act(async () => Socket.instances[0]!.onmessage?.({ data: JSON.stringify({ type: "closed" }) }));
  expect(Socket.instances).toHaveLength(1);
  expect(element.querySelector("button")).not.toBeNull();
});
