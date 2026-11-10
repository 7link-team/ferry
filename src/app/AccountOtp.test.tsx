import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { AccountOtp, OtpLogin } from "./AccountOtp";
import { commonText as t, i18n } from "../shared/i18n/i18n";

let root: Root, host: HTMLDivElement;
const request = vi.fn();
beforeEach(async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  await i18n.changeLanguage("en-US");
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  request.mockReset(); vi.stubGlobal("fetch", request);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); });
function reply(value: unknown, status = 200) { return {ok:status<400,status,json:async()=>value}; }
async function click(text: string) {
  const button = [...document.querySelectorAll("button")].find(value => value.textContent === text)!;
  expect(button).toBeTruthy(); await act(async () => button.click());
}
async function input(name: string, value: string) {
  const element = document.querySelector<HTMLInputElement>(`input[name="${name}"]`)!;
  await act(async () => { Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,"value")!.set!.call(element,value); element.dispatchEvent(new Event("input",{bubbles:true})); });
}
async function submit(id: string) { await act(async () => document.getElementById(id)!.dispatchEvent(new Event("submit",{bubbles:true,cancelable:true}))); }

it("keeps password verification separate from OTP errors and successful authentication", async () => {
  const ready = vi.fn(), cancel = vi.fn();
  request.mockResolvedValueOnce(reply({error:"Invalid code"},400)).mockResolvedValueOnce(reply({id:"1"}));
  await act(async () => root.render(<OtpLogin token="private-second-factor-token" onReady={ready} onCancel={cancel} />));
  await input("code","123456"); await submit("otp-login");
  expect(ready).not.toHaveBeenCalled(); expect(document.querySelector('[role="alert"]')?.textContent).toBe("Invalid code");
  expect(document.querySelector<HTMLInputElement>('input[name="code"]')!.value).toBe("");
  await click(t("otp:useRecovery")); await input("code","12345678-12345678-12345678-12345678"); await submit("otp-login");
  expect(ready).toHaveBeenCalledOnce();
  expect(JSON.parse(request.mock.calls[1]![1].body)).toEqual({token:"private-second-factor-token",code:"12345678-12345678-12345678-12345678",use_recovery:true});
  expect(location.href).not.toContain("private-second-factor-token");
});

it("expires exhausted challenges and offers cancellation instead of trapping login", async () => {
  const cancel = vi.fn(); request.mockResolvedValue(reply({error:"Expired"},410));
  await act(async () => root.render(<OtpLogin token="expired" onReady={vi.fn()} onCancel={cancel} />));
  await input("code","123456"); await submit("otp-login");
  expect(document.querySelector<HTMLButtonElement>('button[form="otp-login"]')!.disabled).toBe(true);
  await click(t("otp:cancel")); expect(cancel).toHaveBeenCalledOnce();
});

it("does not offer new enrollment when disabled but preserves management for a bound account", async () => {
  request.mockResolvedValue(reply({enabled:false,enrollment_enabled:false,recovery_remaining:0}));
  await act(async () => root.render(<AccountOtp />));
  expect(host.textContent).toContain(t("otp:unavailable")); expect(host.querySelector("button")).toBeNull();
  await act(async () => root.render(null));
  request.mockResolvedValue(reply({enabled:true,enrollment_enabled:false,recovery_remaining:8}));
  await act(async () => root.render(<AccountOtp />));
  expect(host.textContent).toContain(t("otp:remaining",{count:8})); await click(t("otp:remove"));
  expect(document.querySelector('[role="dialog"]')?.textContent).toContain(t("otp:removeHelp"));
  expect(document.querySelector('input[name="current_password"]')).toBeTruthy();
});

it("reveals recovery codes only after confirmation and requires acknowledgement before closing", async () => {
  const outerSubmit = vi.fn((event: React.FormEvent) => event.preventDefault());
  request.mockImplementation(async (url: string) => reply(url.endsWith("/enrollment/confirm") ? {recovery_codes:["abcd1234-abcd1234-abcd1234-abcd1234"]} : url.endsWith("/enrollment") ? {secret:"BASE32KEY",qr_svg:'<svg xmlns="http://www.w3.org/2000/svg" />'} : {enabled:false,enrollment_enabled:true,recovery_remaining:0}));
  await act(async () => root.render(<form onSubmit={outerSubmit}><AccountOtp /></form>)); await click(t("otp:bind"));
  await input("current_password","test-password"); await submit("otp-manage");
  expect(document.querySelector('img')?.getAttribute("src")).toMatch(/^data:image\/svg\+xml/);
  expect(document.body.textContent).not.toContain("abcd1234");
  await input("code","123456"); await submit("otp-manage");
  expect(document.querySelector("code")?.textContent).toBe("abcd1234-abcd1234-abcd1234-abcd1234");
  expect([...document.querySelectorAll("button")].find(value => value.textContent === t("otp:close"))!.disabled).toBe(true);
  await act(async () => document.querySelector<HTMLInputElement>('input[type="checkbox"]')!.click());
  await click(t("otp:close")); expect(document.querySelector('[role="dialog"]')).toBeNull();
  expect(outerSubmit).not.toHaveBeenCalled();
});
