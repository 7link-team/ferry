import { useEffect,useState } from "react";
import { api,errorText } from "../../shared/api/api";
import { named } from "../../shared/api/resource";
import { AutoField } from "../../shared/forms/auto-form";
import { hostRepositoryText,useCommonTranslation } from "../../shared/i18n/i18n";
import type { Navigation } from "../../shared/routing/types";
import { Button } from "../../shared/ui/button";
import { Input } from "../../shared/ui/input";
import { repositoryProviderLabel } from "../repositories/RepositorySource";

type Connection = {id:string;name:string;provider:string;state:string;url?:string};
type Remote = {name:string;full_name:string;url:string;default_branch:string};
const selectClass="w-full min-w-0 rounded-md border border-input bg-background px-3 py-2 text-sm";
export type RepositorySelection={connection_id:string;name:string;url:string;default_branch:string};
export function useConnectionRepositories(workspace:string,connectionId:string|undefined,enabled=true) {
  const [connections,setConnections]=useState<Connection[]>([]),[repositories,setRepositories]=useState<Remote[]>([]);
  const [connectionsLoading,setConnectionsLoading]=useState(false),[connectionError,setConnectionError]=useState("");
  const [error,setError]=useState(""),[loading,setLoading]=useState(false),[page,setPage]=useState(1),[hasMore,setHasMore]=useState(false),[revision,setRevision]=useState(0);
  const root=`/workspaces/${encodeURIComponent(workspace)}/code-connections`;
  const connection=connections.find(row=>row.id===connectionId);
  useEffect(()=>{
    if(!enabled)return;
    const controller=new AbortController();setConnectionError("");setConnectionsLoading(true);
    void (async()=>{const all:Connection[]=[];for(let page=1;;page++){const data=await api<{items:Connection[];total:number;page_size:number}>(`${root}?page=${page}&page_size=200`,{signal:controller.signal});all.push(...data.items);if(!data.items.length||page*data.page_size>=data.total)break;}if(!controller.signal.aborted)setConnections(all);})().catch(cause=>{if(!controller.signal.aborted)setConnectionError(errorText(cause));}).finally(()=>{if(!controller.signal.aborted)setConnectionsLoading(false);});
    return()=>controller.abort();
  },[root,revision,enabled]);
  useEffect(()=>{
    setRepositories([]);setPage(1);setHasMore(false);
  },[connectionId]);
  useEffect(()=>{
    if(!enabled || !connection || connection.provider==='git'){setLoading(false);setError("");return;}
    const controller=new AbortController();setLoading(true);setError("");
    void api<{items:Remote[];has_more:boolean}>(`${root}/${encodeURIComponent(connection.id)}/repositories?page=${page}`,{signal:controller.signal})
      .then(value=>{if(!controller.signal.aborted){setRepositories(previous=>page===1?value.items:[...previous,...value.items.filter(row=>!previous.some(existing=>existing.url===row.url))]);setHasMore(value.has_more);}})
      .catch(cause=>{if(!controller.signal.aborted)setError(errorText(cause));}).finally(()=>{if(!controller.signal.aborted)setLoading(false);});
    return()=>controller.abort();
  },[root,connection?.id,connection?.provider,page,revision,enabled]);
  return {connections,repositories,connection,root,error:connectionError||error,loading,connectionsLoading,page,hasMore,setPage,setRevision};
}
export function ConnectionRepositoryFields({ navigation,workspace,data,connectionId,repositoryUrl,repositoryLabel,onChange}:{ navigation: Navigation;workspace:string;data:ReturnType<typeof useConnectionRepositories>;connectionId:string;repositoryUrl:string;repositoryLabel:string;onChange:(value:RepositorySelection)=>void}) {
  useCommonTranslation();
  const {connections,connection,repositories,error,loading,page,hasMore,setPage,setRevision}=data;
  return <>
    <AutoField label={<span className="text-sm">{hostRepositoryText("repository.002")}</span>} className="block space-y-2 sm:col-span-2"><div className="flex min-w-0 flex-col gap-2 sm:flex-row sm:items-center"><select aria-label={hostRepositoryText("repository.002")} className={`${selectClass} sm:flex-1 sm:w-auto`} value={connectionId==='0'?'':connectionId} onChange={event=>{const selected=connections.find(row=>row.id===event.target.value);onChange({connection_id:event.target.value,name:selected?.name??'',url:selected?.provider==='git'?selected.url??'':'',default_branch:''});}}><option value="">{hostRepositoryText("repository.018")}</option>{connectionId&&connectionId!=='0'&&!connections.some(row=>row.id===connectionId)&&<option value={connectionId}>{named(undefined,connectionId)}</option>}{connections.map(row=><option key={row.id} value={row.id}>{named(row.name,row.id)} · {repositoryProviderLabel(row.provider)}{row.state!=='connected'?hostRepositoryText("repository.019"):''}</option>)}</select><div className="flex shrink-0 items-center gap-2"><Button type="button" size="sm" variant="outline" asChild><a href={navigation.href({workspace,group:"repository",item:"repositories",mode:"new",id:""},{})} target="_blank" rel="noopener noreferrer">{hostRepositoryText("repository.020")}</a></Button><Button type="button" size="sm" variant="ghost" onClick={()=>setRevision(value=>value+1)}>{hostRepositoryText("repository.021")}</Button></div></div></AutoField>
    {data.connectionsLoading&&<p role="status" className="text-sm text-muted-foreground">{hostRepositoryText("repository.loadingConnections")}</p>}
    {!data.connectionsLoading&&!error&&!connections.length&&<p className="text-xs text-muted-foreground">{hostRepositoryText("repository.022")}</p>}
    {connection?.provider==='git'?<AutoField label={<span className="text-sm">{hostRepositoryText("repository.023")}</span>} className="block space-y-2"><Input aria-label={hostRepositoryText("repository.023")} value={repositoryUrl} onChange={event=>onChange({connection_id:connectionId,name:connection.name,url:event.target.value,default_branch:''})}/></AutoField>:connectionId&&connectionId!=='0'&&<AutoField label={<span className="text-sm">{hostRepositoryText("repository.004")}</span>} className="block space-y-2"><select aria-label={repositoryLabel} className={selectClass} value={repositoryUrl} disabled={loading&&page===1} onChange={event=>{const selected=repositories.find(row=>row.url===event.target.value);onChange({connection_id:connectionId,name:selected?.name??'',url:event.target.value,default_branch:selected?.default_branch??''});}}><option value="">{loading?hostRepositoryText("repository.024"):hostRepositoryText("repository.025")}</option>{repositoryUrl&&!repositories.some(row=>row.url===repositoryUrl)&&<option value={repositoryUrl}>{repositoryUrl}</option>}{repositories.map(row=><option key={row.url} value={row.url}>{row.full_name||row.name}</option>)}</select></AutoField>}
    {hasMore&&!error&&<Button type="button" variant="outline" disabled={loading} onClick={()=>setPage(value=>value+1)}>{hostRepositoryText("repository.026")}</Button>}
    {loading&&<p role="status" className="text-sm text-muted-foreground">{hostRepositoryText("repository.024")}</p>}
    {connection&&connection.provider!=='git'&&!loading&&!error&&!repositories.length&&<p role="status" className="text-sm text-muted-foreground">{hostRepositoryText("repository.noVisibleRepositories")}</p>}
    {error&&<div className="space-y-2"><p role="alert" className="text-sm text-destructive">{error}</p><Button type="button" variant="outline" disabled={loading} onClick={()=>setRevision(value=>value+1)}>{hostRepositoryText("repository.027")}</Button></div>}
  </>;
}
