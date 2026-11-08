import { Dialog } from "radix-ui";
import { commonText as t } from "../shared/i18n/i18n";
import type { ListConfig } from "../shared/resources/types";
import { Button } from "../shared/ui/button";
import { FileContentPreview, supportsFilePreview } from "../features/workflows/WorkflowAssets";
import { ResourceTable } from "./resources/ResourceTable";
import { href } from "./router";

export type FileRow={id:string;workspace_id:string;workspace:string;name:string;sha256:string;mime:string;size:number;created_at:number;kind:"asset"|"artifact";workflow_id:string|null;run_id:string|null};
const text=(key:string)=>t(`resource-domain:monitor.${key}`);
export function fileSource(row: FileRow) {
  if (row.kind === "artifact" && row.run_id) return href({workspace:row.workspace_id,group:"workflow",item:"runs",id:row.run_id,mode:"view"},{});
  if (row.kind === "asset" && row.workflow_id) return href({workspace:row.workspace_id,group:"workflow",item:"workflows",id:row.workflow_id,mode:"view"},row.run_id ? {execution:row.run_id} : {});
  return null;
}
export function FileActions({row}:{row:FileRow}) {
  const url = row.kind === "asset" ? `/api/platform/monitor/files/catalog/${row.id}/content` : `/api/workspaces/${row.workspace_id}/pipeline-artifacts/${row.id}/content`;
  const source = fileSource(row);
  return <div className="flex flex-wrap items-center gap-3">
    {supportsFilePreview(row.mime) && <Dialog.Root><Dialog.Trigger asChild><Button variant="link" size="sm" className="h-auto px-0">{text("filePreview")}</Button></Dialog.Trigger><Dialog.Portal><Dialog.Overlay className="fixed inset-0 z-50 bg-black/40"/><Dialog.Content className="fixed left-1/2 top-1/2 z-50 max-h-[85dvh] w-[min(48rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 dialog-shell rounded-lg border bg-background p-4 sm:p-6 shadow-lg">
      <Dialog.Title className="break-all font-semibold">{text("filePreview")}</Dialog.Title><Dialog.Description className="my-3 break-all text-sm text-muted-foreground">{row.name}</Dialog.Description>
      <div className="dialog-body"><FileContentPreview file={row} url={url} showDownload={false}/></div>
      <div className="dialog-footer mt-4 flex flex-wrap gap-3"><a className="text-sm text-primary underline" href={url} download={row.name}>{t("workflow-domain:downloadFile")}</a><Dialog.Close asChild><Button variant="outline">{t("common.close")}</Button></Dialog.Close></div>
    </Dialog.Content></Dialog.Portal></Dialog.Root>}
    <a className="text-sm text-primary underline" href={url} download={row.name}>{t("workflow-domain:downloadFile")}</a>
    {source ? <a className="text-sm text-primary underline" href={source} target="_blank" rel="noopener noreferrer">{text("openSource")}</a> : <span className="text-sm text-muted-foreground">{text("fileSourceUnavailable")}</span>}
  </div>;
}
const config:ListConfig<FileRow>={
  resource:"platform-files",endpoint:"/platform/monitor/files/catalog",queryPrefix:"file_catalog",rowKey:row=>`${row.kind}:${row.id}`,
  filters:[{key:"q",label:"Search files",labelKey:"resource-domain:monitor.fileSearch",type:"text"},{key:"kind",label:"Type",labelKey:"resource-domain:monitor.kind",type:"select",options:[{value:"asset",label:"Content file",labelKey:"resource-domain:monitor.contentFiles"},{value:"artifact",label:"Build artifact",labelKey:"resource-domain:monitor.artifacts"}]}],
  toolbar:{create:false,import:false,export:false},
  columns:[
    {key:"name",label:"File / content identifier",labelKey:"resource-domain:monitor.fileName",render:row=><span className="block max-w-72 whitespace-normal break-all">{row.name}</span>},
    {key:"workspace",label:"Workspace",labelKey:"resource-domain:platform.workspaces"},
    {key:"kind",label:"Kind",labelKey:"resource-domain:monitor.kind",render:row=>text(row.kind==="asset"?"contentFiles":"artifacts")},
    {key:"mime",label:"MIME",labelKey:"resource-domain:monitor.mime"},
    {key:"size",label:"Size",labelKey:"resource-domain:monitor.bytes",render:row=>`${(row.size/1024).toLocaleString(undefined,{maximumFractionDigits:1})} KiB`},
    {key:"created_at",label:"Created at",labelKey:"resource-domain:monitor.created_at",render:row=>new Date(row.created_at).toLocaleString()},
    {key:"actions",label:"Actions",labelKey:"common-domain:table.actions",render:row=><FileActions row={row}/>},
  ],
};
export function PlatformFileCatalog(){return <div className="space-y-4"><p className="text-sm text-muted-foreground">{text("catalogHelp")}</p><ResourceTable config={config} workspace="_platform" permissions={new Set()}/></div>;}
