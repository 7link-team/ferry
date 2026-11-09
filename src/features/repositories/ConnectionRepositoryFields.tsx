import { LoadingSkeleton } from "../../shared/ui/loading-skeleton";
import { Dialog } from "radix-ui";
import { useEffect,useState,type ReactNode } from "react";
import { api,errorText } from "../../shared/api/api";
import { AutoField, AutoForm } from "../../shared/forms/auto-form";
import { SelectRefresh } from "../../shared/forms/SelectRefresh";
import { commonText,hostRepositoryText,useCommonTranslation,workflowText as t } from "../../shared/i18n/i18n";
import { currentLocale } from "../../shared/i18n/locale";
import type { Navigation } from "../../shared/routing/types";
import { Button } from "../../shared/ui/button";
import { Input } from "../../shared/ui/input";
import { RepositorySource, repositoryProviderLabel } from "../repositories/RepositorySource";

type Connection = {id:string;name:string;provider:string;state:string;url?:string};
type Remote = {name:string;full_name:string;url:string;default_branch:string;remote_id?:string};
const selectClass="w-full min-w-0 rounded-md border border-input bg-background px-3 py-2 text-sm";
export type RepositorySelection={connection_id:string;name:string;url:string;default_branch:string};
export function useConnectionRepositories(workspace:string,connectionId:string|undefined,enabled=true,allPages=false) {
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
    void (async()=>{
      const collected:Remote[]=[];
      const seen=new Set<string>();
      let nextPage=allPages?1:page;
      for(;;nextPage++){
        const result=await api<{items:Remote[];has_more:boolean}>(`${root}/${encodeURIComponent(connection.id)}/repositories?page=${nextPage}`,{signal:controller.signal});
        if(allPages&&result.has_more&&!result.items.length)throw new Error(commonText("repository.pageEmpty"));
        for(const row of result.items)if(!seen.has(row.url)){seen.add(row.url);collected.push(row);}
        if(!allPages||!result.has_more){
          if(!controller.signal.aborted){
            setRepositories(previous=>allPages||page===1?collected:[...previous,...collected.filter(row=>!previous.some(existing=>existing.url===row.url))]);
            setHasMore(!allPages&&result.has_more);
          }
          break;
        }
      }
    })().catch(cause=>{if(!controller.signal.aborted)setError(errorText(cause));})
      .finally(()=>{if(!controller.signal.aborted)setLoading(false);});
    return()=>controller.abort();
  },[root,connection?.id,connection?.provider,page,revision,enabled,allPages]);
  return {connections,repositories,connection,root,error:connectionError||error,loading,connectionsLoading,page,hasMore,setPage,setRevision};
}

/** Both workflow editors use this picker and the existing connection form/auth lifecycle. */
export function RepositoryConnectionSelect({workspace,data,value,onChange,provider,readOnly=false,canCreate=true}:{
  workspace:string;data:ReturnType<typeof useConnectionRepositories>;value:string;
  onChange:(id:string,connection?:Connection)=>void;provider?:string;readOnly?:boolean;canCreate?:boolean;
}) {
  const [creating,setCreating]=useState(false);
  const [added,setAdded]=useState<Connection>();
  const connections=data.connections.filter(row=>!provider||row.provider===provider);
  const rows=added&&!connections.some(row=>row.id===added.id)&&(!provider||added.provider===provider)?[...connections,added]:connections;
  return <div className="min-w-0 space-y-2">
    <AutoField label={t("integration.connection")} className="block space-y-2">
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        <SelectRefresh label={t("integration.refresh")} loading={data.connectionsLoading} onRefresh={()=>data.setRevision(n=>n+1)} className="min-w-0 flex-1">
          <select aria-label={t("integration.connection")} className={selectClass} value={value==='0'?'':value} disabled={readOnly||data.connectionsLoading} onChange={event=>onChange(event.target.value,rows.find(row=>row.id===event.target.value))}>
            <option value="">{data.connectionsLoading?commonText("common.loading"):hostRepositoryText("repository.018")}</option>
            {value&&value!=='0'&&!rows.some(row=>row.id===value)&&<option value={value}>{commonText("form.unavailableReference",{id:value})}</option>}
            {rows.map(row=><option key={row.id} value={row.id} disabled={!['connected','refreshing'].includes(row.state)}>{row.name} · {repositoryProviderLabel(row.provider)}{!['connected','refreshing'].includes(row.state)?hostRepositoryText("repository.019"):''}</option>)}
          </select>
        </SelectRefresh>
        {!readOnly&&canCreate&&<Button type="button" variant="outline" className="min-h-11" onClick={()=>setCreating(true)}>{hostRepositoryText("repository.020")}</Button>}
      </div>
    </AutoField>
    {!data.connectionsLoading&&!data.error&&!rows.length&&<p role="status" className="text-xs text-muted-foreground">{hostRepositoryText("repository.022")}</p>}
    {creating&&<CreateRepositoryConnection workspace={workspace} {...(provider?{provider}:{})} onClose={()=>setCreating(false)} onCreated={row=>{setAdded(row);data.setRevision(n=>n+1);onChange(row.id,row);setCreating(false);}}/>}
  </div>;
}

function CreateRepositoryConnection({workspace,provider,onClose,onCreated}:{workspace:string;provider?:string;onClose:()=>void;onCreated:(row:Connection)=>void}) {
  const [draft,setDraft]=useState<Record<string,unknown>>({name:'',provider:provider??'',credential_id:'0',base_url:'',api_url:'',organization:'',url:'',description:''});
  const [blocked,setBlocked]=useState(true),[busy,setBusy]=useState(false),[error,setError]=useState('');
  const update=(patch:Record<string,unknown>)=>setDraft(current=>({...current,...patch}));
  const credentialHref=`/${currentLocale()}/workspaces/${encodeURIComponent(workspace)}/credentials/new?scope=build`;
  const authorized=async(id:string)=>{setBusy(true);setError('');try{onCreated(await api<Connection>(`/workspaces/${workspace}/code-connections/${encodeURIComponent(id)}`));}catch(cause){setError(errorText(cause));}finally{setBusy(false);}};
  return <Dialog.Root open onOpenChange={open=>{if(!open&&!busy)onClose();}}><Dialog.Portal>
    <Dialog.Overlay className="fixed inset-0 z-50 bg-black/40"/>
    <Dialog.Content className="fixed left-1/2 top-1/2 z-50 max-h-[90dvh] w-[min(40rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 dialog-shell rounded-xl border bg-background p-5 shadow-xl">
      <Dialog.Title className="text-lg font-semibold">{hostRepositoryText("repository.020")}</Dialog.Title>
      <Dialog.Description className="text-sm text-muted-foreground">{t("integration.connectionHelp")}</Dialog.Description>
      <AutoForm className="dialog-form mt-4" onSubmit={async event=>{event.preventDefault();event.stopPropagation();if(blocked||busy)return;setBusy(true);setError('');try{const row=await api<Connection>(`/workspaces/${workspace}/code-connections`,{method:'POST',body:JSON.stringify(draft)});onCreated({...row,url:String(draft.url??'')});}catch(cause){setError(errorText(cause));}finally{setBusy(false);}}}>
        <fieldset disabled={busy} className="dialog-body min-w-0 space-y-4">
          <AutoField label={commonText("resource-domain:repository.connectionName")}><Input aria-label={commonText("resource-domain:repository.connectionName")} required maxLength={200} value={String(draft.name)} onChange={event=>update({name:event.target.value})}/></AutoField>
          <RepositorySource credentialHref={credentialHref} fixedProvider={!!provider} workspace={workspace} draft={draft} readOnly={false} onChange={update} onValidityChange={setBlocked} onAuthorized={id=>void authorized(id)}/>
          {error&&<p role="alert" className="text-sm text-destructive">{error}</p>}
        </fieldset>
        <div className="dialog-footer mt-4"><Button type="button" variant="outline" disabled={busy} onClick={onClose}>{commonText("common.cancel")}</Button><Button type="submit" disabled={busy||blocked||!String(draft.name).trim()}>{busy?commonText("common.saving"):commonText("common.create")}</Button></div>
      </AutoForm>
    </Dialog.Content>
  </Dialog.Portal></Dialog.Root>;
}
export function ConnectionRepositoryFields({ canCreateConnection=true,workspace,data,connectionId,repositoryUrl,repositoryLabel,onChange,group="",onGroupChange,repositoryPicker,gitAction}:{ canCreateConnection?:boolean;navigation: Navigation;workspace:string;data:ReturnType<typeof useConnectionRepositories>;connectionId:string;repositoryUrl:string;repositoryLabel:string;onChange:(value:RepositorySelection)=>void;group?:string;onGroupChange?:(group:string)=>void;repositoryPicker?:ReactNode;gitAction?:ReactNode}) {
  useCommonTranslation();
  const {connections,connection,repositories,error,loading,page,hasMore,setPage}=data;
  const groups=[...new Set(repositories.flatMap(row=>{
    const parts=row.full_name.split('/').filter(Boolean);parts.pop();
    return parts.map((_,index)=>parts.slice(0,index+1).join('/'));
  }))].sort((a,b)=>a.localeCompare(b));
  const visibleRepositories=group?repositories.filter(row=>row.full_name.startsWith(group+'/')):repositories;
  return <>
    <div className="min-w-0 sm:col-span-2"><RepositoryConnectionSelect canCreate={canCreateConnection} workspace={workspace} data={data} value={connectionId} onChange={(id,created)=>{const selected=created??connections.find(row=>row.id===id);onChange({connection_id:id,name:selected?.name??'',url:selected?.provider==='git'?selected.url??'':'',default_branch:''});}}/></div>
    {data.connectionsLoading&&!connections.length&&<LoadingSkeleton variant="inline" label={hostRepositoryText("repository.loadingConnections")} />}
    {!data.connectionsLoading&&!error&&!connections.length&&<p className="text-xs text-muted-foreground">{hostRepositoryText("repository.022")}</p>}
    {onGroupChange&&connection&&connection.provider!=='git'&&<AutoField label={hostRepositoryText("repository.group")} className="block space-y-2"><select aria-label={hostRepositoryText("repository.group")} className={selectClass} value={group} disabled={loading||!!error} onChange={event=>onGroupChange(event.target.value)}><option value="">{hostRepositoryText("repository.allGroups")}</option>{groups.map(name=><option key={name} value={name}>{name}</option>)}</select></AutoField>}
    {connection?.provider==='git'?<AutoField label={<span className="text-sm">{hostRepositoryText("repository.023")}</span>} className="block space-y-2"><div className="flex min-w-0 items-center gap-2"><Input className="min-w-0 flex-1" aria-label={hostRepositoryText("repository.023")} value={repositoryUrl} onChange={event=>onChange({connection_id:connectionId,name:connection.name,url:event.target.value,default_branch:''})}/>{gitAction}</div></AutoField>:connectionId&&connectionId!=='0'&&<AutoField label={<span className="text-sm">{hostRepositoryText("repository.004")}</span>} className="block space-y-2">{repositoryPicker??<select aria-label={repositoryLabel} className={selectClass} value={repositoryUrl} disabled={loading&&page===1} onChange={event=>{const selected=repositories.find(row=>row.url===event.target.value);onChange({connection_id:connectionId,name:selected?.name??'',url:event.target.value,default_branch:selected?.default_branch??''});}}><option value="">{loading?hostRepositoryText("repository.024"):hostRepositoryText("repository.025")}</option>{repositoryUrl&&!repositories.some(row=>row.url===repositoryUrl)&&<option value={repositoryUrl}>{repositoryUrl}</option>}{visibleRepositories.map(row=><option key={row.url} value={row.url}>{row.full_name||row.name}</option>)}</select>}</AutoField>}
    {hasMore&&!error&&<Button type="button" variant="outline" disabled={loading} onClick={()=>setPage(value=>value+1)}>{hostRepositoryText("repository.026")}</Button>}
    {loading&&!repositories.length&&<LoadingSkeleton variant="inline" label={hostRepositoryText("repository.024")} />}
    {connection&&connection.provider!=='git'&&!loading&&!error&&!repositories.length&&<p role="status" className="text-sm text-muted-foreground">{hostRepositoryText("repository.noVisibleRepositories")}</p>}
    {error&&<p role="alert" className="text-sm text-destructive">{error}</p>}
  </>;
}
