import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { commonText, i18n } from "../../shared/i18n/i18n";
import { NotificationParameters } from "./NotificationParameters";

let host: HTMLDivElement, root: Root;
beforeEach(async () => {
  await i18n.changeLanguage("zh-CN");
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.stubGlobal("fetch", vi.fn(async () => ({ ok: true, status: 200, json: async () => ({ items: [], total: 0 }) })));
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); });

it("lets a workflow notification attach files from earlier nodes, and only there", async () => {
  const label = commonText("notifications:attachments");
  let value: Record<string, unknown> = { channel_id: "", attachments: [] };
  const render = (fileAttachments: boolean) => root.render(
    <NotificationParameters workspace="1" value={value} readOnly={false} onChange={next => { value = next; }} invalid={() => {}} fileAttachments={fileAttachments} />,
  );
  await act(async () => render(true));
  const input = host.querySelector<HTMLInputElement>(`input[aria-label="${label}"]`)!;
  expect(input).toBeTruthy();
  expect(host.textContent).toContain(commonText("notifications:attachmentsHelp"));
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "file, chart ,");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  expect(value.attachments).toEqual(["file", "chart"]);
  // Build notifications (no files) do not show the field.
  await act(async () => render(false));
  expect(host.querySelector(`input[aria-label="${label}"]`)).toBeNull();
});
