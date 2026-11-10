import { act, useState, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { AgentPrompt } from "./AgentPrompt";
import messages from "../../shared/i18n/locales/agents.json";

vi.mock("../../shared/ui/select", () => ({
  Select: ({ children, value, onValueChange }: { children: ReactNode; value: string; onValueChange: (value: string) => void }) => <select value={value} onChange={event => onValueChange(event.target.value)}><option value="" />{children}</select>,
  SelectTrigger: () => null,
  SelectValue: () => null,
  SelectContent: ({ children }: { children: ReactNode }) => <>{children}</>,
  SelectItem: ({ children, value }: { children: ReactNode; value: string }) => <option value={value}>{children}</option>,
}));
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });

it("previews without overwriting, appends to the latest draft, explicitly replaces and supports empty prompts", async () => {
  const host = document.createElement("div"), root = createRoot(host);
  document.body.append(host);
  function Editor() {
    const [draft, setDraft] = useState({ system_prompt: "My existing instructions" });
    return <AgentPrompt draft={draft} readOnly={false} onPatch={setDraft} />;
  }
  try {
    await act(() => root.render(<Editor />));
    const select = host.querySelector("select")!;
    const textarea = host.querySelector("textarea")!;
    await act(() => { select.value = "developer"; select.dispatchEvent(new Event("change", { bubbles: true })); });
    expect(textarea.value).toBe("My existing instructions");
    const preview = host.querySelector("p.whitespace-pre-wrap")!.textContent!;
    expect(preview.length).toBeGreaterThan(80);
    await act(() => host.querySelectorAll("button")[0]!.click());
    expect(textarea.value).toBe(`My existing instructions\n\n${preview}`);
    await act(() => host.querySelectorAll("button")[1]!.click());
    expect(textarea.value).toBe(preview);
    await act(() => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(textarea, "");
      textarea.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect(textarea.value).toBe("");
    expect(host.querySelectorAll("button").length).toBe(1);
    await act(() => host.querySelector("button")!.click());
    expect(textarea.value).toBe(preview);
  } finally { await act(() => root.unmount()); host.remove(); }
});

it("shows saved instructions without template actions in read-only mode", async () => {
  const host = document.createElement("div"), root = createRoot(host), onPatch = vi.fn();
  try {
    await act(() => root.render(<AgentPrompt draft={{ system_prompt: "Saved prompt" }} readOnly onPatch={onPatch} />));
    expect(host.querySelector("textarea")!.value).toBe("Saved prompt");
    expect(host.querySelector("textarea")!.readOnly).toBe(true);
    expect(host.querySelector("select,button")).toBeNull();
    expect(onPatch).not.toHaveBeenCalled();
  } finally { await act(() => root.unmount()); }
});

it("provides six complete localized versions of every template and action", () => {
  const entries = Object.entries(messages).filter(([key]) => /promptTemplate|PromptTemplate/.test(key));
  expect(entries.length).toBe(18);
  for (const [, translations] of entries) {
    expect(translations).toHaveLength(6);
    expect(translations.every(value => value.trim().length > 0)).toBe(true);
  }
});
