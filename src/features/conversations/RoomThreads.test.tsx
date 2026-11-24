import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import * as apiModule from "../../shared/api/api";
import { conversationText as t } from "./resource-config";
import { RoomThreadList, RoomThreadPanel, type RoomThread } from "./RoomThreads";

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  vi.restoreAllMocks();
});
const pipeline: RoomThread = { id: "9", kind: "pipeline", resource_id: "77", name: "API build", deleted: false, linked: true, latest_run: { id: "88", kind: "pipeline_run", state: "failed" } };
const gone: RoomThread = { ...pipeline, id: "10", name: null, deleted: true, latest_run: null };

it("lists threads with their latest run and marks deleted resources", async () => {
  const open = vi.fn();
  await act(async () => root.render(<RoomThreadList threads={[pipeline, gone]} onOpen={open} />));
  const buttons = [...host.querySelectorAll("button")];
  expect(buttons[0]!.textContent).toContain("API build");
  expect(buttons[0]!.textContent).toContain(t("failed"));
  expect(buttons[1]!.textContent).toContain(t("threadResourceDeleted"));
  await act(async () => buttons[0]!.click());
  expect(open).toHaveBeenCalledWith("9");
});

it("posts replies into the open thread and links to the run", async () => {
  const request = vi.spyOn(apiModule, "api").mockResolvedValue({ id: "1" });
  const sent = vi.fn(async () => {});
  await act(async () => root.render(<RoomThreadPanel thread={pipeline} workspace="2" endpoint="/workspaces/2/conversations/5"
    messages={[{ id: "1", role: "notice", prompt: "Run 88 finished: failed.", sender_name: "", created_at: 0 }]}
    onOpenChange={vi.fn()} onSent={sent} />));
  const panel = document.querySelector("[data-thread-panel]")!;
  expect(panel.textContent).toContain("Run 88 finished: failed.");
  expect(panel.querySelector("a")!.getAttribute("href")).toContain("/workspaces/2/pipeline-runs/88");
  const box = panel.querySelector("textarea")!;
  await act(async () => {
    const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!;
    setter.call(box, "why?");
    box.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await act(async () => [...panel.querySelectorAll("button")].find(button => button.textContent === t("send"))!.click());
  expect(request).toHaveBeenCalledWith("/workspaces/2/conversations/5/messages", expect.objectContaining({ method: "POST" }));
  expect(JSON.parse((request.mock.calls[0]![1] as { body: string }).body)).toMatchObject({ prompt: "why?", thread_id: "9" });
  expect(sent).toHaveBeenCalled();
});
