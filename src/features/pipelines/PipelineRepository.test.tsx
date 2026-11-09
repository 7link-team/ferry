import { act,useState } from "react";
import { createRoot,type Root } from "react-dom/client";
import { afterEach,beforeEach,expect,it,vi } from "vitest";
import { MarketplaceRepository,PipelineRepository } from "../../app/pages/PipelineRepository";
import { named } from "../../shared/api/resource";
import { i18n } from "../../shared/i18n/i18n";
let root:Root, host:HTMLDivElement, saved:Record<string,unknown>;
it('shows an empty optional repository and retains unresolved legacy bindings by ID',async()=>{
 const props={workspace:'ops',options:[],onChange:vi.fn(),onValidityChange:vi.fn()};
 await act(async()=>root.render(<MarketplaceRepository {...props} draft={{repository_id:'0'}} readOnly/>));
 expect(host.textContent).toBe('—');
 await act(async()=>root.render(<MarketplaceRepository {...props} draft={{repository_id:'missing'}} readOnly={false}/>));
 expect((host.querySelector('[aria-label="仓库来源"]') as HTMLSelectElement).value).toBe('legacy');
 const repository=host.querySelector('[aria-label="外部仓库"]') as HTMLSelectElement;
 expect(repository.value).toBe('missing');
 expect(repository.selectedOptions[0]?.textContent).toBe(named(undefined,'missing'));
 expect(props.onChange).not.toHaveBeenCalled();
});
it('shows a zero connection as an empty selection without changing its stored value',async()=>{
 await act(async()=>root.render(<MarketplaceRepository workspace="ops" draft={{repository_selection:{connection_id:'0',url:''}}} readOnly={false} onChange={vi.fn()} onValidityChange={()=>{}}/>));
 const connection=host.querySelector('[aria-label="仓库连接"]') as HTMLSelectElement;
 expect(connection.value).toBe('');
 expect([...connection.options].some(option=>option.value==='0')).toBe(false);
 expect(connection.textContent).toContain('GitLab');
});
const definition = 'jobs:\n  build:\n    runs-on: self-hosted\n    steps:\n      - uses: actions/checkout@v4\n      - run: cat proof.txt\n';
function Harness(){const [draft,setDraft]=useState<Record<string,unknown>>({definition,definition_format:"github-actions",checkout_bindings:[]});saved=draft;return <PipelineRepository workspace="ops" draft={draft} jobId="build" stepIndex={0} readOnly={false} onChange={patch=>setDraft(value=>({...value,...patch}))} onValidityChange={()=>{}}/>;}
beforeEach(async()=>{
 await i18n.changeLanguage('zh-CN');
 Object.assign(globalThis,{IS_REACT_ACT_ENVIRONMENT:true});
 host=document.createElement('div');document.body.append(host);root=createRoot(host);
 vi.stubGlobal('fetch',vi.fn(async(url:string)=>({ok:true,status:200,json:async()=>url.includes('/branches?')?{items:[{name:'feature/node-source'}]}:url.includes('/repositories?')?{items:[{name:'repo',full_name:'team/repo',url:'https://git.example/team/repo.git',default_branch:'main'}],has_more:false}: {items:[{id:'c1',name:'My GitLab',provider:'gitlab',state:'connected'}],total:1,page:1,page_size:200}})));
 await act(async()=>root.render(<Harness/>));
});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.unstubAllGlobals();await i18n.changeLanguage('en-US');});
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

it('expands multiple overlapping groups to distinct repository checkouts and retains them across connections',async()=>{
 vi.stubGlobal('fetch',vi.fn(async(url:string)=>({ok:true,status:200,json:async()=>url.includes('/repositories?')?{items:[{name:'one',full_name:'team/one',url:'https://git.example/team/one.git',default_branch:'main'},{name:'two',full_name:'team/sub/two',url:'https://git.example/team/sub/two.git',default_branch:'release'}],has_more:false}:{items:[{id:'7',name:'GitLab',provider:'gitlab',state:'connected'}],total:1,page_size:200}})));
 function Batch(){const [draft,setDraft]=useState<Record<string,unknown>>({definition,checkout_bindings:[]});saved=draft;return <PipelineRepository workspace="ops" draft={draft} jobId="build" stepIndex={0} readOnly={false} onValidityChange={()=>{}} onChange={patch=>setDraft(value=>({...value,...patch}))}/>;}
 await act(async()=>root.render(<Batch/>));
 const details=host.querySelector('details')!;
 await act(async()=>{details.open=true;details.dispatchEvent(new Event('toggle'));});
 const connection=details.querySelector<HTMLSelectElement>('[aria-label="仓库连接"]')!;
 await act(async()=>{connection.value='7';connection.dispatchEvent(new Event('change',{bubbles:true}));});
 const groups=[...host.querySelectorAll<HTMLButtonElement>('button')].find(row=>row.textContent==='选择分组（可多选）')!;
 await act(async()=>groups.dispatchEvent(new MouseEvent('pointerdown',{bubbles:true,button:0})));
 const checkbox=[...document.querySelectorAll<HTMLElement>('[role="menuitemcheckbox"]')].find(row=>row.textContent?.startsWith('team ·'));
 expect(checkbox).toBeDefined();
 await act(async()=>checkbox!.click());
 const sub=[...document.querySelectorAll<HTMLElement>('[role="menuitemcheckbox"]')].find(row=>row.textContent?.startsWith('team/sub ·'))!;
 expect(sub.getAttribute('aria-checked')).toBe('true');
 await act(async()=>{document.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}));});
 const add=[...host.querySelectorAll<HTMLButtonElement>('button')].find(row=>row.textContent==='添加克隆步骤')!;
 await act(async()=>add.click());
 expect(saved.checkout_bindings).toEqual([expect.objectContaining({connection_id:'7',repository_url:'https://git.example/team/one.git',path:'sources/one'}),expect.objectContaining({connection_id:'7',repository_url:'https://git.example/team/sub/two.git',path:'sources/two'})]);
 expect(saved.definition).toContain('checkout_1');expect(saved.definition).toContain('checkout_2');
 expect(String(saved.definition).indexOf('checkout_2')).toBeLessThan(String(saved.definition).indexOf('cat proof.txt'));
});

it('marketplace sources choose a named connection repository without changing source API/project settings',async()=>{
 const {MarketplaceRepository}=await import('../../app/pages/PipelineRepository');
 function MarketHarness(){const [draft,setDraft]=useState<Record<string,unknown>>({repository_id:'',provider:'gitlab',api_base:'https://git.example/api/v4',project:'existing/project',branch:'release'});saved=draft;return <MarketplaceRepository workspace="ops" draft={draft} options={[]} readOnly={false} onChange={patch=>setDraft(value=>({...value,...patch}))} onValidityChange={()=>{}}/>;}
 await act(async()=>root.render(<MarketHarness/>));
 await select('仓库连接','c1');await select('连接仓库','https://git.example/team/repo.git');
 expect(saved.repository_id).toBe('');
 expect(saved.repository_selection).toEqual({connection_id:'c1',name:'repo',url:'https://git.example/team/repo.git',default_branch:'main'});
 expect(saved).toMatchObject({provider:'gitlab',api_base:'https://git.example/api/v4',project:'existing/project',branch:'release'});
});
it('marketplace legacy bindings remain selected until an explicit source change',async()=>{
 const {MarketplaceRepository}=await import('../../app/pages/PipelineRepository');const onChange=vi.fn();const validity=vi.fn();
 await act(async()=>root.render(<MarketplaceRepository workspace="ops" draft={{repository_id:'legacy1'}} options={[{value:'legacy1',label:'Existing private repository'}]} readOnly={false} onChange={onChange} onValidityChange={validity}/>));
 expect(host.querySelector<HTMLSelectElement>('[aria-label="外部仓库"]')?.value).toBe('legacy1');expect(onChange).not.toHaveBeenCalled();expect(validity).toHaveBeenLastCalledWith(false);
});
it('converting native checkout to a bound connection clears conflicting source parameters but preserves other options',async()=>{
 const nativeDefinition='jobs:\n  build:\n    runs-on: self-hosted\n    steps:\n      - uses: actions/checkout@v4\n        with:\n          repository: old/team\n          ref: old-branch\n          path: old-dir\n          fetch-depth: 7\n';
 function NativeHarness(){const [draft,setDraft]=useState<Record<string,unknown>>({definition:nativeDefinition,checkout_bindings:[]});saved=draft;return <PipelineRepository workspace="ops" draft={draft} jobId="build" stepIndex={0} readOnly={false} onChange={patch=>setDraft(value=>({...value,...patch}))} onValidityChange={()=>{}}/>;}
 await act(async()=>root.render(<NativeHarness/>));await select('仓库连接','c1');await select('节点仓库','https://git.example/team/repo.git');
 const {parseActions}=await import('./actions-workflow');
 expect(parseActions(String(saved.definition)).jobs[0]?.steps[0]?.with).toEqual({'fetch-depth':7});
 expect(saved.checkout_bindings).toEqual([expect.objectContaining({connection_id:'c1',repository_url:'https://git.example/team/repo.git',ref:'main',path:''})]);
});

it('loads additional branches, retries the failed page and preserves the ref',async()=>{
 let fail=true;
 const requested:number[]=[];
 const original=globalThis.fetch;
 vi.stubGlobal('fetch',vi.fn(async(url:string,options?:RequestInit)=>{
  if(!url.includes('/branches?'))return original(url,options);
  const page=Number(new URL(url,location.origin).searchParams.get('page')??1);requested.push(page);
  if(page===2&&fail)return {ok:false,status:400,json:async()=>({error:'Codeup请求过于频繁，请稍后重试。'})};
  return {ok:true,status:200,json:async()=>({items:[{name:page===1?'main':'feature/中文'}],has_more:page===1})};
 }));
 await act(async()=>root.render(<Harness/>));
 await select('仓库连接','c1');await select('节点仓库','https://git.example/team/repo.git');
 const click=async(text:string)=>act(async()=>[...host.querySelectorAll('button')].find(button=>button.textContent===text||button.getAttribute('aria-label')===text)!.click());
 await click('加载更多分支');expect(host.querySelector('[role="alert"]')?.textContent).toContain('过于频繁');
 expect(host.querySelector('datalist')?.textContent).toContain('main');
 fail=false;await click('重试读取分支');
 expect(requested).toEqual([1,2,2]);expect(host.querySelector('datalist')?.textContent).toContain('feature/中文');
 expect(host.querySelector('[role="alert"]')).toBeNull();
 expect((host.querySelector('[aria-label="分支 / 标签 / Commit"]') as HTMLInputElement).value).toBe('main');
 await select('仓库连接','');await select('仓库连接','c1');await select('节点仓库','https://git.example/team/repo.git');
 expect(requested).toEqual([1,2,2,1]);
 expect(host.querySelector('datalist')?.textContent).toContain('main');
});

it('ignores late branch responses after changing source',async()=>{
 let late!: (value:unknown)=>void;
 const original=globalThis.fetch;
 vi.stubGlobal('fetch',vi.fn(async(url:string,options?:RequestInit)=>{
  if(!url.includes('/branches?'))return original(url,options);
  return new Promise(resolve=>{late=resolve;});
 }));
 await act(async()=>root.render(<Harness/>));
 await select('仓库连接','c1');await select('节点仓库','https://git.example/team/repo.git');
 expect(host.querySelector('[role="status"]')?.textContent).toContain('读取仓库分支');
 await select('仓库连接','');
 await act(async()=>late({ok:true,status:200,json:async()=>({items:[{name:'stale'}],has_more:false})}));
 expect(host.querySelector('datalist')).toBeNull();expect(host.querySelector('[role="alert"]')).toBeNull();
 expect(host.querySelector('[role="status"]')).toBeNull();
});
