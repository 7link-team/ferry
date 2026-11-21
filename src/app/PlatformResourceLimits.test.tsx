import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { PlatformResourceLimits, resourceLimitGroups } from "./PlatformResourceLimits";

afterEach(() => { document.body.innerHTML = ""; });

it("defaults every resource ceiling to unlimited and shows no hidden numeric default", async () => {
  const element = document.createElement("div");
  document.body.append(element);
  const root = createRoot(element);
  await act(async () => root.render(<PlatformResourceLimits draft={{}} readOnly={false} onPatch={vi.fn()} />));
  expect(element.querySelectorAll("select").length).toBe(resourceLimitGroups.flatMap(group => group.keys).length);
  expect([...element.querySelectorAll("select")].every(input => input.value === "unlimited")).toBe(true);
  expect(element.querySelectorAll('input[type="number"]')).toHaveLength(0);
  await act(async () => root.unmount());
});

it("clears one configured ceiling without deleting the other configured ceilings", async () => {
  const element = document.createElement("div");
  document.body.append(element);
  const patch = vi.fn();
  const root = createRoot(element);
  await act(async () => root.render(<PlatformResourceLimits draft={{ resource_limits: { tool_response_bytes: 4096, file_count: 9 } }} readOnly={false} onPatch={patch} />));
  const select = element.querySelector<HTMLSelectElement>('[data-resource-limit="tool_response_bytes"] select')!;
  await act(async () => { select.value = "unlimited"; select.dispatchEvent(new Event("change", { bubbles: true })); });
  expect(patch).toHaveBeenLastCalledWith({ resource_limits: { tool_response_bytes: null, file_count: 9 } });
  await act(async () => root.unmount());
});

it("makes detail mode immutable", async () => {
  const element = document.createElement("div");
  const root = createRoot(element);
  await act(async () => root.render(<PlatformResourceLimits draft={{resource_limits:{file_count:3}}} readOnly onPatch={vi.fn()} />));
  expect(element.querySelectorAll("select,input,button")).toHaveLength(0);
  expect(element.textContent).toContain("3");
  await act(async () => root.unmount());
});

it("blocks invalid ceilings until corrected or explicitly cleared", async () => {
  const element = document.createElement("div"); document.body.append(element);
  const root = createRoot(element); const validity = vi.fn();
  function Harness() { const [draft, setDraft] = useState<Record<string,unknown>>({resource_limits:{file_count:3,file_upload_bytes:4096}}); return <PlatformResourceLimits draft={draft} readOnly={false} onPatch={patch=>setDraft(value=>({...value,...patch}))} onValidityChange={validity}/>; }
  await act(async()=>root.render(<Harness/>));
  const input=()=>element.querySelector<HTMLInputElement>('[data-resource-limit="file_count"] input')!;
  for(const invalid of ["0","-1","1.5","9007199254740992",""]) {
    await act(async()=>{ Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,"value")!.set!.call(input(),invalid);input().dispatchEvent(new Event("input",{bubbles:true})); });
    expect(validity).toHaveBeenLastCalledWith(true); expect(input().getAttribute("aria-invalid")).toBe("true");
  }
  const select=element.querySelector<HTMLSelectElement>('[data-resource-limit="file_count"] select')!;
  await act(async()=>{select.value="unlimited";select.dispatchEvent(new Event("change",{bubbles:true}));});
  expect(validity).toHaveBeenLastCalledWith(false);
  expect(element.querySelector<HTMLInputElement>('[data-resource-limit="file_upload_bytes"] input')!.value).toBe("4096");
  await act(async()=>root.unmount());
});
