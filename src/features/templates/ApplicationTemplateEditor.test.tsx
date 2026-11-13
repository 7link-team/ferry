import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { parse } from "yaml";
import { ApplicationTemplateEditor, applicationTemplateSource } from "./ApplicationTemplateEditor";

vi.mock("../../shared/editor/CodeEditor", () => ({ CodeEditor: ({ value, readOnly, onChange }: { value: string; readOnly: boolean; onChange: (value: string) => void }) => <textarea aria-label="source" value={value} readOnly={readOnly} onChange={e => onChange(e.target.value)} /> }));
vi.mock("../deployments/ManifestEditor", () => ({ ManifestEditor: ({ value }: { value: string }) => <pre data-manifest>{value}</pre> }));
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
const roots: ReturnType<typeof createRoot>[] = [];
afterEach(async () => { for (const root of roots.splice(0)) await act(() => root.unmount()); document.body.replaceChildren(); vi.clearAllMocks(); });
const blueprint = { apiVersion: "ferry/blueprint-v1", kind: "Blueprint", metadata: { name: "web" }, roles: [{ name: "web", count: "{{ .Values.count }}" }] };
it("formats stored JSON as YAML without changing scalar types or template expressions", () => {
  const value = { ...blueprint, enabled: true, port: 8080, token: "001", data: null, config: "upstream {\n server {{ .Instance.Address }};\n}" };
  const yaml = applicationTemplateSource(JSON.stringify(value));
  expect(yaml.trimStart().startsWith("{")).toBe(false);
  expect(parse(yaml)).toEqual(value);
  const commented = '# keep\napiVersion: ferry/v1\nmetadata: {name: web}\nservices: []\n';
  expect(applicationTemplateSource(commented)).toBe(commented);
  expect(applicationTemplateSource('{"incomplete":')).toBe('{"incomplete":');
});
it("uses a readonly YAML source editor for blueprints without services-form errors or writes", async () => {
  const host=document.createElement('div');document.body.append(host);const root=createRoot(host);roots.push(root);
  const changed=vi.fn(),validity=vi.fn();
  await act(async () => root.render(<ApplicationTemplateEditor value={JSON.stringify(blueprint)} workspace="1" templateParameters={[]} readOnly onChange={changed} onValidityChange={validity} />));
  expect(host.querySelector('[data-manifest]')).toBeNull();
  const input=host.querySelector('textarea')!;
  expect(input.readOnly).toBe(true);expect(parse(input.value)).toEqual(blueprint);
  expect(host.querySelector('[role="alert"]')).toBeNull();expect(changed).not.toHaveBeenCalled();expect(validity).toHaveBeenLastCalledWith(false);
});
it("keeps user-authored JSON and incomplete YAML unchanged while editing and formats external refreshes", async () => {
  const host=document.createElement('div');document.body.append(host);const root=createRoot(host);roots.push(root);
  const validity=vi.fn();let refresh:(value:string)=>void;
  function App(){const[value,setValue]=useState(JSON.stringify(blueprint));refresh=setValue;return <ApplicationTemplateEditor value={value} workspace="1" templateParameters={[]} readOnly={false} onChange={setValue} onValidityChange={validity} />;}
  await act(async()=>root.render(<App/>));
  const json=JSON.stringify({...blueprint,metadata:{name:'edited'}});
  const input=host.querySelector('textarea')!;
  await act(async()=>{Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value')!.set!.call(input,json);input.dispatchEvent(new Event('input',{bubbles:true}));});
  expect(host.querySelector('textarea')!.value).toBe(json);
  const incomplete='apiVersion: ferry/blueprint-v1\nroles: [';
  await act(async()=>{const field=host.querySelector('textarea')!;Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value')!.set!.call(field,incomplete);field.dispatchEvent(new Event('input',{bubbles:true}));});
  expect(host.querySelector('textarea')!.value).toBe(incomplete);
  expect(validity).toHaveBeenLastCalledWith(true);
  await act(()=>refresh!(JSON.stringify({...blueprint,metadata:{name:'refreshed'}})));
  expect(host.querySelector('textarea')!.value).toContain('apiVersion: ferry/blueprint-v1');
});
it("keeps the existing form editor for ordinary applications", async()=>{
  const host=document.createElement('div');document.body.append(host);const root=createRoot(host);roots.push(root);
  const application={apiVersion:'ferry/v1',metadata:{name:'web'},services:[]};
  await act(()=>root.render(<ApplicationTemplateEditor value={JSON.stringify(application)} workspace="1" templateParameters={[]} readOnly onChange={vi.fn()} onValidityChange={vi.fn()} />));
  expect(host.querySelector('[data-manifest]')?.textContent).toContain('apiVersion: ferry/v1');
});
