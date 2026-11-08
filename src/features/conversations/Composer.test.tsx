import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { Composer, type ComposerDraft } from "./Composer";
let host: HTMLDivElement, root: ReturnType<typeof createRoot>;
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  localStorage.clear();
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  localStorage.clear();
  vi.restoreAllMocks();
});
async function mount(send = vi.fn(async () => {})) {
  function Test() {
    const [draft, setDraft] = useState<ComposerDraft>({
      prompt: "hello",
      attachments: [],
    });
    return (
      <Composer
        endpoint="/workspaces/1/conversations/2"
        draft={draft}
        onChange={setDraft}
        onSend={send}
        disabled={false}
        busy={false}
        onStop={() => {}}
        group={false}
        agents={[]}
      />
    );
  }
  await act(async () => root.render(<Test />));
  return send;
}
it("defaults to Enter sending while modifiers, Shift and composition do not accidentally send", async () => {
  const send = await mount(),
    input = host.querySelector("textarea")!;
  await act(async () =>
    input.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "Enter",
        bubbles: true,
        isComposing: true,
      }),
    ),
  );
  await act(async () =>
    input.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "Enter",
        bubbles: true,
        shiftKey: true,
      }),
    ),
  );
  expect(send).not.toHaveBeenCalled();
  input.setSelectionRange(2, 2);
  await act(async () =>
    input.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "Enter",
        bubbles: true,
        ctrlKey: true,
        cancelable: true,
      }),
    ),
  );
  expect(input.value).toBe("he\nllo");
  expect(send).not.toHaveBeenCalled();
  await act(async () =>
    input.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "Enter",
        bubbles: true,
        cancelable: true,
      }),
    ),
  );
  expect(send).toHaveBeenCalledOnce();
});
it("restores Ctrl+Enter mode in the send button and persists upper grip height with keyboard access", async () => {
  localStorage.setItem("ui.chat.sendKey", "ctrl-enter");
  const send = await mount(),
    input = host.querySelector("textarea")!;
  expect(host.querySelector('button[type="submit"]')?.textContent).toContain(
    "Ctrl+Enter",
  );
  expect(localStorage.getItem("ui.chat.sendKey")).toBe("ctrl-enter");
  await act(async () =>
    input.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
    ),
  );
  expect(send).not.toHaveBeenCalled();
  await act(async () =>
    input.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "Enter",
        ctrlKey: true,
        bubbles: true,
        cancelable: true,
      }),
    ),
  );
  expect(send).toHaveBeenCalledOnce();
  const grip = host.querySelector('[role="slider"]')!;
  await act(async () =>
    grip.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "ArrowUp",
        bubbles: true,
        cancelable: true,
      }),
    ),
  );
  expect(input.style.height).toBe("136px");
  expect(localStorage.getItem("ui.chat.composerHeight")).toBe("136");
  expect(input.className).toContain("resize-none");
});
