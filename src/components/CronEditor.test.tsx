import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it } from "vitest";
import { CronEditor, cronError } from "./CronEditor";

let root: Root;
let host: HTMLDivElement;
let schedule: string;
let invalid: boolean;
const render = async () => act(async () => root.render(<CronEditor value={schedule} readOnly={false} onChange={next => { schedule = next; void render(); }} onValidityChange={value => { invalid = value; }} />));

beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  schedule = "0 * * * *"; invalid = false;
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); });

it("converts a cycle to cron and parses a changed expression back into the cycle form", async () => {
  await render();
  const cycle = host.querySelector<HTMLSelectElement>('[aria-label="执行周期类型"]')!;
  expect(cycle.value).toBe("hours");
  await act(async () => { cycle.value = "weekly"; cycle.dispatchEvent(new Event("change", { bubbles: true })); });
  expect(schedule).toBe("0 0 * * 1");
  const cron = host.querySelector<HTMLInputElement>('[aria-label="Cron 表达式"]')!;
  await act(async () => { Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(cron, "30 9 15 * *"); cron.dispatchEvent(new Event("input", { bubbles: true })); });
  expect(host.querySelector<HTMLSelectElement>('[aria-label="执行周期类型"]')!.value).toBe("monthly");
  expect(host.querySelector<HTMLInputElement>('[aria-label="日期"]')!.value).toBe("15");
  expect(invalid).toBe(false);
});

it("rejects malformed fields before saving while retaining custom valid expressions", async () => {
  expect(cronError("61 9 * * *")).not.toBe("");
  expect(cronError("*/5 * * * *")).toBe("");
  expect(cronError("0 15 9 * * MON-FRI")).toBe("");
  schedule = "61 9 * * *";
  await render();
  expect(host.querySelector('[role="alert"]')?.textContent).toContain("无效");
  expect(invalid).toBe(true);
});
