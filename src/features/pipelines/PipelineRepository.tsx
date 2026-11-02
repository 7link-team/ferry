import { useEffect,useId,useState } from "react";
import { api,errorText } from "../../shared/api/api";
import { named } from "../../shared/api/resource";
import { AutoField } from "../../shared/forms/auto-form";
import { hostRepositoryText,useCommonTranslation } from "../../shared/i18n/i18n";
import type { Option } from "../../shared/resources/types";
import type { Navigation } from "../../shared/routing/types";
import { Button } from "../../shared/ui/button";
import { Input } from "../../shared/ui/input";
import type { RepositorySelection } from "../repositories/ConnectionRepositoryFields";
import { ConnectionRepositoryFields,useConnectionRepositories } from "../repositories/ConnectionRepositoryFields";
import { editActionStep,parseActions } from "./actions-workflow";

type Binding = {job_id:string;step_id:string;connection_id:string;repository_url:string;ref:string;path:string};
type Props = { navigation: Navigation; workspace:string;draft:Record<string,unknown>;options?:Option[];jobId?:string;stepIndex?:number;readOnly:boolean;onChange:(patch:Record<string,unknown>)=>void;onValidityChange:(blocked:boolean)=>void };
const selectClass="w-full min-w-0 rounded-md border border-input bg-background px-3 py-2 text-sm";

export function PipelineRepository(props:Props & {jobId:string;stepIndex:number}) {
  return <CheckoutRepository {...props} jobId={props.jobId} stepIndex={props.stepIndex}/>;
}
function CheckoutRepository({navigation,workspace,draft,jobId,stepIndex,readOnly,onChange,onValidityChange}:Props & {jobId:string;stepIndex:number}) {
  useCommonTranslation();
  const [branchRequest,setBranchRequest]=useState({source:'',page:1,revision:0});
  const [branchResult,setBranchResult]=useState({source:'',items:[] as string[],page:0,hasMore:false,error:''});
  const [loadingBranches,setLoadingBranches]=useState(false);
  const branchList=useId();
  const jobs=parseActions(String(draft.definition)).jobs;
  const job=jobs.find(row=>row.id===jobId)!;
  const step=job.steps[stepIndex]!;
  const bindings=(draft.checkout_bindings??[]) as Binding[];
  const current=bindings.find(row=>row.job_id===jobId && row.step_id===step.id);
  const data=useConnectionRepositories(workspace,current?.connection_id);
  const {connection,root}=data;
  const branchSource=JSON.stringify([root,connection?.id,connection?.provider,current?.repository_url]);
  const branchPage=branchRequest.source===branchSource?branchRequest.page:1;
  const branchData=branchResult.source===branchSource?branchResult:undefined;
  const nativeSource=!!(step.with as Record<string,unknown>|undefined)?.repository;
  const legacy=!!draft.repository_id && draft.repository_id!=="0";
  const missing=!current ? !legacy&&!nativeSource : !current.connection_id||current.connection_id==='0'||!current.repository_url;
  useEffect(()=>{onValidityChange(!readOnly && missing);return()=>onValidityChange(false);},[missing,readOnly,onValidityChange]);
  useEffect(()=>{
    setBranchRequest(previous=>previous.source===branchSource?previous:{source:branchSource,page:1,revision:previous.revision});
    if(readOnly || !connection || connection.provider==='git' || !current?.repository_url){setLoadingBranches(false);return;}
    const controller=new AbortController();
    setLoadingBranches(true);
    setBranchResult(previous=>previous.source===branchSource?{...previous,error:''}:{source:branchSource,items:[],page:0,hasMore:false,error:''});
    void api<{items:{name:string}[];has_more:boolean}>(`${root}/${encodeURIComponent(connection.id)}/branches?url=${encodeURIComponent(current.repository_url)}&page=${branchPage}`,{signal:controller.signal})
      .then(value=>{if(!controller.signal.aborted)setBranchResult(previous=>({source:branchSource,items:[...new Set([...(branchPage>1&&previous.source===branchSource?previous.items:[]),...value.items.map(row=>row.name)])],page:branchPage,hasMore:value.has_more,error:''}));})
      .catch(cause=>{if(!controller.signal.aborted)setBranchResult(previous=>({...previous,error:errorText(cause)}));})
      .finally(()=>{if(!controller.signal.aborted)setLoadingBranches(false);});
    return()=>controller.abort();
  },[root,connection?.id,connection?.provider,current?.repository_url,branchSource,branchPage,branchRequest.revision,readOnly]);
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
  if(readOnly)return <dl className="space-y-2 text-sm"><dt>{hostRepositoryText("repository.002")}</dt><dd>{current ? named(connection?.name,current.connection_id) : hostRepositoryText("repository.003")}</dd>{current&&<><dt>{hostRepositoryText("repository.004")}</dt><dd className="break-all">{current.repository_url}</dd><dt>{hostRepositoryText("repository.005")}</dt><dd>{current.ref||hostRepositoryText("repository.006")}</dd><dt>{hostRepositoryText("repository.007")}</dt><dd>{current.path||hostRepositoryText("repository.008")}</dd></>}</dl>;
  return <section aria-label={hostRepositoryText("repository.009")} className="min-w-0 space-y-3">
    <ConnectionRepositoryFields navigation={navigation} workspace={workspace} data={data} connectionId={current?.connection_id??''} repositoryUrl={current?.repository_url??''} repositoryLabel={hostRepositoryText("repository.010")} onChange={selected=>change({connection_id:selected.connection_id,repository_url:selected.url,ref:selected.default_branch})}/>
    {current?.repository_url&&<>
      <AutoField label={<span className="text-sm">{hostRepositoryText("repository.005")}</span>} className="block space-y-2"><Input aria-label={hostRepositoryText("repository.005")} list={branchList} value={current.ref} placeholder={hostRepositoryText("repository.011")} onChange={event=>change({ref:event.target.value})}/><datalist id={branchList}>{branchData?.items.map(name=><option key={name} value={name}>{name}</option>)}</datalist></AutoField>
      {loadingBranches&&<p role="status" className="text-xs text-muted-foreground">{hostRepositoryText("repository.012")}</p>}
      {branchData?.hasMore&&!branchData.error&&<Button type="button" variant="outline" disabled={loadingBranches} onClick={()=>setBranchRequest({source:branchSource,page:branchData.page+1,revision:branchRequest.revision})}>{hostRepositoryText("repository.013")}</Button>}
      {branchData?.error&&<div className="space-y-2"><p role="alert" className="text-sm text-destructive">{branchData.error}</p><Button type="button" variant="outline" disabled={loadingBranches} onClick={()=>setBranchRequest({source:branchSource,page:branchData.page+1,revision:branchRequest.revision+1})}>{hostRepositoryText("repository.014")}</Button></div>}
      <AutoField label={<span className="text-sm">{hostRepositoryText("repository.007")}</span>} className="block space-y-2"><Input aria-label={hostRepositoryText("repository.007")} value={current.path} placeholder={hostRepositoryText("repository.015")} onChange={event=>change({path:event.target.value})}/></AutoField>
    </>}
    {!current&&legacy&&<p className="text-xs text-muted-foreground">{hostRepositoryText("repository.016")}</p>}
    {!current&&nativeSource&&<p className="text-xs text-muted-foreground">{hostRepositoryText("repository.017")}</p>}
  </section>;
}

/** Marketplace sources can bind a new connection repository or retain a legacy repository ID. */
export function MarketplaceRepository({navigation,workspace,draft,options=[],readOnly,onChange,onValidityChange}:Props) {
  useCommonTranslation();
  const selection=draft.repository_selection as RepositorySelection|undefined;
  const legacyId=String(draft.repository_id??'');
  const mode=selection||!legacyId||legacyId==='0'?'connection':'legacy';
  const data=useConnectionRepositories(workspace,selection?.connection_id,!readOnly&&mode==='connection');
  const missing=mode==='connection'? !selection?.connection_id||selection.connection_id==='0'||!selection.url : !legacyId||legacyId==='0';
  useEffect(()=>{onValidityChange(!readOnly&&missing);return()=>onValidityChange(false);},[readOnly,missing,onValidityChange]);
  if(readOnly)return <p className="break-all text-sm">{selection?.url||named(options.find(row=>row.value===legacyId)?.label,legacyId)}</p>;
  return <section aria-label={hostRepositoryText("repository.028")} className="space-y-3">
    <AutoField label={<span className="text-sm">{hostRepositoryText("repository.029")}</span>} className="block space-y-2"><select aria-label={hostRepositoryText("repository.029")} className={selectClass} value={mode} onChange={event=>onChange(event.target.value==='legacy'?{repository_id:options[0]?.value??'',repository_selection:null}:{repository_id:'',repository_selection:{connection_id:'',name:'',url:'',default_branch:''}})}><option value="connection">{hostRepositoryText("repository.030")}</option>{(options.length>0||mode==='legacy')&&<option value="legacy">{hostRepositoryText("repository.031")}</option>}</select></AutoField>
    {mode==='connection'?<ConnectionRepositoryFields navigation={navigation} workspace={workspace} data={data} connectionId={selection?.connection_id??''} repositoryUrl={selection?.url??''} repositoryLabel={hostRepositoryText("repository.032")} onChange={value=>onChange({repository_id:'',repository_selection:value})}/>:<AutoField label={<span className="text-sm">{hostRepositoryText("repository.033")}</span>} className="block space-y-2"><select aria-label={hostRepositoryText("repository.033")} className={selectClass} value={legacyId} onChange={event=>onChange({repository_id:event.target.value,repository_selection:null})}>{!options.some(row=>row.value===legacyId)&&<option value={legacyId}>{named(undefined,legacyId)}</option>}{options.map(row=><option key={row.value} value={row.value}>{row.label}</option>)}</select></AutoField>}
  </section>;
}
