import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { i18n } from "../i18n/i18n";
import { AutoField } from "./auto-form";
import { FieldCaption } from "./FieldHelp";

let root: Root, host: HTMLDivElement;
beforeEach(async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  await i18n.changeLanguage("zh-CN");
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); });

it("opens field-specific help by keyboard without submitting or moving focus to the field", async () => {
  const submit = vi.fn();
  await act(async () => root.render(<form onSubmit={submit}><AutoField label="步骤 ID" help="同一任务内唯一；改名不会自动更新引用。"><input /></AutoField></form>));
  const trigger = host.querySelector<HTMLElement>('[role="button"]')!;
  await act(async () => { trigger.focus(); trigger.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
  expect(document.querySelector('[role="tooltip"]')?.textContent).toContain("同一任务内唯一");
  expect(document.activeElement).toBe(trigger);
  expect(submit).not.toHaveBeenCalled();
  await act(async () => trigger.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
  expect(document.querySelector('[role="tooltip"]')).toBeNull();
});

it("does not toggle a checkbox when its help is clicked", async () => {
  const change = vi.fn();
  await act(async () => root.render(<label><input type="checkbox" onChange={change} /><FieldCaption label="失败后继续" help="失败后仍继续后续步骤。" /></label>));
  await act(async () => host.querySelector<HTMLElement>('[role="button"]')!.click());
  expect(change).not.toHaveBeenCalled();
  expect(host.querySelector('input')!.checked).toBe(false);
  const captionId = host.querySelector('input')!.getAttribute('aria-labelledby')!;
  expect(document.getElementById(captionId)?.textContent).toBe("失败后继续");
  expect(document.querySelector('[role="tooltip"]')?.textContent).toContain("后续步骤");
});

it("uses control metadata as a fallback without exposing the entered value", async () => {
  await act(async () => root.render(<AutoField label="版本"><input placeholder="22 / lts/*" defaultValue="private-value" /></AutoField>));
  const trigger = host.querySelector<HTMLElement>('[role="button"]')!;
  expect(trigger).not.toBeNull();
  await act(async () => trigger.click());
  const content = document.querySelector('[role="tooltip"]')?.textContent;
  expect(content).toContain("22 / lts/*");
  expect(content).not.toContain("private-value");
});

it("removes generic input instructions in readonly mode while retaining business help", async () => {
  await act(async () => root.render(<AutoField label="版本"><input /></AutoField>));
  expect(host.querySelector('[role="button"]')).not.toBeNull();
  await act(async () => root.render(<AutoField label="版本"><input readOnly /></AutoField>));
  expect(host.querySelector('[role="button"]')).toBeNull();
  await act(async () => root.render(<AutoField label="版本"><input disabled /></AutoField>));
  expect(host.querySelector('[role="button"]')).toBeNull();
  await act(async () => root.render(<AutoField label="版本" help="该节点当前运行的版本。"><input disabled /></AutoField>));
  expect(host.querySelector('[role="button"]')).not.toBeNull();
});

it("does not create an invisible keyboard stop for a visually hidden label", async () => {
  await act(async () => root.render(<AutoField label="搜索" hideLabel help="搜索名称"><input /></AutoField>));
  expect(host.querySelector('[role="button"]')).toBeNull();
  expect(host.querySelector('input')!.getAttribute('aria-labelledby')).toBeTruthy();
});
