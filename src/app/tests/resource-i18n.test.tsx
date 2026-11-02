import { execFileSync } from "node:child_process";
import { mkdtempSync,rmSync,writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { act } from "react";
import { createRoot,type Root } from "react-dom/client";
import { afterEach,beforeEach,expect,it,vi } from "vitest";
import { commonText,i18n } from "../../shared/i18n/i18n";
import { setLocale,type Locale } from "../../shared/i18n/locale";
import catalogs from "../../shared/i18n/locales/resource-domain.json";
import { Platform } from "../Platform";
import { RunnerConnection,runnerInstallCommand,runnerUninstallCommand } from "../pages/RunnerConnection";
import { formConfigs } from "../resources";
import { ResourceForm } from "../resources/ResourceForm";

const locales = Object.keys(catalogs) as Locale[];
let root: Root, host: HTMLDivElement;
const language = async (locale: Locale) => act(async () => { setLocale(locale); await i18n.changeLanguage(locale); });
const fill = async (input: HTMLInputElement, value: string) => act(async () => {
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
  input.dispatchEvent(new Event("input", { bubbles: true }));
});
beforeEach(async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  history.replaceState({}, "", "/en-US/workspaces/ops/nodes/new");
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  await language("en-US");
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); });

it("supplies six complete resource catalogs with consistent interpolation parameters", () => {
  const english = catalogs["en-US"];
  for (const catalog of Object.values(catalogs)) {
    expect(Object.keys(catalog).sort()).toEqual(Object.keys(english).sort());
    for (const [key, text] of Object.entries(catalog)) {
      expect(text.trim(), key).not.toBe("");
      expect(text.match(/\{\{[^}]+\}\}/g)?.sort() ?? [], key).toEqual(english[key as keyof typeof english].match(/\{\{[^}]+\}\}/g)?.sort() ?? []);
    }
  }
});

it("updates real host configuration labels in six languages while keeping its draft", async () => {
  vi.stubGlobal("fetch", vi.fn(async () => ({ ok: true, json: async () => ({ items: [], total: 0, page: 1, page_size: 200 }) })));
  const config = formConfigs("ops").hosts!;
  config.blank = { ...config.blank, connection: "ssh" };
  await act(async () => root.render(<ResourceForm config={config} workspace="ops" permissions={new Set(["hosts.create"])} />));
  await fill(host.querySelector<HTMLInputElement>('input[name="name"]')!, "Draft host");
  await fill(host.querySelector<HTMLInputElement>('input[name="address"]')!, "draft.example.invalid");
  for (const locale of locales) {
    await language(locale);
    expect(host.textContent).toContain(commonText("resource-domain:host.port"));
    expect(host.querySelector<HTMLInputElement>('input[name="name"]')!.value).toBe("Draft host");
    expect(host.querySelector<HTMLInputElement>('input[name="address"]')!.value).toBe("draft.example.invalid");
  }
});

it("keeps the enrollment token, server draft and install scope when language changes", async () => {
  const fetcher = vi.fn(async (_url: string, init: RequestInit = {}) => ({ ok: true, status: 200, json: async () => init.method === "POST"
    ? { id: "t1", token: "draft-token", server: "https://example.com", expires_at: Date.now() + 900000 }
    : { id: "t1", state: "pending", host_state: "", host_id: "", used_at: 0 } }));
  vi.stubGlobal("fetch", fetcher);
  await act(async () => root.render(<RunnerConnection workspace="ops" inline />));
  await fill(host.querySelector<HTMLInputElement>('input')!, "https://draft.example.invalid");
  await act(async () => [...host.querySelectorAll("button")].find(button => button.textContent === commonText("resource-domain:runner.generate"))!.click());
  await fill(host.querySelector<HTMLInputElement>('input')!, "https://draft.example.invalid");
  const scope = [...host.querySelectorAll<HTMLButtonElement>('[role="tab"]')].find(tab => tab.textContent === commonText("resource-domain:runner.currentUser"))!;
  await act(async () => scope.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 })));
  const before = fetcher.mock.calls.length;
  for (const locale of locales) {
    await language(locale);
    expect(host.textContent).toContain(commonText("resource-domain:runner.waitHost"));
    expect(host.querySelector<HTMLInputElement>('input')!.value).toBe("https://draft.example.invalid");
    const command = host.querySelector<HTMLTextAreaElement>(`textarea[aria-label="${commonText("resource-domain:runner.installCommand")}"]`)!.value;
    expect(command).toContain("draft-token");
    expect(command).toContain("--user");
  }
  expect(fetcher.mock.calls.length).toBe(before);
  expect(fetcher.mock.calls.filter(([, init]) => init?.method === "POST")).toHaveLength(1);
});

it("translates platform policy settings without resetting an unsaved switch", async () => {
  history.replaceState({}, "", "/en-US/platform/settings/1/edit");
  const fetcher = vi.fn(async () => ({ ok: true, status: 200, json: async () => ({ id: "1", version: 1, runner_auto_update: false, registration_enabled: false }) }));
  vi.stubGlobal("fetch", fetcher);
  await act(async () => root.render(<Platform identity={{ user: { id: "1", name: "Operator", email: "operator@example.invalid", is_superadmin: true }, workspaces: [] }} reload={() => {}} />));
  await act(async () => host.querySelector<HTMLButtonElement>('[role="checkbox"]')!.click());
  const before = fetcher.mock.calls.length;
  for (const locale of locales) {
    await language(locale);
    expect(host.textContent).toContain(commonText("resource-domain:runner.defaultAutoHelp"));
    expect(host.querySelector('[role="checkbox"]')!.getAttribute("aria-checked")).toBe("true");
  }
  expect(fetcher.mock.calls.length).toBe(before);
});

it.skipIf(process.platform !== "win32")("parses every localized Windows command and its embedded payloads with native PowerShell", async () => {
  const scripts: string[] = [];
  for (const locale of locales) {
    await language(locale);
    for (const scope of ["system", "user"] as const) {
      const uninstall = runnerUninstallCommand("windows", scope);
      scripts.push(runnerInstallCommand("windows", "https://example.com", "token'with$quotes", scope), uninstall);
      scripts.push(...[...uninstall.matchAll(/-EncodedCommand'?,? '?([A-Za-z0-9+/=]{20,})/g)].map(match => Buffer.from(match[1]!, "base64").toString("utf16le")));
      expect(uninstall).toContain(catalogs[locale]["runner.command.registrationRemoved"].split("{{registration}}")[0]!);
    }
  }
  const folder = mkdtempSync(join(tmpdir(), "runner-locale-parse-"));
  const fixture = join(folder, "commands.json");
  writeFileSync(fixture, "\ufeff" + JSON.stringify(scripts));
  const harness = `$ErrorActionPreference = 'Stop'
foreach ($script in (Get-Content -Raw -LiteralPath '${fixture.replaceAll("'", "''")}' | ConvertFrom-Json)) {
  $errors = $null
  [void][Management.Automation.Language.Parser]::ParseInput($script, [ref]$null, [ref]$errors)
  if ($errors.Count) { throw ($errors | Out-String) }
}
Write-Output 'PASS all six languages'`;
  try {
    expect(execFileSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-EncodedCommand", Buffer.from(harness, "utf16le").toString("base64")], { encoding: "utf8", windowsHide: true })).toContain("PASS all six languages");
  } finally { rmSync(folder, { recursive: true, force: true }); }
});
