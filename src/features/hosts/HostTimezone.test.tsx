import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it } from "vitest";
import { commonText, i18n } from "../../shared/i18n/i18n";
import { HostTimezone } from "./HostTimezone";
import { hostsForms, type Host as HostRow } from "./resource-config";

type Host = Partial<HostRow>;

let root: Root, host: HTMLDivElement;
beforeEach(async () => {
  await i18n.changeLanguage("zh-CN");
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
});
const text = (key: string, values?: Record<string, unknown>) => commonText(`resource-domain:timezone.${key}`, values);
const type = (input: HTMLInputElement, value: string) => {
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
  input.dispatchEvent(new Event("input", { bubbles: true }));
};

it("offers the Runner time zone only for Runner hosts, defaulting to automatic", () => {
  const config = hostsForms({ workspace: "ops", navigate: () => {} }).hosts!;
  expect(config.blank.timezone).toBe("");
  const field = config.sections.flatMap(section => section.fields).find(f => f.key === "timezone")!;
  expect(field.when!({ connection: "runner" })).toBe(true);
  expect(field.when!({ connection: "ssh" })).toBe(false);
});

it("shows the detected zone as automatic, checks names and reports a change", async () => {
  const changes: string[] = [];
  const validity: boolean[] = [];
  const render = (value: Host) =>
    act(async () => root.render(<HostTimezone host={value} readOnly={false} onChange={zone => changes.push(zone)} onValidityChange={blocked => validity.push(blocked)} />));
  const base: Host = { id: "7", agent_version: "0.8.0-alpha", os: "linux", detected_timezone: "Asia/Tokyo", timezone_supported: true, timezone: "" };
  await render(base);
  const input = host.querySelector<HTMLInputElement>(`input[aria-label="${text("label")}"]`)!;
  expect(input.placeholder).toBe(text("auto", { zone: "Asia/Tokyo" }));
  expect(input.placeholder).toContain("Asia/Tokyo");
  expect(host.textContent).toContain(text("hostHelp"));
  expect(host.textContent).not.toContain(text("upgrade"));
  expect(host.textContent).not.toContain(text("windows"));
  await act(async () => type(input, "Europe/Berlin"));
  expect(changes.at(-1)).toBe("Europe/Berlin");
  // An unknown name is flagged and blocks saving.
  await render({ ...base, timezone: "Mars/Olympus" });
  expect(host.textContent).toContain(text("invalid"));
  expect(validity.at(-1)).toBe(true);
  await render({ ...base, timezone: "Europe/Berlin" });
  expect(validity.at(-1)).toBe(false);
});

it("asks to upgrade an old Runner and warns about Windows tools", async () => {
  await act(async () => root.render(<HostTimezone host={{ id: "8", agent_version: "0.6.0-alpha", os: "windows", detected_timezone: "", timezone_supported: false, timezone: "" }} readOnly={false} onChange={() => {}} />));
  const input = host.querySelector<HTMLInputElement>(`input[aria-label="${text("label")}"]`)!;
  expect(input.placeholder).toBe(text("autoUnknown"));
  expect(host.querySelector('[role="status"]')?.textContent).toBe(text("upgrade"));
  expect(host.textContent).toContain(text("windows"));
  // A host that has not enrolled yet says nothing about upgrading.
  await act(async () => root.render(<HostTimezone host={{ connection: "runner", timezone: "" }} readOnly onChange={() => {}} />));
  expect(host.textContent).not.toContain(text("upgrade"));
  expect(host.textContent).toContain(text("autoUnknown"));
  expect(text("label")).toBe("时区");
});
