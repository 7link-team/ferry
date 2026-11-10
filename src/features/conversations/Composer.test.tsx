import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { Composer, type ComposerDraft } from "./Composer";
import {
  mentionAt,
  mentionRanges,
  selectMentionDeletion,
} from "./MentionSuggestions";
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
  vi.unstubAllGlobals();
});

it("recognizes a caret mention in the middle of a message, excluding emails and text selections", () => {
  expect(mentionAt("before @Al after", 10)).toEqual({
    start: 7,
    end: 10,
    query: "Al",
  });
  expect(mentionAt("x@example.com", 6)).toBeNull();
  expect(mentionAt("@Alice", 2, 5)).toBeNull();
  expect(mentionAt("@Alice done", 11)).toBeNull();
  expect(mentionAt("first\n@张", 8)?.query).toBe("张");
});

async function mountGroup(prompt = "", extra = 0) {
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      disconnect() {}
    },
  );
  const send = vi.fn(async () => {}),
    drafts: ComposerDraft[] = [];
  function Test() {
    const [draft, setDraft] = useState<ComposerDraft>({
      prompt,
      attachments: [],
    });
    return (
      <Composer
        endpoint="/workspaces/1/conversations/2"
        draft={draft}
        onChange={(value) => {
          drafts.push(value);
          setDraft(value);
        }}
        onSend={send}
        disabled={false}
        busy={false}
        onStop={() => {}}
        group
        agents={[
          { id: "off", name: "Offline", state: "disabled" },
          { id: "ai", name: "Assistant", state: "active" },
        ]}
        people={[
          { id: "alice", name: "Alice" },
          { id: "alex", name: "Alex" },
          ...Array.from({ length: extra }, (_, i) => ({
            id: `p${i}`,
            name: `Person ${i}`,
          })),
        ]}
      />
    );
  }
  await act(async () => root.render(<Test />));
  return { input: host.querySelector("textarea")!, send, drafts };
}
async function write(
  input: HTMLTextAreaElement,
  value: string,
  caret = value.length,
  inputType = "insertText",
) {
  await act(async () => {
    Object.getOwnPropertyDescriptor(
      HTMLTextAreaElement.prototype,
      "value",
    )!.set!.call(input, value);
    input.setSelectionRange(caret, caret);
    input.dispatchEvent(new InputEvent("input", { bubbles: true, inputType }));
  });
}
async function key(
  input: HTMLTextAreaElement,
  name: string,
  composing = false,
) {
  await act(async () =>
    input.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: name,
        bubbles: true,
        cancelable: true,
        isComposing: composing,
      }),
    ),
  );
}
it("filters at the caret, navigates and inserts a person without sending or losing following text", async () => {
  const { input, send, drafts } = await mountGroup();
  await write(input, "before @Al after", 10);
  expect(document.querySelectorAll('[role="option"]')).toHaveLength(2);
  await key(input, "ArrowDown");
  await key(input, "Enter");
  expect(input.value).toBe("before @Alex after");
  expect(drafts.at(-1)?.mentions).toEqual(["alex"]);
  expect(send).not.toHaveBeenCalled();
  expect(document.querySelector("[data-mention-popup]")).toBeNull();
});
it("Backspace selects the whole mention and its separator; deleting text clears the selected recipient", async () => {
  const { input, drafts } = await mountGroup();
  await write(input, "@Ass");
  await key(input, "Enter");
  input.setSelectionRange(input.value.length, input.value.length);
  await key(input, "Backspace");
  expect([input.selectionStart, input.selectionEnd]).toEqual([
    0,
    "@Assistant ".length,
  ]);
  await write(input, "", 0, "deleteContentBackward");
  expect(drafts.at(-1)?.targets).toEqual([]);
  expect(
    document.querySelector(
      '[data-conversation-composer] button[title="Assistant"]',
    ),
  ).toBeNull();
});
it("Delete inside a mention expands to its full name without deleting surrounding text", async () => {
  const { input, drafts } = await mountGroup();
  await write(input, "before @Ali after", 11);
  await key(input, "Enter");
  input.setSelectionRange(9, 9);
  await key(input, "Delete");
  expect([input.selectionStart, input.selectionEnd]).toEqual([7, 14]);
  await write(input, "before after", 7, "deleteContentForward");
  expect(drafts.at(-1)?.mentions).toEqual([]);
  expect(input.value).toBe("before after");
});
it("cutting one of repeated mentions keeps its target; removing the last clears it and undo/redo restores coherently", async () => {
  const { input, drafts } = await mountGroup();
  await write(input, "@Ass");
  await key(input, "Enter");
  await write(input, "@Assistant @Assistant ");
  await write(input, "@Assistant ", 11, "deleteByCut");
  expect(drafts.at(-1)?.targets).toEqual(["ai"]);
  await write(input, "", 0, "deleteByCut");
  expect(drafts.at(-1)?.targets).toEqual([]);
  await write(input, "@Assistant ", 11, "historyUndo");
  expect(drafts.at(-1)?.targets).toEqual(["ai"]);
  await write(input, "", 0, "historyRedo");
  expect(drafts.at(-1)?.targets).toEqual([]);
});
it("matches full names including spaces and expands partial selection; emails and name prefixes stay ordinary text", () => {
  const alice = { id: "1", name: "Alice", human: true, state: "active" },
    full = { ...alice, id: "2", name: "Alice Smith" };
  expect(
    mentionRanges("x@Alice @Alice Smith, @Alice!", [alice, full]).map(
      (r) => r.option.id,
    ),
  ).toEqual(["2", "1"]);
  const input = document.createElement("textarea");
  input.value = "before @Alice Smith after";
  input.setSelectionRange(10, 13);
  selectMentionDeletion(input, [full], true);
  expect([input.selectionStart, input.selectionEnd]).toEqual([7, 19]);
  input.value = "@AliceExtra ";
  input.setSelectionRange(5, 5);
  selectMentionDeletion(input, [alice], true);
  expect([input.selectionStart, input.selectionEnd]).toEqual([5, 5]);
});
it("keeps a bounded list, skips unavailable agents, and Escape dismisses without sending", async () => {
  const { input, send } = await mountGroup("", 70);
  await write(input, "@");
  expect(document.querySelectorAll('[role="option"]')).toHaveLength(74);
  expect(document.querySelector('[role="listbox"]')?.className).toContain(
    "overflow-y-auto",
  );
  await key(input, "ArrowDown");
  expect(input.getAttribute("aria-activedescendant")).toBe(
    "conversation-mention-2",
  );
  await key(input, "Escape");
  expect(document.querySelector("[data-mention-popup]")).toBeNull();
  expect(send).not.toHaveBeenCalled();
  await write(input, "@unmatched");
  expect(document.querySelectorAll('[role="option"]')).toHaveLength(0);
  expect(
    document.querySelector('[role="listbox"] [role="status"]'),
  ).toBeTruthy();
  await key(input, "Enter");
  expect(send).not.toHaveBeenCalled();
});
it("does not choose on IME Enter and reselecting a mention does not remove its target", async () => {
  const { input, drafts } = await mountGroup();
  await write(input, "@Ass");
  await key(input, "Enter", true);
  expect(input.value).toBe("@Ass");
  await key(input, "Enter");
  expect(drafts.at(-1)?.targets).toEqual(["ai"]);
  await write(input, "@Assistant @Ass");
  await key(input, "Enter");
  expect(drafts.at(-1)?.targets).toEqual(["ai"]);
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
