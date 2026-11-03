import { act } from "react";
import { createRoot,type Root } from "react-dom/client";
import { afterEach,beforeEach,expect,it } from "vitest";
import { commonText,i18n } from "../../shared/i18n/i18n";
import { CronEditor,cronError } from "./CronEditor";

let root: Root;
let host: HTMLDivElement;
let schedule: string;
let invalid: boolean;
const render = async () => act(async () => root.render(<CronEditor value={schedule} readOnly={false} onChange={next => { schedule = next; void render(); }} onValidityChange={value => { invalid = value; }} />));

beforeEach(() => {
  void i18n.changeLanguage("zh-CN");
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  schedule = "0 * * * *"; invalid = false;
});
it("changes schedule labels and validation language while preserving the expression and valid zero fields", async () => {
  schedule = "0 0 * * 0";
  await render();
  for (const locale of ["en-US", "zh-CN", "zh-TW", "ja-JP", "ko-KR", "ms-MY"]) {
    await act(async () => { await i18n.changeLanguage(locale); });
    expect(schedule).toBe("0 0 * * 0");
    expect(host.querySelector<HTMLInputElement>(`input[aria-label="${commonText("cron.expression")}"]`)?.value).toBe(schedule);
    expect(host.querySelector<HTMLInputElement>(`input[aria-label="${commonText("cron.hour")}"]`)?.value).toBe("0");
    expect(host.querySelector<HTMLSelectElement>(`select[aria-label="${commonText("cron.weekday")}"]`)?.value).toBe("0");
    expect(invalid).toBe(false);
  }
  schedule = "broken";
  await render();
  for (const locale of ["en-US", "zh-CN", "ja-JP"]) {
    await act(async () => { await i18n.changeLanguage(locale); });
    expect(host.querySelector('[role="alert"]')?.textContent).toBe(commonText("cron.fields"));
    expect(schedule).toBe("broken");
    expect(invalid).toBe(true);
  }
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

it("selects custom mode without rewriting its expression and retains it through edits and validation", async () => {
  await render();
  const select = host.querySelector<HTMLSelectElement>('[aria-label="执行周期类型"]')!;
  expect(select.querySelector<HTMLOptionElement>('option[value="custom"]')!.disabled).toBe(false);
  await act(async () => { select.value = "custom"; select.dispatchEvent(new Event("change", { bubbles: true })); });
  expect(select.value).toBe("custom");
  expect(schedule).toBe("0 * * * *");
  const input = host.querySelector<HTMLInputElement>('[aria-label="Cron 表达式"]')!;
  const type = async (value: string) => act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await type("0 0 * * *");
  expect(schedule).toBe("0 0 * * *");
  expect(select.value).toBe("custom");
  expect(invalid).toBe(false);
  await type("not cron");
  expect(schedule).toBe("not cron");
  expect(select.value).toBe("custom");
  expect(invalid).toBe(true);
  await type("15 8 * * MON-FRI");
  expect(schedule).toBe("15 8 * * MON-FRI");
  expect(invalid).toBe(false);
  await act(async () => { select.value = "weekly"; select.dispatchEvent(new Event("change", { bubbles: true })); });
  expect(schedule).toBe("0 0 * * 1");
  expect(select.value).toBe("weekly");
  await act(async () => { select.value = "custom"; select.dispatchEvent(new Event("change", { bubbles: true })); });
  expect(schedule).toBe("0 0 * * 1");
  expect(select.value).toBe("custom");
});
