import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { parse } from "yaml";
import { ApplicationTemplateEditor, applicationTemplateSource } from "./ApplicationTemplateEditor";
import { commonText, hostRepositoryText } from "../../shared/i18n/i18n";

vi.mock("../../shared/editor/CodeEditor", () => ({ CodeEditor: ({ value, readOnly, onChange }: { value: string; readOnly: boolean; onChange: (value: string) => void }) => <textarea aria-label="source" value={value} readOnly={readOnly} onChange={e => onChange(e.target.value)} /> }));
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
it("previews blueprint fields and toggles to readonly YAML without services-form errors or writes", async () => {
  const host=document.createElement('div');document.body.append(host);const root=createRoot(host);roots.push(root);
  const changed=vi.fn(),validity=vi.fn();
  await act(async () => root.render(<ApplicationTemplateEditor value={JSON.stringify(blueprint)} workspace="1" templateParameters={[]} readOnly onChange={changed} onValidityChange={validity} />));
  expect(host.querySelector('[data-manifest]')).toBeNull();
  expect(host.querySelector('[data-blueprint-form]')).not.toBeNull();
  expect(host.textContent).toContain('{{ .Values.count }}');
  expect(host.querySelector('input, select, textarea')).toBeNull();
  expect(host.querySelectorAll('button')).toHaveLength(1);
  await act(async () => host.querySelector('button')!.click());
  const input=host.querySelector('textarea')!;
  expect(input.readOnly).toBe(true);expect(parse(input.value)).toEqual(blueprint);
  expect(host.querySelector('button')!.textContent).toBe(hostRepositoryText('manifest.formPreview'));
  await act(async () => host.querySelector('button')!.click());
  expect(host.querySelector('[data-blueprint-form]')).not.toBeNull();
  expect(host.querySelector('[role="alert"]')).toBeNull();expect(changed).not.toHaveBeenCalled();expect(validity).toHaveBeenLastCalledWith(false);
});
it("keeps user-authored JSON and incomplete YAML unchanged while editing and formats external refreshes", async () => {
  const host=document.createElement('div');document.body.append(host);const root=createRoot(host);roots.push(root);
  const validity=vi.fn();let refresh:(value:string)=>void;
  function App(){const[value,setValue]=useState(JSON.stringify(blueprint));refresh=setValue;return <ApplicationTemplateEditor value={value} workspace="1" templateParameters={[]} readOnly={false} onChange={setValue} onValidityChange={validity} />;}
  await act(async()=>root.render(<App/>));
  await act(async()=>host.querySelector('button')!.click());
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
it("uses the shared field tree for ordinary application authoring", async()=>{
  const host=document.createElement('div');document.body.append(host);const root=createRoot(host);roots.push(root);
  const application={apiVersion:'ferry/v1',metadata:{name:'web'},services:[]};
  await act(()=>root.render(<ApplicationTemplateEditor value={JSON.stringify(application)} workspace="1" templateParameters={[]} readOnly={false} onChange={vi.fn()} onValidityChange={vi.fn()} />));
  expect(host.querySelector('[data-application-definition]')).not.toBeNull();
  expect(host.querySelector('[data-definition-path]')).not.toBeNull();
});

it("uses the same textual definition layout and source toggle for ordinary readonly applications", async()=>{
  const host=document.createElement('div');document.body.append(host);const root=createRoot(host);roots.push(root);
  const source='# original comment\napiVersion: ferry/v1\nmetadata: {name: web}\nservices:\n  - name: web\n    image: nginx:alpine\n    replicas: 0\n    command: [echo, two words]\n    extension: {enabled: false, nullable: null}\n';
  const changed=vi.fn();
  await act(()=>root.render(<ApplicationTemplateEditor value={source} workspace="1" readOnly onChange={changed} onValidityChange={vi.fn()} />));
  expect(host.querySelector('[data-application-definition]')).not.toBeNull();
  expect(host.querySelector('[data-manifest], input, select, textarea')).toBeNull();
  const services=host.querySelector('[data-blueprint-group="services"]')!;
  expect(services.textContent).toContain('nginx:alpine');
  expect(services.textContent).toContain('null');
  // The readonly layout retains the same name field as the editor.
  expect([...services.querySelectorAll('dd')].map(field=>field.textContent)).toEqual(expect.arrayContaining(['web','nginx:alpine']));
  expect([...services.querySelectorAll('ol li')].map(x=>x.textContent)).toEqual(['echo','two words']);
  await act(async()=>host.querySelector('button')!.click());
  expect(host.querySelector('textarea')!.value).toBe(source);
  expect(host.querySelector('textarea')!.readOnly).toBe(true);
  await act(async()=>host.querySelector('button')!.click());
  expect(host.querySelector('[data-application-definition]')).not.toBeNull();
  expect(changed).not.toHaveBeenCalled();
});

function atPath(host: HTMLElement, path: (string | number)[]) {
  return [...host.querySelectorAll<HTMLElement>('[data-definition-path]')].find(node => node.dataset.definitionPath === JSON.stringify(path))!;
}
async function fill(input: HTMLInputElement | HTMLTextAreaElement, value: string) {
  await act(() => { Object.getOwnPropertyDescriptor(input instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype, 'value')!.set!.call(input, value);input.dispatchEvent(new Event('input', { bubbles: true })); });
}
it.each([false, true])("edits actual nested fields and appends/removes collections without losing source comments (blueprint=%s)", async blueprint => {
  const value = blueprint
    ? { ...blueprintFixture(), roles: [{ name:'web', identity:'stateless', instances:{min:1,max:3,default:1}, service:{image:'nginx:old',command:['echo','two words']} }] }
    : { apiVersion:'ferry/v2',kind:'Application',metadata:{name:'web'},services:[{name:'web',image:'nginx:old',replicas:1,command:['echo','two words']}] };
  Object.assign(value, { extension: { zero: 0, flag: false, nested: { retained: 'untouched' } } });
  let current = '# untouched comment\n' + applicationTemplateSource(JSON.stringify(value));
  const host=document.createElement('div');document.body.append(host);const root=createRoot(host);roots.push(root);
  function App(){const[source,setSource]=useState(current);return <ApplicationTemplateEditor value={source} readOnly={false} workspace="" onChange={next=>{current=next;setSource(next);}} onValidityChange={()=>{}}/>;}
  await act(()=>root.render(<App/>));
  expect(atPath(host,['extension','zero']).querySelector('input')!.value).toBe('0');
  const service=blueprint?['roles',0,'service']:['services',0];
  await fill(atPath(host,[...service,'image']).querySelector('textarea')!,'nginx:changed');
  expect(parse(current)).toMatchObject(blueprint?{roles:[{service:{image:'nginx:changed',command:['echo','two words']}}]}:{services:[{image:'nginx:changed',command:['echo','two words']}]});
  expect(current).toContain('# untouched comment');
  expect(parse(current).extension).toEqual({zero:0,flag:false,nested:{retained:'untouched'}});
  const group=host.querySelector(`[data-blueprint-group="${blueprint?'roles':'services'}"]`)!;
  const add=[...group.querySelectorAll<HTMLButtonElement>('button')].find(button=>button.textContent===hostRepositoryText('manifest-extra.003',{value1:blueprint?commonText('resource-domain:wizard.roles'):commonText('resource-domain:deployment.servicesSection')}))!;
  await act(()=>add.click());
  expect(parse(current)[blueprint?'roles':'services']).toHaveLength(2);
  expect(current).toContain('# untouched comment');
  const items=group.querySelectorAll(`[data-blueprint-item="${blueprint?'roles':'services'}"]`);
  await act(()=>items[1]!.querySelector('button')!.click());
  expect(document.querySelector('[role="dialog"]')).not.toBeNull();
  const dialog=document.querySelector('[role="dialog"]')!;
  await act(async()=>[...dialog.querySelectorAll<HTMLButtonElement>('button')].at(-1)!.click());
  expect(parse(current)[blueprint?'roles':'services']).toHaveLength(1);
  expect(current).toContain('# untouched comment');
});
function blueprintFixture(){return {apiVersion:'ferry/blueprint-v1',kind:'Blueprint',metadata:{name:'web'},allowSingleHost:false};}

it("keeps numeric drafts invalid without changing their YAML type, then restores validity",async()=>{
  let current=JSON.stringify({apiVersion:'ferry/v2',metadata:{name:'web'},services:[{name:'web',image:'nginx',replicas:2}]});
  const validity=vi.fn(),host=document.createElement('div');document.body.append(host);const root=createRoot(host);roots.push(root);
  function App(){const[source,setSource]=useState(current);return <ApplicationTemplateEditor value={source} readOnly={false} workspace="" onChange={next=>{current=next;setSource(next);}} onValidityChange={validity}/>;}
  await act(()=>root.render(<App/>));
  await fill(atPath(host,['services',0,'replicas']).querySelector('input')!,'');
  expect(validity).toHaveBeenLastCalledWith(true);expect(parse(current).services[0].replicas).toBe(2);
  expect(host.querySelector('button')!.disabled).toBe(true);
  await fill(atPath(host,['services',0,'replicas']).querySelector('input')!,'3');
  expect(validity).toHaveBeenLastCalledWith(false);expect(parse(current).services[0].replicas).toBe(3);
});

it("keeps YAML aliases in source mode without recursively expanding a cyclic document",async()=>{
  const host=document.createElement('div');document.body.append(host);const root=createRoot(host);roots.push(root);
  await act(()=>root.render(<ApplicationTemplateEditor value={'apiVersion: ferry/v2\nmetadata: {name: test}\nservices: []\nunknown: &cycle {loop: *cycle}'} readOnly={false} workspace="" onChange={vi.fn()} onValidityChange={vi.fn()}/>));
  expect(host.querySelector('[data-application-definition]')).toBeNull();expect(host.querySelector('textarea')).not.toBeNull();expect(host.querySelector('[role="alert"]')).not.toBeNull();
});
