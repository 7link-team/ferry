import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { PipelineRepository } from "./PipelineRepository";
let root:Root, host:HTMLDivElement, saved:Record<string,unknown>;
const definition = 'jobs:\n  build:\n    runs-on: self-hosted\n    steps:\n      - uses: actions/checkout@v4\n      - run: cat proof.txt\n';
function Harness(){const [draft,setDraft]=useState<Record<string,unknown>>({definition,definition_format:"github-actions",checkout_bindings:[]});saved=draft;return <PipelineRepository workspace="ops" draft={draft} jobId="build" stepIndex={0} readOnly={false} onChange={patch=>setDraft(value=>({...value,...patch}))} onValidityChange={()=>{}}/>;}
beforeEach(async()=>{
 Object.assign(globalThis,{IS_REACT_ACT_ENVIRONMENT:true});
 host=document.createElement('div');document.body.append(host);root=createRoot(host);
 vi.stubGlobal('fetch',vi.fn(async(url:string)=>({ok:true,status:200,json:async()=>url.includes('/branches?')?{items:[{name:'feature/node-source'}]}:url.includes('/repositories?')?{items:[{name:'repo',full_name:'team/repo',url:'https://git.example/team/repo.git',default_branch:'main'}],has_more:false}: {items:[{id:'c1',name:'My GitLab',provider:'gitlab',state:'connected'}],total:1,page:1,page_size:200}})));
 await act(async()=>root.render(<Harness/>));
});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.unstubAllGlobals();});
async function select(label:string,value:string){const node=host.querySelector(`[aria-label="${label}"]`) as HTMLSelectElement;expect(node).not.toBeNull();await act(async()=>{node.value=value;node.dispatchEvent(new Event('change',{bubbles:true}));});}
it('binds a visible repository and non-default branch to the checkout step, not pipeline globals',async()=>{
 await select('仓库连接','c1');await select('节点仓库','https://git.example/team/repo.git');
 const input=host.querySelector('[aria-label="分支 / 标签 / Commit"]') as HTMLInputElement;
 await act(async()=>{Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value')!.set!.call(input,'feature/node-source');input.dispatchEvent(new Event('input',{bubbles:true}));});
 expect(saved).not.toHaveProperty('repository_id');expect(saved).not.toHaveProperty('repository_selection');
 expect(saved.checkout_bindings).toEqual([expect.objectContaining({job_id:'build',step_id:expect.any(String),connection_id:'c1',repository_url:'https://git.example/team/repo.git',ref:'feature/node-source'})]);
 expect(saved.definition).toContain('id: checkout_');
 expect(host.querySelector('datalist')?.textContent).toContain('feature/node-source');
});

it('marketplace sources choose a named connection repository without changing source API/project settings',async()=>{
 const {MarketplaceRepository}=await import('./PipelineRepository');
 function MarketHarness(){const [draft,setDraft]=useState<Record<string,unknown>>({repository_id:'',provider:'gitlab',api_base:'https://git.example/api/v4',project:'existing/project',branch:'release'});saved=draft;return <MarketplaceRepository workspace="ops" draft={draft} options={[]} readOnly={false} onChange={patch=>setDraft(value=>({...value,...patch}))} onValidityChange={()=>{}}/>;}
 await act(async()=>root.render(<MarketHarness/>));
 await select('仓库连接','c1');await select('连接仓库','https://git.example/team/repo.git');
 expect(saved.repository_id).toBe('');
 expect(saved.repository_selection).toEqual({connection_id:'c1',name:'repo',url:'https://git.example/team/repo.git',default_branch:'main'});
 expect(saved).toMatchObject({provider:'gitlab',api_base:'https://git.example/api/v4',project:'existing/project',branch:'release'});
});
it('marketplace legacy bindings remain selected until an explicit source change',async()=>{
 const {MarketplaceRepository}=await import('./PipelineRepository');const onChange=vi.fn();const validity=vi.fn();
 await act(async()=>root.render(<MarketplaceRepository workspace="ops" draft={{repository_id:'legacy1'}} options={[{value:'legacy1',label:'Existing private repository'}]} readOnly={false} onChange={onChange} onValidityChange={validity}/>));
 expect(host.querySelector<HTMLSelectElement>('[aria-label="外部仓库"]')?.value).toBe('legacy1');expect(onChange).not.toHaveBeenCalled();expect(validity).toHaveBeenLastCalledWith(false);
});
it('converting native checkout to a bound connection clears conflicting source parameters but preserves other options',async()=>{
 const nativeDefinition='jobs:\n  build:\n    runs-on: self-hosted\n    steps:\n      - uses: actions/checkout@v4\n        with:\n          repository: old/team\n          ref: old-branch\n          path: old-dir\n          fetch-depth: 7\n';
 function NativeHarness(){const [draft,setDraft]=useState<Record<string,unknown>>({definition:nativeDefinition,checkout_bindings:[]});saved=draft;return <PipelineRepository workspace="ops" draft={draft} jobId="build" stepIndex={0} readOnly={false} onChange={patch=>setDraft(value=>({...value,...patch}))} onValidityChange={()=>{}}/>;}
 await act(async()=>root.render(<NativeHarness/>));await select('仓库连接','c1');await select('节点仓库','https://git.example/team/repo.git');
 const {parseActions}=await import('../actions-workflow');
 expect(parseActions(String(saved.definition)).jobs[0]?.steps[0]?.with).toEqual({'fetch-depth':7});
 expect(saved.checkout_bindings).toEqual([expect.objectContaining({connection_id:'c1',repository_url:'https://git.example/team/repo.git',ref:'main',path:''})]);
});
