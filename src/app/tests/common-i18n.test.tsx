import { act } from "react";
import { createRoot,type Root } from "react-dom/client";
import { afterEach,beforeEach,expect,it,vi } from "vitest";
import { AutoForm } from "../../shared/forms/auto-form";
import { commonText,configText,i18n } from "../../shared/i18n/i18n";
import { setLocale,type Locale } from "../../shared/i18n/locale";
import catalogs from "../../shared/i18n/locales/common-domain.json";
import type { FormConfig,ListConfig } from "../../shared/resources/types";
import { ResourceForm } from "../resources/ResourceForm";
import { ResourceTable } from "../resources/ResourceTable";
import { GroupSidebar } from "../pages/GroupSidebar";
import { Invitation } from "../../features/workspaces/Invitation";

let root: Root, host: HTMLDivElement;
it("resolves namespace-qualified configuration messages containing spaces", async () => {
  await i18n.changeLanguage("en-US");
  expect(configText("合并 merge", "insight-domain:合并 merge")).toBe("Merges (merge)");
  await i18n.changeLanguage("zh-CN");
  expect(configText("合并 merge", "insight-domain:合并 merge")).toBe("合并 merge");
});
const language = async (locale: Locale) => act(async () => { setLocale(locale); await i18n.changeLanguage(locale); });
beforeEach(async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { callback(0); return 0; });
  history.replaceState({}, "", "/en-US/workspaces/ops/tags/new");
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  await language("en-US");
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); });
const fill = async (input: HTMLInputElement, value: string) => act(async () => {
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
  input.dispatchEvent(new Event("input", { bubbles: true }));
});

it("translates the complete invitation flow and updates existing feedback when the language changes", async () => {
  vi.stubGlobal("fetch", vi.fn(async () => new Response(JSON.stringify({code:"fixture-invite"}), { headers: { "Content-Type": "application/json" } })));
  vi.spyOn(window, "confirm").mockReturnValue(true);
  await act(async () => root.render(<Invitation workspace="ops" />));
  for (const locale of ["en-US", "zh-CN", "zh-TW", "ja-JP", "ko-KR", "ms-MY"] as const) {
    await language(locale);
    expect(host.querySelector("section")?.getAttribute("aria-label")).toBe(commonText("invitation.title"));
    expect(host.textContent).toContain(commonText("invitation.help"));
    const view = [...host.querySelectorAll("button")].find(button => button.textContent === commonText("invitation.view"))!;
    await act(async () => view.click());
    expect(host.querySelector<HTMLInputElement>("input")?.value).toBe("fixture-invite");
    const rotate = [...host.querySelectorAll("button")].find(button => button.textContent === commonText("invitation.rotate"))!;
    await act(async () => rotate.click());
    expect(window.confirm).toHaveBeenLastCalledWith(commonText("invitation.rotateConfirm"));
    expect(host.querySelector('[role="status"]')?.textContent).toBe(commonText("invitation.rotated"));
  }
  await language("en-US");
  expect(host.querySelector('[role="status"]')?.textContent).toBe(commonText("invitation.rotated"));
  expect(host.textContent).not.toMatch(/[\u4e00-\u9fff]/);
  vi.mocked(window.confirm).mockRestore();
});

it("uses translated All and Ungrouped labels without concatenating a resource noun in six languages", async () => {
  vi.stubGlobal("fetch", vi.fn(async () => new Response(JSON.stringify({items:[],total:0,page:1,page_size:200}))));
  for (const locale of ["en-US", "zh-CN", "zh-TW", "ja-JP", "ko-KR", "ms-MY"] as const) {
    await language(locale);
    for (const resource of ["host-groups", "pipeline-groups"] as const) {
      await act(async () => root.render(<GroupSidebar workspace="ops" permissions={new Set()} resource={resource} noun="RESOURCE" active="" onSelect={() => {}} />));
      const buttons = [...host.querySelectorAll('button')].map(button => button.textContent);
      expect(buttons).toContain(commonText("group.all"));
      expect(buttons).toContain(commonText("group.ungrouped"));
      expect(buttons.some(text => text?.includes("RESOURCE"))).toBe(false);
    }
  }
});

it("contains six complete actual catalogs with identical interpolation contracts", () => {
  const english = catalogs["en-US"];
  for (const catalog of Object.values(catalogs)) {
    expect(Object.keys(catalog).sort()).toEqual(Object.keys(english).sort());
    for (const [key, text] of Object.entries(catalog)) {
      expect(text.trim()).not.toBe("");
      expect(text.match(/\{\{[^}]+\}\}/g)?.sort() ?? []).toEqual(english[key as keyof typeof english].match(/\{\{[^}]+\}\}/g)?.sort() ?? []);
    }
  }
});

it("translates stable form metadata EN/ZH/JA without losing unsaved draft or group identity", async () => {
  const config: FormConfig<Record<string, unknown>> = {
    resource: "tags", heading: () => "tag", blank: {}, sections: [
      { title: "Raw section", titleKey: "common.basic", group: "stable-group", groupKey: "common.basic", fields: [
        { key: "name", label: "Raw name", labelKey: "common.name", help: "Raw help", helpKey: "group.help", type: "text", required: true },
      ] },
    ],
  };
  await act(async () => root.render(<ResourceForm config={config} workspace="ops" permissions={new Set(["tags.create"])} />));
  const input = host.querySelector<HTMLInputElement>('input[name="name"]')!;
  await fill(input, "Unsaved draft");
  for (const locale of ["zh-CN", "ja-JP", "en-US"] as const) {
    await language(locale);
    expect(host.textContent).toContain(commonText("group.help"));
    expect(host.textContent).toContain(commonText("common.basic"));
    expect(host.querySelector<HTMLInputElement>('input[name="name"]')!.value).toBe("Unsaved draft");
    expect(config.sections[0]!.group).toBe("stable-group");
  }
});

it("updates validation language while retaining an invalid email draft", async () => {
  const save = vi.fn();
  await act(async () => root.render(<AutoForm fields={{ email: { name: "Email", type: "email", required: true } }} onValuesSubmit={save} />));
  await fill(host.querySelector<HTMLInputElement>('input[name="email"]')!, "invalid@");
  await act(async () => host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
  expect(host.textContent).toContain("Please enter a valid email address");
  for (const locale of ["zh-CN", "ja-JP"] as const) {
    await language(locale);
    expect(host.textContent).toContain(commonText("form.email", { name: "Email" }));
    expect(host.querySelector<HTMLInputElement>('input[name="email"]')!.value).toBe("invalid@");
  }
  expect(save).not.toHaveBeenCalled();
});

it("keeps row action semantics independent of translated labels", async () => {
  type Row = { id: string; name: string };
  const execute = vi.fn(async () => {});
  vi.stubGlobal("fetch", vi.fn(async () => ({ ok: true, json: async () => ({ items: [{ id: "7", name: "User-owned name" }], total: 1, page: 1, page_size: 20 }) })));
  const config: ListConfig<Row> = { resource: "tags", rowKey: row => row.id, columns: [{ key: "name", label: "Raw name", labelKey: "common.name" }], actions: [{ label: "查看", labelKey: "common.view", action: execute }], toolbar: { create: false, import: false, export: false, refresh: [0] } };
  await act(async () => root.render(<ResourceTable config={config} workspace="ops" permissions={new Set()} />));
  for (const locale of ["ja-JP", "zh-CN", "en-US"] as const) {
    await language(locale);
    const button = host.querySelector<HTMLButtonElement>(`button[aria-label="${commonText("common.view")}"]`)!;
    expect(button).not.toBeNull();
    await act(async () => button.click());
    expect(host.textContent).toContain("User-owned name");
    expect(config.actions![0]!.label).toBe("查看");
  }
  expect(execute).toHaveBeenCalledTimes(3);
});
