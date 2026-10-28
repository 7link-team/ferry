import { useEffect, useId, useState } from "react";
import { editActionStep, ensureActionCheckout, parseActions } from "../actions-workflow";
import { api, errorText } from "../api";
import { href } from "../router";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import type { Option } from "./dsl";

type Connection = {id:string;name:string;provider:string;state:string;url?:string};
type Remote = {name:string;full_name:string;url:string;default_branch:string};
type Binding = {job_id:string;step_id:string;connection_id:string;repository_url:string;ref:string;path:string};
type Props = { workspace:string;draft:Record<string,unknown>;options?:Option[];jobId?:string;stepIndex?:number;readOnly:boolean;onChange:(patch:Record<string,unknown>)=>void;onValidityChange:(blocked:boolean)=>void };
const selectClass="w-full min-w-0 rounded-md border border-input bg-background px-3 py-2 text-sm";

export function PipelineRepository(props:Props) {
  if(props.jobId===undefined || props.stepIndex===undefined) return <LegacyRepository {...props}/>;
  return <CheckoutRepository {...props} jobId={props.jobId} stepIndex={props.stepIndex}/>;
}
function CheckoutRepository({workspace,draft,jobId,stepIndex,readOnly,onChange,onValidityChange}:Props & {jobId:string;stepIndex:number}) {
  const [branches,setBranches]=useState<string[]>([]);
  const [error,setError]=useState("");
  const branchList=useId();
  const jobs=parseActions(String(draft.definition)).jobs;
  const job=jobs.find(row=>row.id===jobId)!;
  const step=job.steps[stepIndex]!;
  const bindings=(draft.checkout_bindings??[]) as Binding[];
  const current=bindings.find(row=>row.job_id===jobId && row.step_id===step.id);
  const data=useConnectionRepositories(workspace,current?.connection_id);
  const {connection,root}=data;
  const nativeSource=!!(step.with as Record<string,unknown>|undefined)?.repository;
  const legacy=!!draft.repository_id && draft.repository_id!=="0";
  const missing=!current ? !legacy&&!nativeSource : !current.connection_id||!current.repository_url;
  useEffect(()=>{onValidityChange(!readOnly && missing);return()=>onValidityChange(false);},[missing,readOnly,onValidityChange]);
  useEffect(()=>{
    setBranches([]);
    if(!connection || connection.provider==='git' || !current?.repository_url)return;
    const controller=new AbortController();
    void api<{items:{name:string}[]}>(`${root}/${encodeURIComponent(connection.id)}/branches?url=${encodeURIComponent(current.repository_url)}`,{signal:controller.signal})
      .then(value=>{if(!controller.signal.aborted)setBranches(value.items.map(row=>row.name));})
      .catch(cause=>{if(!controller.signal.aborted)setError(errorText(cause));});
    return()=>controller.abort();
  },[root,connection?.id,connection?.provider,current?.repository_url]);
  function change(patch:Partial<Binding>) {
    let stepId=typeof step.id==='string'?step.id:'';
    if(!stepId){let n=1;while(job.steps.some(row=>row.id===`checkout_${n}`))n++;stepId=`checkout_${n}`;}
    const next={job_id:jobId,step_id:stepId,connection_id:'',repository_url:'',ref:'',path:'',...current,...patch};
    const parameters=step.with&&typeof step.with==='object'&&!Array.isArray(step.with)?step.with as Record<string,unknown>:undefined;
    const hasNativeSource=parameters&&['repository','ref','path'].some(key=>key in parameters);
    const retained=parameters?Object.fromEntries(Object.entries(parameters).filter(([key])=>!['repository','ref','path'].includes(key))):undefined;
    const definition=!step.id||hasNativeSource?editActionStep(String(draft.definition),jobId,stepIndex,{id:stepId,...(hasNativeSource?{with:Object.keys(retained!).length?retained:null}:{})}):draft.definition;
    onChange({definition,checkout_bindings:[...bindings.filter(row=>!(row.job_id===jobId&&row.step_id===stepId)),next]});
  }
  if(readOnly)return <dl className="space-y-2 text-sm"><dt>仓库连接</dt><dd>{connection?.name||current?.connection_id||'沿用原有 YAML / 旧版仓库配置'}</dd>{current&&<><dt>仓库</dt><dd className="break-all">{current.repository_url}</dd><dt>分支 / 标签 / Commit</dt><dd>{current.ref||'仓库默认 HEAD'}</dd><dt>检出目录</dt><dd>{current.path||'工作目录'}</dd></>}</dl>;
  return <section aria-label="拉取代码配置" className="min-w-0 space-y-3">
    <ConnectionRepositoryFields workspace={workspace} data={data} connectionId={current?.connection_id??''} repositoryUrl={current?.repository_url??''} repositoryLabel="节点仓库" onChange={selected=>change({connection_id:selected.connection_id,repository_url:selected.url,ref:selected.default_branch})}/>
    {current?.repository_url&&<>
      <label className="block space-y-2"><span className="text-sm">分支 / 标签 / Commit</span><Input aria-label="分支 / 标签 / Commit" list={branchList} value={current.ref} placeholder="选择或输入；留空使用仓库默认 HEAD" onChange={event=>change({ref:event.target.value})}/><datalist id={branchList}>{branches.map(name=><option key={name} value={name}>{name}</option>)}</datalist></label>
      <label className="block space-y-2"><span className="text-sm">检出目录</span><Input aria-label="检出目录" value={current.path} placeholder="默认工作目录；多仓库可填写不同子目录" onChange={event=>change({path:event.target.value})}/></label>
    </>}
    {!current&&legacy&&<p className="text-xs text-muted-foreground">该历史流水线沿用已保存的仓库。选择连接后，此步骤使用独立来源。</p>}
    {!current&&nativeSource&&<p className="text-xs text-muted-foreground">当前使用 YAML 中的 checkout 参数；选择连接后按此节点配置拉取。</p>}
    {error&&<p role="alert" className="text-sm text-destructive">{error}</p>}
  </section>;
}

type RepositorySelection={connection_id:string;name:string;url:string;default_branch:string};
function useConnectionRepositories(workspace:string,connectionId:string|undefined,enabled=true) {
  const [connections,setConnections]=useState<Connection[]>([]),[repositories,setRepositories]=useState<Remote[]>([]);
  const [error,setError]=useState(""),[loading,setLoading]=useState(false),[page,setPage]=useState(1),[hasMore,setHasMore]=useState(false),[revision,setRevision]=useState(0);
  const root=`/workspaces/${encodeURIComponent(workspace)}/code-connections`;
  const connection=connections.find(row=>row.id===connectionId);
  useEffect(()=>{
    if(!enabled)return;
    const controller=new AbortController();setError("");
    void (async()=>{const all:Connection[]=[];for(let page=1;;page++){const data=await api<{items:Connection[];total:number;page_size:number}>(`${root}?page=${page}&page_size=200`,{signal:controller.signal});all.push(...data.items);if(!data.items.length||page*data.page_size>=data.total)break;}if(!controller.signal.aborted)setConnections(all);})().catch(cause=>{if(!controller.signal.aborted)setError(errorText(cause));});
    return()=>controller.abort();
  },[root,revision,enabled]);
  useEffect(()=>{
    setRepositories([]);setPage(1);setHasMore(false);
  },[connectionId]);
  useEffect(()=>{
    if(!enabled || !connection || connection.provider==='git')return;
    const controller=new AbortController();setLoading(true);setError("");
    void api<{items:Remote[];has_more:boolean}>(`${root}/${encodeURIComponent(connection.id)}/repositories?page=${page}`,{signal:controller.signal})
      .then(value=>{if(!controller.signal.aborted){setRepositories(previous=>page===1?value.items:[...previous,...value.items.filter(row=>!previous.some(existing=>existing.url===row.url))]);setHasMore(value.has_more);}})
      .catch(cause=>{if(!controller.signal.aborted)setError(errorText(cause));}).finally(()=>{if(!controller.signal.aborted)setLoading(false);});
    return()=>controller.abort();
  },[root,connection?.id,connection?.provider,page,revision,enabled]);
  return {connections,repositories,connection,root,error,loading,page,hasMore,setPage,setRevision};
}
function ConnectionRepositoryFields({workspace,data,connectionId,repositoryUrl,repositoryLabel,onChange}:{workspace:string;data:ReturnType<typeof useConnectionRepositories>;connectionId:string;repositoryUrl:string;repositoryLabel:string;onChange:(value:RepositorySelection)=>void}) {
  const {connections,connection,repositories,error,loading,page,hasMore,setPage,setRevision}=data;
  return <>
    <label className="block space-y-2"><span className="text-sm">仓库连接</span><select aria-label="仓库连接" className={selectClass} value={connectionId} onChange={event=>{const selected=connections.find(row=>row.id===event.target.value);onChange({connection_id:event.target.value,name:selected?.name??'',url:selected?.provider==='git'?selected.url??'':'',default_branch:''});}}><option value="">请选择连接</option>{connectionId&&!connections.some(row=>row.id===connectionId)&&<option value={connectionId}>{connectionId}</option>}{connections.map(row=><option key={row.id} value={row.id}>{row.name} · {row.provider}{row.state!=='connected'?'（需要重新授权）':''}</option>)}</select></label>
    <div className="flex flex-wrap gap-3 text-xs"><a className="underline" href={href({workspace,group:'build',item:'repositories',mode:'new',id:''},{})} target="_blank" rel="noopener noreferrer">新建仓库连接</a><Button type="button" size="sm" variant="ghost" onClick={()=>setRevision(value=>value+1)}>刷新连接</Button></div>
    {!connections.length&&<p className="text-xs text-muted-foreground">尚无可用仓库连接。先创建连接，再返回选择。</p>}
    {connection?.provider==='git'?<label className="block space-y-2"><span className="text-sm">Git 仓库地址</span><Input aria-label="Git 仓库地址" value={repositoryUrl} onChange={event=>onChange({connection_id:connectionId,name:connection.name,url:event.target.value,default_branch:''})}/></label>:connectionId&&<label className="block space-y-2"><span className="text-sm">仓库</span><select aria-label={repositoryLabel} className={selectClass} value={repositoryUrl} disabled={loading&&page===1} onChange={event=>{const selected=repositories.find(row=>row.url===event.target.value);onChange({connection_id:connectionId,name:selected?.name??'',url:event.target.value,default_branch:selected?.default_branch??''});}}><option value="">{loading?'正在读取可见仓库…':'请选择可见仓库'}</option>{repositoryUrl&&!repositories.some(row=>row.url===repositoryUrl)&&<option value={repositoryUrl}>{repositoryUrl}</option>}{repositories.map(row=><option key={row.url} value={row.url}>{row.full_name||row.name}</option>)}</select></label>}
    {hasMore&&<Button type="button" variant="outline" disabled={loading} onClick={()=>setPage(value=>value+1)}>加载更多仓库</Button>}
    {error&&<p role="alert" className="text-sm text-destructive">{error}</p>}
  </>;
}
/** Marketplace sources can bind a new connection repository or retain a legacy repository ID. */
export function MarketplaceRepository({workspace,draft,options=[],readOnly,onChange,onValidityChange}:Props) {
  const selection=draft.repository_selection as RepositorySelection|undefined;
  const legacyId=String(draft.repository_id??'');
  const mode=selection||!legacyId||legacyId==='0'?'connection':'legacy';
  const data=useConnectionRepositories(workspace,selection?.connection_id,!readOnly&&mode==='connection');
  const missing=mode==='connection'? !selection?.connection_id||!selection.url : !legacyId||legacyId==='0';
  useEffect(()=>{onValidityChange(!readOnly&&missing);return()=>onValidityChange(false);},[readOnly,missing,onValidityChange]);
  if(readOnly)return <p className="break-all text-sm">{selection?.url||options.find(row=>row.value===legacyId)?.label||legacyId||'未选择仓库'}</p>;
  return <section aria-label="市场来源仓库" className="space-y-3">
    <label className="block space-y-2"><span className="text-sm">仓库来源</span><select aria-label="仓库来源" className={selectClass} value={mode} onChange={event=>onChange(event.target.value==='legacy'?{repository_id:options[0]?.value??'',repository_selection:null}:{repository_id:'',repository_selection:{connection_id:'',name:'',url:'',default_branch:''}})}><option value="connection">代码连接</option>{options.length>0&&<option value="legacy">已有仓库</option>}</select></label>
    {mode==='connection'?<ConnectionRepositoryFields workspace={workspace} data={data} connectionId={selection?.connection_id??''} repositoryUrl={selection?.url??''} repositoryLabel="连接仓库" onChange={value=>onChange({repository_id:'',repository_selection:value})}/>:<label className="block space-y-2"><span className="text-sm">外部仓库</span><select aria-label="外部仓库" className={selectClass} value={legacyId} onChange={event=>onChange({repository_id:event.target.value,repository_selection:null})}>{!options.some(row=>row.value===legacyId)&&<option value={legacyId}>{legacyId}</option>}{options.map(row=><option key={row.value} value={row.value}>{row.label}</option>)}</select></label>}
  </section>;
}
function LegacyRepository({draft,options=[],readOnly,onChange}:Props){
 const [error,setError]=useState('');
 return <div className="space-y-3"><label className="block space-y-2"><span>仓库</span><select aria-label="仓库" className={selectClass} disabled={readOnly} value={String(draft.repository_id??'')} onChange={event=>onChange({repository_id:event.target.value,repository_selection:null})}><option value="">不选择（仅执行命令）</option>{options.map(row=><option key={row.value} value={row.value}>{row.label}</option>)}</select></label>
 {!readOnly&&!!draft.repository_id&&<Button type="button" variant="outline" onClick={()=>{try{if(draft.definition_format==='github-actions')onChange({definition:ensureActionCheckout(String(draft.definition))});else{const definition=JSON.parse(String(draft.definition));if(!definition.steps.some((step:{checkout?:boolean})=>step.checkout))definition.steps.unshift({name:'拉取代码',checkout:true,timeout_seconds:1800});onChange({definition:JSON.stringify(definition,null,2)});}setError('');}catch{setError('请先修正流水线定义。');}}}>添加拉取代码步骤</Button>}{error&&<p role="alert">{error}</p>}</div>;
}
