import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useAutoRefresh } from "./use-auto-refresh";

let root: Root;
let host: HTMLDivElement;
beforeEach(() => {
  vi.useFakeTimers();
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.spyOn(document, "hidden", "get").mockReturnValue(false);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.restoreAllMocks(); vi.useRealTimers(); });
function Clock({ seconds, refresh, pending = false }: { seconds: number; refresh: () => Promise<unknown>; pending?: boolean }) {
  const remaining = useAutoRefresh(seconds, refresh, pending);
  return <span role="timer">{remaining}</span>;
}
const advance = async (milliseconds: number) => act(async () => { await vi.advanceTimersByTimeAsync(milliseconds); });

it("counts 10 down to 0, waits for the request and restarts without overlap", async () => {
  let finish!: () => void;
  const refresh = vi.fn(() => new Promise<void>(resolve => { finish = resolve; }));
  await act(async () => root.render(<Clock seconds={10} refresh={refresh} />));
  expect(host.textContent).toBe("10");
  for (let left = 9; left >= 0; left--) { await advance(1000); expect(host.textContent).toBe(String(left)); }
  expect(refresh).toHaveBeenCalledTimes(1);
  await advance(15000); expect(refresh).toHaveBeenCalledTimes(1); expect(host.textContent).toBe("0");
  await act(async () => finish()); expect(host.textContent).toBe("10");
  await advance(1000); expect(host.textContent).toBe("9");
});

it("resets for manual loads and interval changes, and disabling cancels polling", async () => {
  const refresh = vi.fn(async () => {});
  await act(async () => root.render(<Clock seconds={10} refresh={refresh} />));
  await advance(3000); expect(host.textContent).toBe("7");
  await act(async () => root.render(<Clock seconds={10} refresh={refresh} pending />));
  expect(host.textContent).toBe("0"); await advance(15000); expect(refresh).not.toHaveBeenCalled();
  await act(async () => root.render(<Clock seconds={30} refresh={refresh} />));
  expect(host.textContent).toBe("30"); await advance(1000); expect(host.textContent).toBe("29");
  await act(async () => root.render(<Clock seconds={0} refresh={refresh} />));
  await advance(60000); expect(host.textContent).toBe("0"); expect(refresh).not.toHaveBeenCalled();
});

it("waits for a failed request before starting a full new countdown", async () => {
  let fail!: (error: Error) => void;
  const refresh = vi.fn(() => new Promise<void>((_resolve, reject) => { fail = reject; }));
  await act(async () => root.render(<Clock seconds={5} refresh={refresh} />));
  await advance(5000);
  await advance(20000);
  expect(refresh).toHaveBeenCalledTimes(1);
  expect(host.textContent).toBe("0");
  await act(async () => fail(new Error("Network failed")));
  expect(host.textContent).toBe("5");
  await advance(4000);
  expect(host.textContent).toBe("1");
  expect(refresh).toHaveBeenCalledTimes(1);
  await advance(1000);
  expect(refresh).toHaveBeenCalledTimes(2);
});

it("pauses in the background, refreshes once on return and cleans up on unmount", async () => {
  const refresh = vi.fn(async () => {});
  await act(async () => root.render(<Clock seconds={10} refresh={refresh} />));
  vi.mocked(Object.getOwnPropertyDescriptor(document, "hidden")!.get!).mockReturnValue(true);
  await act(async () => document.dispatchEvent(new Event("visibilitychange")));
  await advance(30000); expect(refresh).not.toHaveBeenCalled();
  vi.mocked(Object.getOwnPropertyDescriptor(document, "hidden")!.get!).mockReturnValue(false);
  await act(async () => document.dispatchEvent(new Event("visibilitychange")));
  expect(refresh).toHaveBeenCalledTimes(1); expect(host.textContent).toBe("10");
  await act(async () => root.render(null)); await advance(30000); expect(refresh).toHaveBeenCalledTimes(1);
});
