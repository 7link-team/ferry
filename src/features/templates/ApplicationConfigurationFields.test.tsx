import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, expect, it, vi } from 'vitest';
import { marketplaceForms } from '../marketplace/resource-config';
import { templateForm } from './resource-config';
import { deploymentsForms } from '../deployments/resource-config';
import { i18n } from '../../shared/i18n/i18n';
import { applicationSections } from '../deployments/application-sections';

vi.mock('../../shared/editor/CodeEditor', () => ({ CodeEditor: ({value,readOnly,onChange}:{value:string;readOnly:boolean;onChange:(value:string)=>void}) => <textarea value={value} readOnly={readOnly} onChange={event=>onChange(event.target.value)} /> }));
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
let root:Root|undefined;
afterEach(async()=>{if(root)await act(()=>root!.unmount());root=undefined;document.body.replaceChildren();});
it('all resource configurations use canonical definition fields while template editing retains its writable contract',async()=>{
  await i18n.changeLanguage('en-US');
  const context={workspace:'test',navigate:vi.fn()};
  const forms=[marketplaceForms(context as never).marketplace!,templateForm('app-templates',true,'test'),deploymentsForms(context as never).deployments!];
  const source=JSON.stringify({apiVersion:'ferry/blueprint-v1',kind:'Blueprint',metadata:{name:'same'},roles:[{name:'web',service:{image:'nginx:fixed'}}]});
  const host=document.createElement('div');document.body.append(host);root=createRoot(host);
  for(const config of forms){
    const field=config.sections.flatMap(section=>section.fields).find(field=>['content','manifest'].includes(field.key))!;
    expect(field.type).toBe('custom');expect(field.labelKey).toBe('resource-domain:template.definition');
    const patch=vi.fn();await act(()=>root!.render(<>{field.render!({workspace:'test',draft:{content:source,manifest:source,parameters:'[]'},readOnly:true,onPatch:patch,onValidityChange:vi.fn()})}</>));
    expect(host.querySelector('[data-blueprint-form]')?.textContent).toContain('nginx:fixed');
    expect(host.querySelector('input,textarea')).toBeNull();expect(patch).not.toHaveBeenCalled();
  }
  const config=forms[1]!,field=config.sections.flatMap(section=>section.fields).find(field=>field.key==='content')!,patch=vi.fn();
  await act(()=>root!.render(<>{field.render!({workspace:'test',draft:{content:source,parameters:'[]'},readOnly:false,onPatch:patch,onValidityChange:vi.fn()})}</>));
  await act(async()=>host.querySelector('button')!.click());
  expect(host.querySelector('textarea')!.readOnly).toBe(false);
  await act(()=>{const input=host.querySelector('textarea')!;Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value')!.set!.call(input,'apiVersion: ferry/blueprint-v1\nroles: []');input.dispatchEvent(new Event('input',{bubbles:true}));});
  expect(patch).toHaveBeenCalledWith({content:'apiVersion: ferry/blueprint-v1\nroles: []'});
});
it('preserves only declared sections and gives actual instance values the shared parameter section',()=>{
  const sections=applicationSections([{fields:[{key:'values',label:'Parameters',type:'custom'},{key:'runtime',label:'Runtime',type:'custom'}]}],'test');
  expect(sections.map(section=>section.group)).toEqual(['parameters','runtime']);
  expect(sections[0]!.fields[0]!.when!({values:{enabled:false,count:0}})).toBe(true);
  expect(sections[0]!.fields[0]!.when!({})).toBe(false);
});
