import { StrictMode, act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { HostUpload } from "./HostUpload";
vi.mock("../../shared/i18n/i18n", () => ({
  hostRepositoryText: (key: string) => key,
}));
vi.mock("../../shared/i18n/locale", () => ({
  localeHeaders: (headers: object) => headers,
}));
vi.mock("../../shared/api/api", () => ({
  api: vi.fn(),
  localizedFetch: vi.fn(),
  errorText: (error: Error) => error.message,
}));
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
class Request {
  static all: Request[] = [];
  upload: { onprogress?: (event: { loaded: number }) => void } = {};
  onload?: () => void;
  onabort?: () => void;
  onerror?: () => void;
  status = 204;
  responseText = "";
  body?: File;
  constructor() {
    Request.all.push(this);
  }
  open = vi.fn();
  setRequestHeader = vi.fn();
  send(file: File) {
    this.body = file;
  }
  abort() {
    this.onabort?.();
  }
}
afterEach(() => {
  vi.unstubAllGlobals();
  Request.all = [];
});
it("requires confirmation, reports each file, waits for server ACK, guards close and retries only failures", async () => {
  vi.stubGlobal("XMLHttpRequest", Request);
  const host = document.createElement("div"),
    root = createRoot(host),
    complete = vi.fn();
  document.body.append(host);
  const file = new File(["x".repeat(100)], "one.txt"),
    next = new File(["x"], "two.txt");
  const selection = Promise.resolve([
    { path: "one.txt", file },
    { path: "two.txt", file: next },
  ]);
  const click = async (key: string) =>
    act(() =>
      (
        Array.from(document.querySelectorAll("button")).find(
          (b) => b.textContent === key,
        ) as HTMLButtonElement
      ).click(),
    );
  try {
    await act(() =>
      root.render(
        <StrictMode>
          <HostUpload
            endpoint="/api/workspaces/1/nodes/2/files"
            directory="/test"
            initialSelection={selection}
            onClose={() => {}}
            onComplete={complete}
          />
        </StrictMode>,
      ),
    );
    expect(Request.all).toHaveLength(0);
    await click("host-files.uploadConfirm");
    expect(Request.all).toHaveLength(1);
    expect(Request.all[0]!.body).toBe(file);
    const guard = new Event("beforeunload", { cancelable: true });
    window.dispatchEvent(guard);
    expect(guard.defaultPrevented).toBe(true);
    await act(() => Request.all[0]!.upload.onprogress?.({ loaded: 100 }));
    expect(document.body.textContent).toContain(
      "host-files.uploadState.confirming",
    );
    expect(complete).not.toHaveBeenCalled();
    await act(() => Request.all[0]!.onload?.());
    expect(Request.all).toHaveLength(2);
    Request.all[1]!.status = 403;
    Request.all[1]!.responseText = JSON.stringify({
      error: "Permission denied: /test/two.txt",
    });
    await act(() => Request.all[1]!.onload?.());
    expect(document.body.textContent).toContain(
      "Permission denied: /test/two.txt",
    );
    await click("host-files.uploadRetry");
    expect(Request.all).toHaveLength(3);
    expect(Request.all[2]!.body).toBe(next);
    await act(() => Request.all[2]!.onload?.());
    expect(
      document
        .querySelector(
          '[aria-label="host-files.uploadOverall"] [role="progressbar"]',
        )
        ?.getAttribute("aria-valuenow"),
    ).toBe("100");
    const released = new Event("beforeunload", { cancelable: true });
    window.dispatchEvent(released);
    expect(released.defaultPrevented).toBe(false);
  } finally {
    await act(() => root.unmount());
    host.remove();
  }
});
