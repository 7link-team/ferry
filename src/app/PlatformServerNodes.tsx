import { commonText as t } from "../shared/i18n/i18n";
import type { ListConfig } from "../shared/resources/types";
import { ResourceTable } from "./resources/ResourceTable";
import { AutoTable } from "../shared/tables/auto-table";
import { Dialog } from "radix-ui";
import { Button } from "../shared/ui/button";

type Partition = {name:string;mount:string;filesystem:string;kind?:string;total:number;available:number;used:number;usage_percent:number|null;data_disk:boolean};
type ServerNode = {worker_id:number;instance_id:string;heartbeat_at:number;online:boolean;current:boolean;info:{hostname?:string;addresses?:string[];bind?:string;version?:string;os?:string;arch?:string;resources?:{cpu_cores?:number;cpu_logical_processors?:number;cpu_physical_cores?:number|null;cpu_packages?:number|null;cpu_model?:string;cpu_percent?:number;memory_total?:number;memory_available?:number;disk_total?:number|null;disk_available?:number|null;disk_count?:number;disks?:Partition[];sampled_at?:number}}};
const capacity=(n?:number|null)=>n==null?"—":`${(n/1024/1024/1024).toLocaleString(undefined,{maximumFractionDigits:1})} GiB`;
const nodeText=(key:string,values?:Record<string,unknown>)=>t(`resource-domain:platform.server.${key}`,values);
function Partitions({node}:{node:ServerNode}) {
  const disks=node.info.resources?.disks;
  if (!disks) return null;
  return <Dialog.Root><Dialog.Trigger asChild><Button variant="link" size="sm" className="h-auto px-0">{nodeText("partitionsCount",{count:disks.length})}</Button></Dialog.Trigger><Dialog.Portal><Dialog.Overlay className="fixed inset-0 z-50 bg-black/40"/><Dialog.Content className="fixed left-1/2 top-1/2 z-50 max-h-[85dvh] w-[min(64rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 dialog-shell rounded-lg border bg-background p-6 shadow-lg">
    <Dialog.Title className="text-lg font-semibold">{nodeText("partitions")} · {node.info.hostname || node.instance_id}</Dialog.Title>
    <Dialog.Description className="mb-4 mt-2 text-sm text-muted-foreground">{nodeText("samplingHelp")}</Dialog.Description>
    <div className="dialog-body"><AutoTable data={disks} rowKey={disk=>disk.mount} pageSize={10} columns={[
      {field:"mount",name:nodeText("mount"),render:(_value,disk:Partition)=><div>{disk.mount}{disk.data_disk&&<p className="text-xs text-muted-foreground">{nodeText("dataPartition")}</p>}</div>},
      {field:"name",name:nodeText("volume")},{field:"filesystem",name:nodeText("filesystem")},
      {field:"kind",name:nodeText("diskKind"),render:value=>nodeText(value==="ssd"?"diskSsd":value==="hdd"?"diskHdd":"diskUnknown")},
      ...(["total","used","available"] as const).map(field=>({field,name:nodeText(field),render:(value:unknown)=>capacity(value as number)})),
      {field:"usage_percent",name:nodeText("usage"),render:value=>value==null?"—":`${Number(value).toFixed(1)}%`},
    ]}/></div>
    <div className="dialog-footer mt-4"><Dialog.Close asChild><Button variant="outline">{t("common.close")}</Button></Dialog.Close></div>
  </Dialog.Content></Dialog.Portal></Dialog.Root>;
}
const permissions=new Set<string>();
const config:ListConfig<ServerNode>={
  resource:"server-nodes",endpoint:"/platform/server-nodes",queryPrefix:"server_nodes",rowKey:node=>String(node.worker_id),
  footerSummary:data=><span className="text-muted-foreground">{nodeText("onlineTotal",{online:(data.coverage as {online?:number}|undefined)?.online??0,total:data.total})}</span>,
  columns:[
    {key:"instance_id",label:"Server / workerId",labelKey:"resource-domain:platform.server.node",render:node=><div>{node.info.hostname || node.instance_id}<p className="text-xs text-muted-foreground">workerId {node.worker_id} · {t(`resource-domain:platform.server.${node.online?'online':'offline'}`)}{node.current ? ` · ${t("resource-domain:platform.server.current")}` : ""}</p></div>},
    {key:"address",label:"Address",labelKey:"resource-domain:platform.server.address",width:210,render:node=><div className="max-w-52 whitespace-normal break-all">{node.info.addresses?.join(" · ") || "—"}<p className="text-xs text-muted-foreground">{nodeText("bind")} {node.info.bind || "—"}</p></div>},
    {key:"version",label:"Version",labelKey:"resource-domain:platform.server.version",render:node=><div>{node.info.version || "—"}<p className="text-xs text-muted-foreground">{node.info.os} / {node.info.arch}</p></div>},
    {key:"heartbeat_at",label:"Last heartbeat",labelKey:"resource-domain:platform.server.heartbeat",render:node=>node.heartbeat_at?new Date(node.heartbeat_at).toLocaleString():"—"},
    {key:"cpu",label:"CPU",width:230,render:node=>node.info.resources?<div>{node.info.resources.cpu_percent?.toFixed(1)}%<p className="text-xs text-muted-foreground">{nodeText("cpuTopology",{packages:node.info.resources.cpu_packages??"—",cores:node.info.resources.cpu_physical_cores??"—",logical:node.info.resources.cpu_logical_processors??node.info.resources.cpu_cores??"—"})}</p><p className="max-w-56 whitespace-normal break-words text-xs text-muted-foreground">{node.info.resources.cpu_model}</p></div>:"—"},
    {key:"memory",label:"Memory",labelKey:"resource-domain:monitor.memory",width:190,render:node=><div>{capacity(node.info.resources?.memory_available)} / {capacity(node.info.resources?.memory_total)}<p className="text-xs text-muted-foreground">{t("resource-domain:monitor.availableTotal")}</p></div>},
    {key:"disk",label:"Disk",labelKey:"resource-domain:monitor.disk",width:190,render:node=><div>{capacity(node.info.resources?.disk_available)} / {capacity(node.info.resources?.disk_total)}<p className="text-xs text-muted-foreground">{t("resource-domain:monitor.availableTotal")}</p><Partitions node={node}/></div>},
    {key:"sampled_at",label:"Sampled at",labelKey:"resource-domain:platform.server.sampledAt",render:node=><div>{node.info.resources?.sampled_at?new Date(node.info.resources.sampled_at).toLocaleString():"—"}<p className="text-xs text-muted-foreground">{node.online?nodeText("heartbeatSample"):t("resource-domain:monitor.lastSample")}</p></div>},
  ],
};
export function PlatformServerNodes() {
  return <section className="flex min-h-0 flex-1 flex-col gap-4"><p className="text-sm text-muted-foreground">{t("resource-domain:platform.serverNodesHelp")} {nodeText("samplingHelp")}</p><ResourceTable config={config} workspace="_platform" permissions={permissions}/></section>;
}
