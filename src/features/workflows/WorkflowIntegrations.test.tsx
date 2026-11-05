import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { api } from "../../shared/api/api";
import { i18n, workflowText as t } from "../../shared/i18n/i18n";
import { WorkflowAssetContext } from "./WorkflowAssets";
import { WorkflowNodePicker } from "./WorkflowNodePicker";
import { WorkflowNodeParameters } from "./WorkflowNodeParameters";
import { emptyGraph, newNode } from "./graph";
import {
  integrationManifests,
  integrationText,
  integrationTitle,
  installIntegrationCatalog,
} from "./integrations";

vi.mock("../../shared/api/api", () => ({
  api: vi.fn(),
  errorText: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
}));
let root: Root, host: HTMLDivElement;
const calls = vi.mocked(api);
beforeEach(async () => {
  await i18n.changeLanguage("zh-CN");
  calls.mockReset();
  calls.mockResolvedValue({
    items: [
      { id: "7", name: "Slack token", username: "slack", kind: "token" },
      { id: "8", name: "GitHub token", username: "github", kind: "token" },
    ],
    page: 1,
    page_size: 200,
    total: 2,
  });
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
});
const button = (label: string) =>
  [...document.querySelectorAll<HTMLButtonElement>("button")].find(
    (button) =>
      button.getAttribute("aria-label") === label ||
      button.textContent?.trim() === label,
  )!;
async function input(label: string, value: string) {
  const field = [
    ...document.querySelectorAll<
      HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement
    >("input,select,textarea"),
  ].find((field) => field.getAttribute("aria-label") === label)!;
  expect(field, label).toBeDefined();
  await act(async () => {
    const prototype =
      field instanceof HTMLSelectElement
        ? HTMLSelectElement.prototype
        : field instanceof HTMLTextAreaElement
          ? HTMLTextAreaElement.prototype
          : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(prototype, "value")!.set!.call(
      field,
      value,
    );
    field.dispatchEvent(
      new Event(field instanceof HTMLSelectElement ? "change" : "input", {
        bubbles: true,
      }),
    );
  });
}

it("discovers applications and selects a specific declared operation while respecting compatible ports", async () => {
  const select = vi.fn();
  await act(async () =>
    root.render(
      <WorkflowNodePicker
        inserting={false}
        accepts={(type) => type !== "integration.github.workflow.dispatch"}
        select={select}
        close={() => {}}
      />,
    ),
  );
  await input(t("search"), "git");
  const spec = integrationManifests.find(
    (node) => node.type === "integration.github.issue.create",
  )!;
  expect(button(integrationTitle(spec))).toBeDefined();
  expect(button(integrationTitle(integrationManifests.find(node => node.ui.application === "gitlab")!))).toBeDefined();
  expect(button(integrationTitle(integrationManifests.find(node => node.ui.application === "slack")!))).toBeUndefined();
  await act(async () => button(integrationTitle(spec)).click());
  expect(select).toHaveBeenCalledWith(spec.type);
  expect(
    button(
      integrationTitle(
        integrationManifests.find(
          (node) => node.type === "integration.github.workflow.dispatch",
        )!,
      ),
    ),
  ).toBeUndefined();
});

it("creates application-bound credentials, selects remote choices and saves plain parameters without secrets", async () => {
  const spec = integrationManifests.find(
    (node) => node.type === "integration.slack.message.create",
  )!;
  const node = newNode(emptyGraph(), spec.type, "Notify");
  let current = node.params;
  function Harness() {
    const [params, update] = useState(current);
    current = params;
    return (
      <WorkflowAssetContext.Provider value={{ workspace: "11", workflow: "" }}>
        <WorkflowNodeParameters
          node={{ ...node, params }}
          update={update}
          invalid={() => {}}
          readOnly={false}
        />
      </WorkflowAssetContext.Provider>
    );
  }
  await act(async () => root.render(<Harness />));
  expect(document.querySelector('option[value="8"]')).toBeNull();
  await act(async () => button(t("integration.createCredential")).click());
  await input(t("integration.name"), "Notification connection");
  await input(t("integration.token"), "LOCAL_TEST_TOKEN");
  calls.mockResolvedValueOnce({ id: "9" });
  await act(async () =>
    document
      .querySelector<HTMLFormElement>('[role="dialog"] form')!
      .dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })),
  );
  expect(calls).toHaveBeenCalledWith(
    "/workspaces/11/credentials",
    expect.objectContaining({
      body: expect.stringContaining('"username":"slack"'),
    }),
  );
  expect(current.credential_id).toBe("9");
  expect(host.querySelector('option[value="9"]')?.textContent).toBe(
    "Notification connection",
  );
  expect(document.querySelector('input[type="password"]')).toBeNull();
  calls.mockResolvedValueOnce({ channels: [{ id: "C123", name: "general" }] });
  await act(async () =>
    button(
      `${t("integration.loadOptions")} · ${t("integration.channel")}`,
    ).click(),
  );
  await input(`${t("integration.channel")} · ${t("selectValue")}`, "C123");
  await input(t("integration.message"), "={{ $json.message }}");
  expect(current).toEqual({
    credential_id: "9",
    channel: "C123",
    text: "={{ $json.message }}",
  });
  expect(JSON.stringify(current)).not.toContain("LOCAL_TEST_TOKEN");
  expect(document.querySelectorAll("details[open]")).toHaveLength(0);
});

it("makes all parameter mutations unavailable in read-only mode", async () => {
  const node = newNode(
    emptyGraph(),
    "integration.github.issue.create",
    "Issue",
  );
  await act(async () =>
    root.render(
      <WorkflowNodeParameters
        node={node}
        readOnly
        update={() => {
          throw new Error("read-only mutation");
        }}
        invalid={() => {}}
      />,
    ),
  );
  expect(button(t("integration.createCredential"))).toBeUndefined();
  expect(
    [
      ...host.querySelectorAll<
        HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement
      >("input,select,textarea"),
    ].every(
      (field) => field.disabled || ("readOnly" in field && field.readOnly),
    ),
  ).toBe(true);
});

it("shows credential creation errors inside the dialog and retains input for retry", async () => {
  const node = newNode(
    emptyGraph(),
    "integration.slack.message.create",
    "Notify",
  );
  await act(async () =>
    root.render(
      <WorkflowAssetContext.Provider value={{ workspace: "11", workflow: "" }}>
        <WorkflowNodeParameters
          node={node}
          update={() => {}}
          invalid={() => {}}
          readOnly={false}
        />
      </WorkflowAssetContext.Provider>,
    ),
  );
  await act(async () => button(t("integration.createCredential")).click());
  await input(t("integration.name"), "Retry connection");
  await input(t("integration.token"), "LOCAL_RETRY_TOKEN");
  calls.mockRejectedValueOnce(new Error("Request failed"));
  const submit = () =>
    document
      .querySelector<HTMLFormElement>('[role="dialog"] form')!
      .dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
  await act(async () => {
    submit();
  });
  expect(
    document.querySelector('[role="dialog"] [role="alert"]')?.textContent,
  ).toBe("Request failed");
  expect(
    document.querySelector<HTMLInputElement>(
      '[role="dialog"] input[type="password"]',
    )?.value,
  ).toBe("LOCAL_RETRY_TOKEN");
  calls.mockResolvedValueOnce({
    id: "9",
    name: "Retry connection",
    kind: "token",
    username: "slack",
  });
  await act(async () => {
    submit();
  });
  expect(document.querySelector('[role="dialog"]')).toBeNull();
  expect(document.querySelector('input[type="password"]')).toBeNull();
});

it("finds the existing AI chat node by its Anthropic protocol", async () => {
  const select = vi.fn();
  await act(async () =>
    root.render(
      <WorkflowNodePicker
        inserting={false}
        accepts={() => true}
        select={select}
        close={() => {}}
      />,
    ),
  );
  await input(t("search"), "anthropic");
  expect(button(t("aiChat"))).toBeDefined();
  await act(async () => button(t("aiChat")).click());
  expect(select).toHaveBeenCalledWith("ai.llm.chat");
});

it("discards remote options returned after the selected credential changes", async () => {
  const node = newNode(
    emptyGraph(),
    "integration.slack.message.create",
    "Notify",
  );
  let finish!: (value: unknown) => void;
  function Harness() {
    const [params, update] = useState<Record<string, unknown>>({
      ...node.params,
      credential_id: "7",
    });
    return (
      <WorkflowAssetContext.Provider value={{ workspace: "11", workflow: "" }}>
        <WorkflowNodeParameters
          node={{ ...node, params }}
          update={update}
          invalid={() => {}}
          readOnly={false}
        />
      </WorkflowAssetContext.Provider>
    );
  }
  await act(async () => root.render(<Harness />));
  calls.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  await act(async () =>
    button(
      `${t("integration.loadOptions")} · ${t("integration.channel")}`,
    ).click(),
  );
  const request = calls.mock.calls.at(-1)![1]!;
  await input(t("integration.credential"), "0");
  expect(request.signal?.aborted).toBe(true);
  await act(async () => finish({ channels: [{ id: "OLD", name: "Stale" }] }));
  expect(host.querySelector('option[value="OLD"]')).toBeNull();
});

it("all declared fields and operation labels are translated in all six locales", async () => {
  for (const locale of ["en-US", "zh-CN", "zh-TW", "ja-JP", "ko-KR", "ms-MY"]) {
    await i18n.changeLanguage(locale);
    for (const spec of integrationManifests) {
      for (const key of [
        spec.ui.resource_key,
        spec.ui.operation_key,
        spec.ui.credential_help_key,
        ...Object.values(spec.params_schema.properties).map(
          (field) => field.title_key,
        ),
      ])
        expect(integrationText(key)).not.toBe(key);
    }
  }
});

it("runtime packages join the existing picker and graph without retaining removed packages", async () => {
  const bundled = [...integrationManifests];
  const original = bundled[0]!;
  const extension = {
    ...original,
    type: "integration.ext.sample.v1.read",
    ui: {...original.ui, application:"ext.sample.v1", application_name:"Sample connector v1", resource_key:"literal:Repository", operation_key:"literal:Read", credential_help_key:"literal:Package connection", aliases:["sample"]},
  };
  try {
    installIntegrationCatalog([...bundled, extension]);
    expect(newNode(emptyGraph(), extension.type, "Sample").type).toBe(extension.type);
    expect(integrationTitle(extension)).toBe("Sample connector v1 · Repository · Read");
    await act(async () => root.render(<WorkflowNodePicker inserting={false} accepts={() => true} select={vi.fn()} close={vi.fn()} />));
    expect(button("Sample connector v1")).toBeDefined();
    await act(async () => button("Sample connector v1").click());
    await act(async () => button("Repository").click());
    expect(button("Sample connector v1 · Repository · Read")).toBeDefined();
    installIntegrationCatalog(bundled);
    expect(() => newNode(emptyGraph(), extension.type, "Removed")).toThrow("Unknown node type");
  } finally { installIntegrationCatalog(bundled); }
});
