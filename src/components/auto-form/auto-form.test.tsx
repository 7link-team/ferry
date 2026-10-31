import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { AutoForm, AutoField } from "./auto-form";
import type { FormConfig } from "./types";

let root: Root, host: HTMLDivElement;
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
    callback(0);
    return 0;
  });
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  vi.unstubAllGlobals();
});
const fill = async (name: string, value: string) => {
  await act(async () => {
    const input = host.querySelector<HTMLInputElement>(
      `input[name="${name}"]`,
    )!;
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
};
const submit = async () =>
  act(async () => {
    host
      .querySelector("form")!
      .dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
  });

it("blocks invalid input, associates an inline error and lets the summary focus the field", async () => {
  const save = vi.fn();
  await act(async () =>
    root.render(
      <AutoForm
        columns={1}
        fields={{ name: { type: "text", name: "名称", required: true } }}
        onValuesSubmit={save}
      >
        <button type="submit">保存</button>
      </AutoForm>,
    ),
  );
  await submit();
  expect(save).not.toHaveBeenCalled();
  const input = host.querySelector("input")!;
  expect(host.querySelector<HTMLLabelElement>("label[data-auto-label]")!.control).toBe(input);
  expect(input.getAttribute("aria-invalid")).toBe("true");
  const errorId = input.getAttribute("aria-describedby")!.split(" ").at(-1)!;
  expect(document.getElementById(errorId)?.textContent).toContain("请填写名称");
  await act(async () =>
    host.querySelector<HTMLButtonElement>('[role="alert"] button')!.click(),
  );
  expect(document.activeElement).toBe(input);
  await fill("name", "部署");
  await submit();
  expect(save).toHaveBeenCalledWith({ name: "部署" });
});

it("validates composed fields by their shared frame even when their native names differ", async () => {
  const save = vi.fn();
  await act(async () =>
    root.render(
      <AutoForm onSubmit={save}>
        <AutoField label="邮箱">
          <input name="email" type="email" required />
        </AutoField>
      </AutoForm>,
    ),
  );
  await submit();
  expect(save).not.toHaveBeenCalled();
  expect(
    host.querySelector('[data-auto-field] [role="alert"]')?.textContent,
  ).toContain("邮箱");
  await fill("email", "person@example.invalid");
  await submit();
  expect(save).toHaveBeenCalledTimes(1);
  expect(host.querySelector("input")!.hasAttribute("aria-invalid")).toBe(false);
  expect(host.querySelector('[data-auto-field] [role="alert"]')).toBeNull();
});

it("labels the visible select trigger without exposing Radix's hidden form control", async () => {
  await act(async () => root.render(
    <AutoForm fields={{ target: { name: "执行目标", type: "select", config: { options: [{ value: "node-1", label: "节点 1" }] } } }} />,
  ));
  const trigger = host.querySelector<HTMLElement>('[role="combobox"]')!;
  expect(trigger.getAttribute("aria-labelledby")).toBeTruthy();
  expect(host.querySelector<HTMLLabelElement>("label[data-auto-label]")!.htmlFor).toBe(trigger.id);
  const hidden = host.querySelector('select[aria-hidden="true"]')!;
  expect(hidden.hasAttribute("aria-labelledby")).toBe(false);
});

it("keeps nested editor labels associated with their own controls", async () => {
  await act(async () => root.render(
    <AutoField name="editor">
      <AutoField label="应用标识"><input /></AutoField>
      <label><input type="checkbox" />启用</label>
    </AutoField>,
  ));
  const label = host.querySelector<HTMLLabelElement>("label[data-auto-label]")!;
  expect(label.control).toBe(host.querySelector("input"));
});

it("ignores hidden required fields and preserves password and multiline content exactly", async () => {
  const save = vi.fn();
  const fields: FormConfig = {
    hidden: { name: "隐藏项", type: "text", required: true, show: false },
    password: { name: "密码", type: "password", value: "  password  " },
    code: { name: "脚本", type: "code", value: "  echo hello\n\n" },
    metadata: {
      name: "仅展示",
      type: "text",
      value: "read-only",
      submit: false,
    },
  };
  await act(async () =>
    root.render(<AutoForm fields={fields} onValuesSubmit={save} />),
  );
  await submit();
  expect(
    host.querySelector('input[name="password"]')?.getAttribute("type"),
  ).toBe("password");
  expect(save).toHaveBeenCalledWith({
    password: "  password  ",
    code: "  echo hello\n\n",
  });
});

it("runs custom validation and prevents submissions in detail mode", async () => {
  const save = vi.fn();
  const fields: FormConfig = {
    name: {
      name: "名称",
      type: "text",
      value: "taken",
      config: {
        rules: [
          async (value) => (value === "taken" ? "名称已被使用" : undefined),
        ],
      },
    },
  };
  await act(async () =>
    root.render(<AutoForm fields={fields} onValuesSubmit={save} />),
  );
  await submit();
  expect(host.textContent).toContain("名称已被使用");
  expect(save).not.toHaveBeenCalled();
  await act(async () =>
    root.render(
      <AutoForm
        fields={fields}
        mode="detail"
        initialData={{ name: "detail" }}
        onValuesSubmit={save}
      />,
    ),
  );
  await submit();
  expect(save).not.toHaveBeenCalled();
  expect(host.querySelector("input")).toBeNull();
});
