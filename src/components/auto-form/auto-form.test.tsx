import { i18n } from "../../i18n";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { AutoForm, AutoField, FieldControl, selectDisplay } from "./auto-form";
import type { FormConfig } from "./types";

let root: Root, host: HTMLDivElement;
it("distinguishes absent references from valid zero enum values and unavailable selections", () => {
  const optional = { emptyValues: [0, "0"], emptyLabel: "No jump host", options: [{ value: "7", label: "Gateway" }] };
  expect(selectDisplay("0", optional)).toBe("No jump host");
  expect(selectDisplay(0, optional)).toBe("No jump host");
  expect(selectDisplay(null, optional)).toBe("No jump host");
  expect(selectDisplay(undefined, optional)).toBe("No jump host");
  expect(selectDisplay("", optional)).toBe("No jump host");
  expect(selectDisplay("7", optional)).toBe("Gateway");
  expect(selectDisplay("9", optional)).toBe("选项不可用（9）");
  expect(selectDisplay(0, { ...optional, options: [{ value: "0", label: "Disabled" }] })).toBe("Disabled");
  expect(selectDisplay(0, { options: [] })).toBe("选项不可用（0）");
});
it("shows an absent reference in edit mode without modifying its raw zero and permits explicit clearing", async () => {
  const change = vi.fn();
  await act(async () => root.render(<FieldControl name="jump_host_id" field={{ name: "Jump host", type: "select", config: { emptyValues: [0], emptyLabel: "No jump host", options: [] } }} value={0} onChange={change} />));
  expect(host.querySelector('[role="combobox"]')?.textContent).toBe("No jump host");
  expect(change).not.toHaveBeenCalled();
});
it("retains missing multiselect references and numeric zero in read-only presentation", async () => {
  await act(async () => root.render(<><FieldControl name="tags" field={{ name: "Tags", type: "multi-select", config: { options: [{ value: "7", label: "Known tag" }] } }} value={["7", "9"]} readonly onChange={() => {}} /><FieldControl name="threshold" field={{ name: "Threshold", type: "number" }} value={0} readonly onChange={() => {}} /></>));
  expect(host.textContent).toContain("Known tag, 选项不可用（9）");
  expect(host.querySelectorAll("p")[1]?.textContent).toBe("0");
});
it("keeps unavailable selected multiselect entries visible instead of dropping them", async () => {
  const change = vi.fn();
  await act(async () => root.render(<FieldControl name="tags" field={{ name: "Tags", type: "multi-select", config: { options: [{ value: "7", label: "Known tag" }] } }} value={["7", "9"]} onChange={change} />));
  const select = host.querySelector("select")!;
  expect([...select.selectedOptions].map(option => option.value)).toEqual(["9", "7"]);
  expect(select.textContent).toContain("选项不可用（9）");
  expect(change).not.toHaveBeenCalled();
});
beforeEach(() => {
  void i18n.changeLanguage("zh-CN");
  localStorage.setItem("ui.locale", "zh-CN");
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
