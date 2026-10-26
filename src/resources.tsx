import { importResources, exportResources } from "./components/ResourceTransfer";
import { api } from "./api";
/**
 * Every resource page in one place: a `ListConfig` and a `FormConfig` per
 * resource. This is the whole of a page — there is no per-resource component.
 *
 * A page absent from here renders the "not built yet" placeholder, so adding a
 * module means adding entries here plus the server handlers.
 */
import { duration, stateBadge, type PipelineRun } from "./pipeline";
import { deploymentState, type Deployment, type Revision } from "./deployment";
import { nav } from "./nav";
import { navigate } from "./router";
import { Badge } from "@/components/ui/badge";
import { deleteResource, listResource, runAction, runItemAction } from "./resource";
import type { FormConfig, ListConfig, Option } from "./components/dsl";

/* The registries are keyed by resource name, so their values have to share one
   type while each config stays checked against its own row type. These two
   helpers do exactly that: the argument is checked, the return is erased. */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type AnyRow = any;
const list = <T,>(config: ListConfig<T>): ListConfig<AnyRow> => ({ ...config, toolbar: { ...config.toolbar, import: !config.endpoint && importResources.has(config.resource), export: !config.endpoint && exportResources.has(config.resource) } });
const form = <T,>(config: FormConfig<T>): FormConfig<AnyRow> => config;

export interface HostGroup {
  id: string;
  name: string;
  description: string;
  parent_id: string;
  version: number;
}
export interface Host {
  id: string;
  name: string;
  address: string;
  port: number;
  username: string;
  connection: string;
  state: string;
  group_id: string;
  tag_ids: string[];
  tags: { id: string; name: string; color: string }[];
  credential_id: string;
  description: string;
  seen_at: number;
  version: number;
}
export interface Activity {
  id: string;
  actor_name: string;
  action: string;
  resource: string;
  resource_id: string;
  details: unknown;
  created_at: number;
}
export interface Credential {
  id: string;
  name: string;
  kind: string;
  username: string;
  description: string;
  /** The server reports only whether a secret is set, never its value. */
  has_secret: boolean;
  /** Write-only: sent when creating or rotating, never returned by any read. */
  secret?: string;
  version: number;
}
export interface Variable {
  id: string; name: string; scope: string; scope_ref: string; kind: string;
  description: string; has_value: boolean; value?: string; version: number;
}
const VARIABLE_SCOPES = [{ value: "global", label: "全局" }, { value: "deploy", label: "部署" }, { value: "build", label: "构建" }];
const VARIABLE_KINDS = [{ value: "secret", label: "密钥" }, { value: "plain", label: "普通" }];
const variableSource = (row: Variable) => row.scope === "global" ? "全局" : `${label(VARIABLE_SCOPES, row.scope)} · ${row.scope_ref === "0" ? "模块默认" : `对象 ${row.scope_ref}`}`;
export interface Tag {
  id: string;
  name: string;
  color: string;
  version: number;
}

const CREDENTIAL_KINDS: Option[] = [
  { value: "password", label: "密码" },
  { value: "ssh-key", label: "SSH 私钥" },
  { value: "token", label: "访问令牌" },
];
const CONNECTIONS: Option[] = [
  { value: "runner", label: "代理（runner）" },
  { value: "ssh", label: "SSH 直连" },
];
const STATES: Option[] = [
  { value: "online", label: "在线" },
  { value: "offline", label: "离线" },
  { value: "unknown", label: "未知" },
];

const label = (options: Option[], value: string) =>
  options.find((option) => option.value === value)?.label ?? value;

/** Relative time; absolute dates make an operator do arithmetic. */
function ago(at: number) {
  if (!at) return "—";
  const seconds = Math.max(0, Math.round((Date.now() - at) / 1000));
  if (seconds < 60) return `${seconds} 秒前`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)} 分钟前`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)} 小时前`;
  return `${Math.floor(seconds / 86400)} 天前`;
}

/** Group options for the filter and the form, loaded once per render pass. */
/** SSH login needs a password or a key; a token is for Git, not for sshd. */
const sshCredentialOptions = (workspace: string) => async (): Promise<Option[]> => {
  const page = await listResource<Credential>(workspace, "credentials", { page_size: 200 });
  return page.items
    .filter((c) => c.kind === "password" || c.kind === "ssh-key")
    .map((c) => ({ value: c.id, label: `${c.name}（${label(CREDENTIAL_KINDS, c.kind)}）` }));
};

const groupOptions = (workspace: string) => namedOptions(workspace, "host-groups");
const tagOptions = (workspace: string) => namedOptions(workspace, "tags");

interface Repository {
  id: string; name: string; url: string; default_branch: string;
  credential_id: string; description: string;
}
interface Pipeline {
  id: string; name: string; repository_id: string; target_kind: string;
  target_ref: string; definition: string; description: string;
}
const namedOptions = (workspace: string, resource: string) => async (): Promise<Option[]> => {
  const items: Option[] = [];
  for (let page = 1; ; page += 1) {
    const result = await listResource<{ id: string; name: string }>(workspace, resource, { page, page_size: 200 });
    items.push(...result.items.map((row) => ({ value: row.id, label: row.name })));
    if (!result.items.length || page * result.page_size >= result.total) return items;
  }
};
const TARGETS = [{ value: "host", label: "主机" }, { value: "host-group", label: "主机组" }];

interface Template { id: string; name: string; description: string; content: string; parameters: string; version: number }
interface TemplateParameter { name: string; label?: string; type: "string" | "integer" | "boolean"; default?: unknown; enum?: string[]; min?: number; max?: number; min_length?: number; max_length?: number; secret?: boolean }
const templateList = (workspace: string, resource: string, target: string, group: string, item: string) => list<Template>({
  resource, rowKey: r => r.id, filters: [{ key: "q", label: "名称 / 说明", type: "text" }],
  columns: [{ key: "name", label: "名称", sortable: true }, { key: "description", label: "说明" }, { key: "version", label: "版本" }],
  actions: [
    { label: "查看", to: r => ({ id: r.id, mode: "view" }) },
    { label: "使用模板", permission: `${target}.create`, to: () => ({ group, item, mode: "new" }), query: r => ({ template: r.id }) },
    { label: "复制", permission: `${resource}.create`, to: () => ({ mode: "new" }), query: r => ({ copy: r.id }) },
    { label: "编辑", permission: `${resource}.update`, to: r => ({ id: r.id, mode: "edit" }) },
    { label: "删除模板", permission: `${resource}.delete`, destructive: true, action: r => deleteResource(workspace, resource, r.id) },
  ], toolbar: { create: true, import: false, export: false },
});
const templateForm = (resource: string, application: boolean) => form<Template>({
  resource, heading: r => r.name,
  blank: { name: "", description: "", content: resource === "config-templates" ? "port={{ .Values.port }}\n" : application ? JSON.stringify({apiVersion:"ferry/v1",kind:"Application",metadata:{name:"web"},services:[{name:"web",image:"{{ .Values.image }}"}]},null,2) : JSON.stringify({workdir:"/tmp/build/{{run}}",steps:[{name:"构建",run:"{{ .Values.command }}"}]},null,2),
    parameters: resource === "config-templates" ? JSON.stringify([{name:"port",type:"integer",default:8080,min:1,max:65535}],null,2) : JSON.stringify([{name: application ? "image" : "command",label: application ? "镜像" : "构建命令",type:"string",default:application ? "nginx:alpine" : "echo build",min_length:1}],null,2) },
  copy: r => ({ name: `${r.name}（副本）`, description: r.description, content: r.content, parameters: r.parameters }),
  sections: [{ fields: [
    { key: "name", label: "名称", type: "text", required: true },
    { key: "description", label: "说明", type: "textarea" },
    { key: "parameters", label: "参数定义（JSON 数组）", type: "code", span: 2, required: true, help: '每项包含 name、type（string/integer/boolean），可选 label、default、enum（字符串）、min/max（整数）、min_length/max_length。无 default 时必填。' },
    { key: "content", label: resource === "config-templates" ? "配置文件模板" : application ? "应用模板（YAML / JSON）" : "流水线模板（JSON / YAML）", type: "code", span: 2, required: true, help: resource === "config-templates" ? '填写文件文本，使用 {{ .Values.参数名 }}；secret参数填写变量名，部署时加密保存历史值。通过 manifest.files 的 fromTemplate 引用模板ID或唯一名称，values 传入参数。' : application ? '使用 "{{ .Values.image }}" 等占位符；secret: true 参数填写变量名，只能作为 valueFrom 的完整值。保存校验模板语法，生成时校验完整配置。' : '使用 "{{ .Values.command }}" 等创建参数；保留 {{run}}、{{branch}}、{{repo_url}}、{{commit}} 运行时变量。当前不支持 secret 参数。' },
  ] }],
});
const templateGenerator = (workspace: string, resource: string, field: string): NonNullable<FormConfig<AnyRow>["generator"]> => ({
  options: namedOptions(workspace, resource),
  load: async id => {
    const row = await api<Template>(`/workspaces/${workspace}/${resource}/${id}`);
    const parameters = JSON.parse(row.parameters) as TemplateParameter[];
    return { name: row.name, version: row.version, values: Object.fromEntries(parameters.filter(p => p.default !== undefined).map(p => [p.name, p.type === "boolean" ? String(p.default) : p.default])),
      fields: parameters.map(p => ({ key: p.name, label: `${p.label || p.name}${p.secret ? "（密钥变量名）" : ""}`, type: p.enum || p.type === "boolean" ? "select" as const : p.type === "integer" ? "number" as const : "text" as const,
        required: p.default === undefined,
        ...(p.enum ? { options: p.enum.map(value => ({value,label:value})) } : p.type === "boolean" ? { options: [{value:"true",label:"是"},{value:"false",label:"否"}] } : {}),
        help: p.secret ? "填写已存在的变量名，不要填写密钥明文。生成后仍需配置目标主机。" : `${p.name}${p.min !== undefined ? ` · 最小 ${p.min}` : ""}${p.max !== undefined ? ` · 最大 ${p.max}` : ""}${p.min_length !== undefined ? ` · 至少 ${p.min_length} 字符` : ""}${p.max_length !== undefined ? ` · 最多 ${p.max_length} 字符` : ""}`,
      })),
    };
  },
  generate: async (id, version, values) => {
    // The displayed template version is checked by the server; metadata fetch
    // here is only for typed controls, never an implicit version upgrade.
    const row = await api<Template>(`/workspaces/${workspace}/${resource}/${id}`);
    const parameters = JSON.parse(row.parameters) as TemplateParameter[];
    const typed = Object.fromEntries(Object.entries(values).filter(([key,value]) => !(parameters.find(p => p.name === key)?.type === "integer" && value === "")).map(([key,value]) => [key, parameters.find(p => p.name === key)?.type === "boolean" ? value === "true" : value]));
    const result = await api<Record<string,string>>(`/workspaces/${workspace}/${resource}/${id}/actions/render`, { method: "POST", body: JSON.stringify({ version, values: typed }) });
    return { [field]: result[field] };
  },
});

interface RepositoryAuthorization {id:string;provider:string;web_available:boolean;state:string;credential_id:string|null;account:string;expires_at:number;last_error:string;setup:string}
const authorizeRepository=(workspace:string)=>async(row:RepositoryAuthorization)=>{const result=await runItemAction<{url:string}>(workspace,"repository-auth",row.id,"authorize",{});window.location.assign(result.url);};
interface MarketSource {id:string;name:string;repository_id:string;provider:string;api_base:string;project:string;organization:string;branch:string;index_path:string;state:string;last_error:string;synced_at:number;version:number}
interface MarketEntry {id:string;source_id:string;entry_key:string;name:string;description:string;content:string;parameters:string;version:number}
const marketProviders=[{value:"github",label:"GitHub"},{value:"gitlab",label:"GitLab"},{value:"gitee",label:"Gitee"},{value:"codeup",label:"Codeup"}];
const marketInstall=(workspace:string)=>({label:"导入应用模板",permission:"marketplace.install",action:async(r:MarketEntry)=>{const created=await runItemAction<{id:string}>(workspace,"marketplace",r.id,"install",{version:r.version});navigate({group:"deploy",item:"app-templates",id:created.id,mode:"view"},{});}});
interface AlertRule {id:string;name:string;description:string;target_kind:string;target_ref:string;metric:string;threshold:number;enabled:boolean;has_webhook:boolean;webhook?:string;clear_webhook?:boolean;last_error:string;version:number}
interface AlertEvent {id:string;rule_id:string;rule_name:string;state:string;message:string;triggered_at:number;recovered_at:number;trigger_delivery:string;recovery_delivery:string;version:number}
const alertTargets=[{value:"host",label:"主机"},{value:"pipeline",label:"流水线"},{value:"deployment",label:"应用"},{value:"cron-job",label:"定时任务"}];
const alertMetrics=[{value:"offline",label:"离线"},{value:"load1",label:"一分钟负载"},{value:"memory-percent",label:"内存使用率（%）"},{value:"disk-percent",label:"磁盘使用率（%）"},{value:"execution-failed",label:"最近执行失败"}];
const alertEventLink={label:"告警事件",to:()=>({group:"deploy",item:"alert-events",mode:"list" as const})};
interface CronJob {id:string;name:string;description:string;scope:string;schedule:string;timezone:string;enabled:boolean;action:string;target_kind:string;target_ref:string;command:string;timeout_seconds:number;max_retries:number;retry_delay_seconds:number;next_run_at:number;run_as_name:string;version:number}
interface CronRun {id:string;job_id:string;job_name:string;scope:string;state:string;scheduled_at:number;started_at:number;finished_at:number;attempt:number;output:string;config:string;downstream_id:string;downstream_kind:string;version:number;trigger_kind:string}
const cronState=(state:string)=>({queued:"排队中",running:"执行中",succeeded:"成功",failed:"失败",skipped:"已跳过","needs-review":"需核实"}[state]??state);
const cronDate=(value:number)=>value?new Date(value).toLocaleString():"—";
const cronReview=(workspace:string)=>async(row:CronRun)=>{const note=prompt("确认远端及下游执行已结束后，填写核实说明；解除阻塞不会停止远端进程。");if(note===null)return;await runItemAction(workspace,"cron-runs",row.id,"acknowledge",{version:row.version,note});};
const cronLinks=(workspace:string)=>[
 {label:"返回任务",to:(r:CronRun)=>({group:r.scope,item:"cron-jobs",id:r.job_id,mode:"view" as const})},
 {label:"查看下游执行",when:(r:CronRun)=>r.downstream_id!=="0",to:(r:CronRun)=>({group:r.downstream_kind==="pipeline-runs"?"build":"deploy",item:r.downstream_kind==="pipeline-runs"?"runs":"revisions",id:r.downstream_id,mode:"view" as const})},
 {label:"核实并解除阻塞",permission:"cron-jobs.run",when:(r:CronRun)=>r.state==="needs-review",action:cronReview(workspace)},
];

export function listConfigs(workspace: string, permissions?: Set<string>, onChanged?: () => void, hostStates: Option[] = STATES): Record<string, ListConfig<AnyRow>> {
  return {
    "repository-auth": list<RepositoryAuthorization>({resource:"repository-auth",rowKey:r=>r.id,columns:[{key:"provider",label:"平台",render:r=>marketProviders.find(p=>p.value===r.provider)?.label??r.provider},{key:"account",label:"授权账号"},{key:"state",label:"授权状态",render:r=>({connected:"已连接",expired:"已过期",refreshable:"使用时续期",refreshing:"续期中",reauthorize:"需要重新授权","not-connected":"未连接"}[r.state]??r.state)},{key:"expires_at",label:"有效期至",render:r=>cronDate(r.expires_at)},{key:"setup",label:"授权方式"},{key:"last_error",label:"最近异常"}],actions:[{label:"网页授权",permission:"credentials.create",when:r=>r.web_available&&!r.credential_id,action:authorizeRepository(workspace)},{label:"重新授权",permission:"credentials.create",when:r=>r.web_available&&!!r.credential_id,action:authorizeRepository(workspace)},{label:"查看凭据",when:r=>!!r.credential_id,to:r=>({group:"build",item:"credentials",id:r.credential_id!,mode:"view"})},{label:"使用 API 令牌",permission:"credentials.create",to:()=>({group:"build",item:"api-tokens",mode:"new"})}],toolbar:{create:false,import:false,export:false}}),
    "marketplace-sources": list<MarketSource>({resource:"marketplace-sources",rowKey:r=>r.id,filters:[{key:"q",label:"名称",type:"text"}],columns:[{key:"name",label:"名称",sortable:true},{key:"provider",label:"平台"},{key:"state",label:"同步状态",render:r=>({idle:"空闲",syncing:"同步中",failed:"失败"}[r.state]??r.state)},{key:"synced_at",label:"上次成功",render:r=>cronDate(r.synced_at)},{key:"last_error",label:"同步错误"}],actions:[{label:"查看",to:r=>({id:r.id,mode:"view"})},{label:"编辑",permission:"marketplace-sources.update",to:r=>({id:r.id,mode:"edit"})},{label:"立即同步",permission:"marketplace-sources.sync",action:async r=>{await runItemAction(workspace,"marketplace-sources",r.id,"sync",{});}},{label:"目录",to:()=>({group:"deploy",item:"marketplace",mode:"list"}),query:r=>({source_id:r.id})},{label:"删除",permission:"marketplace-sources.delete",destructive:true,action:r=>deleteResource(workspace,"marketplace-sources",r.id)}],toolbar:{import:false,export:false}}),
    marketplace: list<MarketEntry>({resource:"marketplace",rowKey:r=>r.id,filters:[{key:"q",label:"名称 / 说明",type:"text"},{key:"source_id",label:"来源 ID",type:"text"}],columns:[{key:"name",label:"应用模板",sortable:true},{key:"description",label:"说明"},{key:"entry_key",label:"目录标识"}],actions:[{label:"查看",to:r=>({id:r.id,mode:"view"})},marketInstall(workspace)],toolbar:{create:false,import:false,export:false}}),
    "alert-rules": list<AlertRule>({resource:"alert-rules",permissionResource:"alerts",rowKey:r=>r.id,filters:[{key:"q",label:"名称",type:"text"},{key:"target_kind",label:"目标类型",type:"select",options:alertTargets}],columns:[{key:"name",label:"名称",sortable:true},{key:"metric",label:"指标",render:r=>alertMetrics.find(m=>m.value===r.metric)?.label??r.metric},{key:"threshold",label:"阈值"},{key:"enabled",label:"启用",render:r=>r.enabled?"是":"否"},{key:"has_webhook",label:"Webhook",render:r=>r.has_webhook?"已配置":"未配置"},{key:"last_error",label:"检查异常"}],actions:[{label:"查看",to:r=>({id:r.id,mode:"view"})},{label:"编辑",permission:"alerts.update",to:r=>({id:r.id,mode:"edit"})},{...alertEventLink,query:(r:AlertRule)=>({rule_id:r.id})},{label:"删除",permission:"alerts.delete",destructive:true,action:r=>deleteResource(workspace,"alert-rules",r.id)}],toolbar:{create:true,import:false,export:false}}),
    "alert-events": list<AlertEvent>({resource:"alert-events",rowKey:r=>r.id,filters:[{key:"rule_id",label:"规则 ID",type:"text"},{key:"state",label:"状态",type:"select",options:[{value:"firing",label:"未恢复"},{value:"recovered",label:"已恢复"}]}],columns:[{key:"rule_name",label:"规则"},{key:"state",label:"状态",render:r=>r.state==="firing"?"未恢复":"已恢复"},{key:"triggered_at",label:"触发时间",sortable:true,render:r=>cronDate(r.triggered_at)},{key:"recovered_at",label:"恢复时间",render:r=>cronDate(r.recovered_at)},{key:"trigger_delivery",label:"触发通知"},{key:"recovery_delivery",label:"恢复通知"}],actions:[{label:"查看",to:r=>({id:r.id,mode:"view"})}],toolbar:{create:false,import:false,export:false}}),
    "cron-jobs": list<CronJob>({resource:"cron-jobs",rowKey:r=>r.id,filters:[{key:"q",label:"名称 / 说明",type:"text"}],columns:[{key:"name",label:"名称",sortable:true},{key:"schedule",label:"Cron 表达式"},{key:"timezone",label:"时区"},{key:"enabled",label:"启用",render:r=>r.enabled?"是":"否"},{key:"next_run_at",label:"下次执行",sortable:true,render:r=>cronDate(r.next_run_at)},{key:"run_as_name",label:"执行身份"}],actions:[
      {label:"查看",to:r=>({id:r.id,mode:"view"})},{label:"编辑",permission:"cron-jobs.update",to:r=>({id:r.id,mode:"edit"})},
      {label:"立即执行",permission:"cron-jobs.run",action:async r=>{const result=await runItemAction<{id:string}>(workspace,"cron-jobs",r.id,"run-now",{});navigate({group:r.scope,item:"cron-runs",id:result.id,mode:"view"},{});}},
      {label:"执行历史",to:r=>({group:r.scope,item:"cron-runs",mode:"list"}),query:r=>({job_id:r.id})},
      {label:"删除",permission:"cron-jobs.delete",destructive:true,action:r=>deleteResource(workspace,"cron-jobs",r.id)}
    ],toolbar:{import:false,export:false}}),
    "cron-runs": list<CronRun>({resource:"cron-runs",rowKey:r=>r.id,filters:[{key:"job_id",label:"任务 ID",type:"text"},{key:"state",label:"状态",type:"select",options:["queued","running","succeeded","failed","skipped","needs-review"].map(value=>({value,label:cronState(value)}))}],columns:[{key:"job_name",label:"任务"},{key:"state",label:"状态",render:r=>cronState(r.state)},{key:"scheduled_at",label:"计划时间",sortable:true,render:r=>cronDate(r.scheduled_at)},{key:"attempt",label:"尝试次数"},{key:"trigger_kind",label:"触发方式",render:r=>r.trigger_kind==="manual"?"手动":"计划"}],actions:[{label:"查看",to:r=>({id:r.id,mode:"view"})},...cronLinks(workspace)],toolbar:{create:false,import:false,export:false}}),
    settings: list<Setting>({resource:"settings",rowKey:r=>r.id,filters:[{key:"q",label:"配置键",type:"text"}],columns:[{key:"name",label:"配置项"},{key:"key",label:"键",sortable:true},{key:"value",label:"当前值"},{key:"default",label:"默认值"}],actions:[{label:"查看",to:r=>({id:r.id,mode:"view"})},{label:"编辑",permission:"settings.update",to:r=>({id:r.id,mode:"edit"})}],toolbar:{create:false,import:false,export:false}}),
    dictionaries: list<Dictionary>({resource:"dictionaries",rowKey:r=>r.id,filters:[{key:"q",label:"标识 / 名称",type:"text"}],columns:[{key:"key",label:"标识",sortable:true},{key:"name",label:"名称",sortable:true},{key:"description",label:"说明"},{key:"builtin",label:"类型",render:r=>r.builtin?"系统字典":"自定义"}],actions:[{label:"查看",to:r=>({id:r.id,mode:"view"})},{label:"编辑",permission:"dictionaries.update",to:r=>({id:r.id,mode:"edit"})},{label:"复制",permission:"dictionaries.create",to:()=>({mode:"new"}),query:r=>({copy:r.id})},{label:"删除",permission:"dictionaries.delete",when:r=>!r.builtin,destructive:true,action:r=>deleteResource(workspace,"dictionaries",r.id)}],toolbar:{import:false,export:false}}),
    "config-templates": { ...templateList(workspace, "config-templates", "deployments", "deploy", "apps"), actions: templateList(workspace, "config-templates", "deployments", "deploy", "apps").actions?.filter(a => a.label !== "使用模板") ?? [] },
    "app-templates": templateList(workspace, "app-templates", "deployments", "deploy", "apps"),
    "pipeline-templates": templateList(workspace, "pipeline-templates", "pipelines", "build", "pipelines"),
    members: list({
      resource: "members", rowKey: (r: Member) => r.id,
      filters: [{ key: "q", label: "姓名或邮箱", type: "text" }],
      columns: [{ key: "name", label: "姓名" }, { key: "email", label: "邮箱" }, { key: "role_name", label: "角色" }, { key: "state", label: "状态", render: r => r.owner ? "拥有者" : !r.active ? "账号已停用" : r.state === "active" ? "正常" : r.state }],
      actions: [
        { label: "查看", to: r => ({ id: r.id, mode: "view" }) },
        { label: "调整角色", permission: "members.update", when: r => !r.owner, to: r => ({ id: r.id, mode: "edit" }) },
        { label: "移除", permission: "members.delete", when: r => !r.owner, destructive: true, action: async r => { await deleteResource(workspace, "members", r.id); onChanged?.(); } },
      ], toolbar: { create: false, import: false, export: false, refresh: [0, 30] },
    }),
    roles: list({
      resource: "roles", rowKey: (r: Role) => r.id,
      filters: [{ key: "q", label: "名称", type: "text" }],
      columns: [{ key: "name", label: "名称", sortable: true }, { key: "builtin", label: "类型", render: r => r.builtin ? "内置角色" : "自定义角色" }, { key: "permissions", label: "权限数", render: r => r.permissions.length }],
      actions: [
        { label: "查看", to: r => ({ id: r.id, mode: "view" }) },
        { label: "复制", permission: "roles.create", when: r => r.permissions.every(p => permissions?.has(p) ?? true), to: () => ({ mode: "new" }), query: r => ({ copy: r.id }) },
        { label: "编辑", permission: "roles.update", when: r => !r.builtin, to: r => ({ id: r.id, mode: "edit" }) },
        { label: "删除", permission: "roles.delete", when: r => !r.builtin, destructive: true, action: async r => { await deleteResource(workspace, "roles", r.id); onChanged?.(); } },
      ], toolbar: { create: true, import: false, export: false, refresh: [0, 30] },
    }),
    deployments: list({
      resource: "deployments", rowKey: (r: Deployment) => r.id,
      filters: [{ key: "q", label: "名称", type: "text" }],
      columns: [{ key: "name", label: "名称", sortable: true }, { key: "state", label: "状态", render: (r) => deploymentState(r.state) }, { key: "target_ref", label: "目标主机 ID" }, { key: "description", label: "备注" }],
      actions: [
        { label: "查看", permission: "deployments.read", to: (r) => ({ id: r.id, mode: "view" }) },
        { label: "编辑", permission: "deployments.update", to: (r) => ({ id: r.id, mode: "edit" }) },
        { label: "删除记录（保留容器和卷）", permission: "deployments.delete", destructive: true, action: (r) => deleteResource(workspace, "deployments", r.id) },
      ],
    }),
    "deployment-revisions": list({
      resource: "deployment-revisions", rowKey: (r: Revision) => r.id,
      filters: [{ key: "deployment_id", label: "应用 ID", type: "text" }],
      columns: [{ key: "number", label: "Revision", sortable: true }, { key: "deployment_id", label: "应用 ID" }, { key: "action", label: "操作", render: (r) => r.action === "rollback" ? "回滚" : "部署" }, { key: "state", label: "状态", render: (r) => deploymentState(r.state) }, { key: "started_at", label: "开始时间", render: (r) => r.started_at ? new Date(r.started_at).toLocaleString() : "—" }],
      actions: [{ label: "查看", permission: "deployments.read", to: (r) => ({ id: r.id, mode: "view" }) }],
      toolbar: { create: false, import: false, export: false },
    }),
    repositories: list({
      resource: "repositories", rowKey: (r: Repository) => r.id,
      filters: [{ key: "q", label: "名称 / URL", type: "text" }],
      columns: [
        { key: "name", label: "名称" }, { key: "url", label: "URL" },
        { key: "default_branch", label: "默认分支" }, { key: "description", label: "备注" },
      ],
      actions: [
        { label: "查看", permission: "repositories.read", to: (r) => ({ id: r.id, mode: "view" }) },
        { label: "编辑", permission: "repositories.update", to: (r) => ({ id: r.id, mode: "edit" }) },
        { label: "删除", permission: "repositories.delete", destructive: true,
          action: (r) => deleteResource(workspace, "repositories", r.id) },
      ],
    }),
    pipelines: list({
      resource: "pipelines", rowKey: (r: Pipeline) => r.id,
      filters: [{ key: "q", label: "名称", type: "text" }],
      columns: [
        { key: "name", label: "名称" }, { key: "repository_id", label: "仓库 ID" },
        { key: "target_kind", label: "目标类型", render: (r) => label(TARGETS, r.target_kind) },
        { key: "target_ref", label: "目标 ID" }, { key: "description", label: "备注" },
      ],
      actions: [
        { label: "查看", permission: "pipelines.read", to: (r) => ({ id: r.id, mode: "view" }) },
        { label: "触发", permission: "pipelines.run", action: async (r) => {
          const run = await runItemAction<{ id: string }>(workspace, "pipelines", r.id, "run", {});
          const group = nav.find((g) => g.items.some((i) => i.resource === "pipeline-runs"))!;
          const item = group.items.find((i) => i.resource === "pipeline-runs")!;
          navigate({ workspace, group: group.key, item: item.key, id: run.id, mode: "view" }, {});
        } },
        { label: "编辑", permission: "pipelines.update", to: (r) => ({ id: r.id, mode: "edit" }) },
        { label: "删除", permission: "pipelines.delete", destructive: true,
          action: (r) => deleteResource(workspace, "pipelines", r.id) },
      ],
    }),
    "pipeline-artifacts": list({
      resource: "pipeline-artifacts", permissionResource: "pipelines", rowKey: (r: { id: string }) => r.id,
      filters: [{ key: "q", label: "文件路径", type: "text" }, { key: "run_id", label: "执行 ID", type: "text" }, { key: "pipeline_id", label: "流水线 ID", type: "text" }],
      columns: [
        { key: "path", label: "文件路径" }, { key: "run_id", label: "执行 ID" },
        { key: "size", label: "大小（字节）", sortable: true }, { key: "sha256", label: "SHA-256" },
        { key: "download", label: "下载", render: (r: { id: string; path: string }) => <a className="text-primary underline" href={`/api/workspaces/${encodeURIComponent(workspace)}/pipeline-artifacts/${r.id}/download`} download>下载 {r.path}</a> },
      ],
      toolbar: { create: false, import: false, export: false },
      empty: "还没有构建产物。在流水线定义的 artifacts 中声明相对文件路径，构建成功后会自动归档。",
    }),
    "pipeline-runs": list({
      resource: "pipeline-runs", rowKey: (r: PipelineRun) => r.id,
      filters: [{ key: "state", label: "状态", type: "select", options:
        ["queued", "running", "succeeded", "failed", "cancelled"].map((value) => ({ value, label: stateBadge(value).label })) }],
      columns: [
        { key: "number", label: "执行序号", sortable: true, render: (r) => `#${r.number}` },
        { key: "pipeline_id", label: "流水线 ID" },
        { key: "state", label: "状态", render: (r) => {
          const { label, ...props } = stateBadge(r.state);
          return <Badge {...props}>{label}</Badge>;
        } },
        { key: "triggered_by", label: "触发人 ID" },
        { key: "duration", label: "耗时", render: (r) => duration(r.started_at, r.finished_at) },
        { key: "started_at", label: "开始时间", render: (r) => r.started_at ? new Date(r.started_at).toLocaleString("zh-CN") : "—" },
      ],
      actions: [{ label: "查看", permission: "pipelines.read", to: (r) => ({ id: r.id, mode: "view" }) }],
      toolbar: { create: false, import: false, export: false },
    }),
    "host-groups": list({
      resource: "host-groups",
      rowKey: (row: HostGroup) => row.id,
      filters: [{ key: "q", label: "名称/说明", type: "text", placeholder: "输入关键字" }],
      columns: [
        { key: "name", label: "名称" },
        { key: "description", label: "说明" },
      ],
      actions: [
        { label: "查看", permission: "host-groups.read", to: (row: HostGroup) => ({ id: row.id, mode: "view" }) },
        { label: "编辑", permission: "host-groups.update", to: (row: HostGroup) => ({ id: row.id, mode: "edit" }) },
        {
          label: "删除",
          permission: "host-groups.delete",
          destructive: true,
          action: (row: HostGroup) => deleteResource(workspace, "host-groups", row.id),
        },
      ],
      empty: "还没有主机组。先建一个，再把主机归进去。",
    }),

    hosts: list({
      resource: "hosts",
      rowKey: (row: Host) => row.id,
      filters: [
        { key: "tag_id", label: "标签", type: "select", options: tagOptions(workspace) },
        { key: "q", label: "名称/地址/账号", type: "text", placeholder: "输入关键字" },
        { key: "group_id", label: "主机组", type: "select", options: groupOptions(workspace) },
        { key: "connection", label: "接入方式", type: "select", options: CONNECTIONS },
        { key: "state", label: "状态", type: "select", options: hostStates },
      ],
      columns: [
        { key: "name", label: "名称" },
        { key: "tags", label: "标签", render: (row: Host) => <span className="flex flex-wrap gap-1">{row.tags?.map(tag => <Badge variant="secondary" key={tag.id}>{tag.name}</Badge>)}</span> },
        { key: "address", label: "地址", width: 180 },
        {
          key: "connection",
          label: "接入方式",
          width: 130,
          render: (row: Host) => label(CONNECTIONS, row.connection),
        },
        {
          key: "state",
          label: "状态",
          width: 90,
          render: (row: Host) => (
            <Badge
              variant={
                row.state === "online"
                  ? "default"
                  : row.state === "offline"
                    ? "destructive"
                    : "secondary"
              }
            >
              {label(hostStates, row.state)}
            </Badge>
          ),
        },
        { key: "seen_at", label: "最近心跳", width: 130, sortable: true, render: (row: Host) => ago(row.seen_at) },
      ],
      actions: [
        { label: "查看", permission: "hosts.read", to: (row: Host) => ({ id: row.id, mode: "view" }) },
        {
          label: "测试连接",
          permission: "hosts.read",
          action: async (row: Host) => {
            const result = await runItemAction<{ ok: boolean; stderr: string }>(
              workspace, "hosts", row.id, "test-connection", {},
            );
            if (!result.ok) throw new Error(result.stderr || "连接失败。");
          },
        },
        { label: "编辑", permission: "hosts.update", to: (row: Host) => ({ id: row.id, mode: "edit" }) },
        {
          label: "删除",
          permission: "hosts.delete",
          destructive: true,
          action: (row: Host) => deleteResource(workspace, "hosts", row.id),
        },
      ],
      bulk: [
        {
          label: "批量删除",
          permission: "hosts.delete",
          destructive: true,
          action: async (ids) => {
            await runAction(workspace, "hosts", "bulk-delete", { ids });
          },
        },
      ],
      empty: "还没有主机。用代理一条命令接入，或填 SSH 账号直连。",
    }),

    tags: list({
      resource: "tags",
      rowKey: (row: Tag) => row.id,
      filters: [{ key: "q", label: "名称", type: "text", placeholder: "输入关键字" }],
      columns: [
        { key: "name", label: "名称" },
        {
          key: "color",
          label: "颜色",
          width: 120,
          render: (row: Tag) =>
            row.color ? (
              <span className="inline-flex items-center gap-2">
                <span
                  className="inline-block size-3 rounded-full border border-border"
                  style={{ background: row.color }}
                />
                {row.color}
              </span>
            ) : (
              "—"
            ),
        },
      ],
      actions: [
        { label: "编辑", permission: "tags.update", to: (row: Tag) => ({ id: row.id, mode: "edit" }) },
        {
          label: "删除",
          permission: "tags.delete",
          destructive: true,
          action: (row: Tag) => deleteResource(workspace, "tags", row.id),
        },
      ],
      empty: "还没有标签。",
    }),

    variables: list({
      resource: "variables", rowKey: (row: Variable) => row.id,
      filters: [{ key: "q", label: "名称 / 备注", type: "text" }, { key: "kind", label: "类型", type: "select", options: VARIABLE_KINDS }, { key: "scope_ref", label: "对象 ID", type: "text", placeholder: "0 表示模块默认" }],
      columns: [
        { key: "name", label: "名称", sortable: true },
        { key: "kind", label: "类型", render: (row) => label(VARIABLE_KINDS, row.kind) },
        { key: "scope", label: "来源层", render: variableSource },
        { key: "value", label: "值", render: (row) => row.kind === "secret" ? (row.has_value ? "已设置" : "未设置") : <span className="whitespace-pre-wrap break-all">{row.value === "" ? "（空字符串）" : row.value}</span> },
        { key: "description", label: "备注" },
      ],
      actions: [
        { label: "查看", permission: "variables.read", to: (row) => ({ id: row.id, mode: "view" }) },
        { label: "编辑", permission: "variables.update", to: (row) => ({ id: row.id, mode: "edit" }) },
        { label: "删除", permission: "variables.delete", destructive: true, action: (row) => deleteResource(workspace, "variables", row.id) },
      ],
      toolbar: { import: false, export: false },
      empty: "当前作用域没有变量。全局、模块和对象同名时，具体对象优先。",
    }),

    credentials: list({
      resource: "credentials",
      rowKey: (row: Credential) => row.id,
      filters: [
        { key: "q", label: "名称/账号", type: "text", placeholder: "输入关键字" },
        { key: "kind", label: "类型", type: "select", options: CREDENTIAL_KINDS },
      ],
      columns: [
        { key: "name", label: "名称" },
        { key: "kind", label: "类型", width: 120, render: (r: Credential) => label(CREDENTIAL_KINDS, r.kind) },
        { key: "username", label: "账号", width: 160 },
        {
          key: "has_secret",
          label: "内容",
          width: 100,
          render: (r: Credential) =>
            r.has_secret ? (
              <Badge variant="secondary">已设置</Badge>
            ) : (
              <Badge variant="destructive">未设置</Badge>
            ),
        },
      ],
      actions: [
        { label: "编辑", permission: "credentials.update", to: (r: Credential) => ({ id: r.id, mode: "edit" }) },
        {
          label: "删除",
          permission: "credentials.delete",
          destructive: true,
          action: (r: Credential) => deleteResource(workspace, "credentials", r.id),
        },
      ],
      toolbar: { import: false, export: false },
      empty: "还没有凭据。SSH 主机和 Git 仓库都从这里取登录信息。",
    }),

    activity: list({
      resource: "activity",
      rowKey: (row: Activity) => row.id,
      filters: [
        { key: "q", label: "操作人/资源", type: "text", placeholder: "输入关键字" },
        {
          key: "action",
          label: "动作",
          type: "select",
          options: [
            { value: "create", label: "新增" },
            { value: "update", label: "修改" },
            { value: "delete", label: "删除" },
            { value: "bulk-delete", label: "批量删除" },
          ],
        },
      ],
      columns: [
        { key: "created_at", label: "时间", width: 180, sortable: true, render: (row: Activity) => new Date(row.created_at).toLocaleString("zh-CN") },
        { key: "actor_name", label: "操作人", width: 120 },
        { key: "action", label: "动作", width: 100 },
        { key: "resource", label: "资源", width: 140 },
        {
          key: "details",
          label: "详情",
          render: (row: Activity) => (
            <code className="text-xs text-muted-foreground">{JSON.stringify(row.details)}</code>
          ),
        },
      ],
      toolbar: { create: false, import: false },
      empty: "还没有操作记录。",
    }),
  };
}

export function formConfigs(workspace: string, permissions?: Set<string>): Record<string, FormConfig<AnyRow>> {
  return {
    "marketplace-sources": form<MarketSource>({resource:"marketplace-sources",heading:r=>r.name,blank:{name:"",repository_id:"",provider:"github",api_base:"https://api.github.com",project:"",organization:"",branch:"main",index_path:"marketplace.json"},sections:[{fields:[{key:"name",label:"名称",type:"text",required:true},{key:"repository_id",label:"外部仓库",type:"select",required:true,options:namedOptions(workspace,"repositories"),help:"使用所选仓库的加密令牌；公共仓库可以不配置凭据。"},{key:"provider",label:"平台",type:"select",required:true,options:marketProviders},{key:"api_base",label:"API 接入点",type:"text",required:true,help:"GitHub: https://api.github.com；GitLab: https://gitlab.com/api/v4；Gitee: https://gitee.com/api/v5；Codeup中心版: https://openapi-rdc.aliyuncs.com。自托管使用所选仓库同主机的API。"},{key:"project",label:"项目路径 / ID",type:"text",required:true,help:"GitHub/Gitee填写owner/repository；GitLab/Codeup支持项目完整路径或ID。"},{key:"organization",label:"组织 ID",type:"text",when:d=>d.provider==="codeup",help:"Codeup中心版需要组织ID；Region版可留空。"},{key:"branch",label:"分支 / 标签 / Commit",type:"text",required:true},{key:"index_path",label:"索引文件路径",type:"text",required:true,help:"仓库内JSON文件，格式为version:1及templates数组；所有模板验证成功后才替换目录。"}]}]}),
    marketplace: form<MarketEntry>({resource:"marketplace",heading:r=>r.name,blank:{},readOnly:()=>true,viewActions:[marketInstall(workspace)],sections:[{fields:[{key:"name",label:"名称",type:"text"},{key:"description",label:"说明",type:"textarea",span:2},{key:"content",label:"应用配置",type:"code",span:2},{key:"parameters",label:"模板参数",type:"code",span:2}]}]}),
    "alert-rules": form<AlertRule>({resource:"alert-rules",permissionResource:"alerts",heading:r=>r.name,blank:{name:"",description:"",target_kind:"host",target_ref:"",metric:"offline",threshold:0,enabled:false,webhook:"",clear_webhook:false},viewActions:[alertEventLink],sections:[{fields:[
      {key:"name",label:"名称",type:"text",required:true},{key:"enabled",label:"启用",type:"switch"},
      {key:"target_kind",label:"目标类型",type:"select",required:true,immutable:true,options:alertTargets,resetOnChange:["target_ref","metric"]},
      ...([['host','hosts','主机'],['pipeline','pipelines','流水线'],['deployment','deployments','应用'],['cron-job','cron-jobs','定时任务']] as const).map(([kind,resource,label])=>({key:"target_ref",label:`目标${label}`,type:"select" as const,required:true,immutable:true,when:(d:Partial<AlertRule>)=>d.target_kind===kind,options:namedOptions(workspace,resource)})),
      {key:"metric",label:"指标",type:"select",required:true,immutable:true,when:d=>d.target_kind==="host",options:alertMetrics.filter(m=>m.value!=="execution-failed")},
      {key:"metric",label:"指标",type:"select",required:true,immutable:true,when:d=>d.target_kind!=="host",options:alertMetrics.filter(m=>m.value==="execution-failed")},
      {key:"threshold",label:"阈值",type:"number",required:true,when:d=>!!d.metric&&!['offline','execution-failed'].includes(d.metric),help:"指标严格大于阈值时触发；内存和磁盘范围0–100，负载为非负整数。"},
      {key:"webhook",label:"Webhook 地址",type:"password",stored:"has_webhook",help:"可选HTTP(S)地址；加密保存、不回显。编辑留空保留原地址。触发和恢复各尝试一次，失败不自动重试。"},
      {key:"clear_webhook",label:"清除已有 Webhook",type:"switch",when:d=>!!d.has_webhook},
      {key:"description",label:"说明",type:"textarea",span:2},
      {key:"last_error",label:"检查异常",type:"text",immutable:true,when:d=>!!d.last_error,help:"检查执行身份的账号权限和目标是否仍然有效。"}
    ]}]}),
    "alert-events": form<AlertEvent>({resource:"alert-events",heading:r=>r.rule_name,blank:{},readOnly:()=>true,pollMs:r=>r.state==="firing"||r.trigger_delivery==="投递中"||r.recovery_delivery==="投递中"?2000:0,viewActions:[{label:"返回规则",to:()=>({group:"deploy",item:"alerts",mode:"list"})}],sections:[{fields:[{key:"rule_name",label:"规则",type:"text"},{key:"state",label:"状态",type:"select",options:[{value:"firing",label:"未恢复"},{value:"recovered",label:"已恢复"}]},{key:"message",label:"触发信息",type:"textarea",span:2},{key:"trigger_delivery",label:"触发通知",type:"text"},{key:"recovery_delivery",label:"恢复通知",type:"text"}]}]}),
    "cron-jobs": form<CronJob>({resource:"cron-jobs",heading:r=>r.name,blank:{name:"",description:"",scope:"host",schedule:"0 * * * *",timezone:"",enabled:false,action:"",target_kind:"",target_ref:"",command:"",timeout_seconds:1800,max_retries:0,retry_delay_seconds:30},viewActions:[{label:"执行历史",to:r=>({group:r.scope,item:"cron-runs",mode:"list"}),query:r=>({job_id:r.id})}],sections:[{fields:[
      {key:"name",label:"名称",type:"text",required:true},{key:"scope",label:"所属模块",type:"select",resetOnChange:["action","target_kind","target_ref","command"],options:[{value:"host",label:"主机"},{value:"build",label:"构建"},{value:"deploy",label:"部署"}],immutable:true},
      {key:"enabled",label:"启用定时执行",type:"switch"},{key:"schedule",label:"Cron 表达式",type:"text",required:true,help:"5字段：分 时 日 月 周；6字段在最前面增加秒。停机错过的触发不补跑，同任务重叠时跳过并记录。"},
      {key:"timezone",label:"时区",type:"text",required:true,placeholder:"Asia/Shanghai",help:"必须明确填写 IANA 时区，例如 Asia/Shanghai、Asia/Singapore、UTC。"},
      {key:"action",label:"动作",type:"select",required:true,when:d=>d.scope==="host",options:[{value:"command",label:"执行命令"}]},
      {key:"action",label:"动作",type:"select",required:true,when:d=>d.scope==="build",options:[{value:"pipeline",label:"触发流水线"}]},
      {key:"action",label:"动作",type:"select",required:true,when:d=>d.scope==="deploy",options:[{value:"redeploy",label:"重新部署当前配置"},{value:"restart",label:"重启当前成功版本"},{value:"sync-marketplace",label:"同步市场目录"}]},
      {key:"target_kind",label:"目标类型",type:"select",required:true,resetOnChange:["target_ref"],when:d=>d.scope==="host",options:TARGETS},
      {key:"target_kind",label:"目标类型",type:"select",required:true,when:d=>d.scope==="build",options:[{value:"pipeline",label:"流水线"}]},
      {key:"target_kind",label:"目标类型",type:"select",required:true,when:d=>d.scope==="deploy"&&d.action!=="sync-marketplace",options:[{value:"deployment",label:"应用"}]},
      {key:"target_kind",label:"目标类型",type:"select",required:true,when:d=>d.action==="sync-marketplace",options:[{value:"marketplace-source",label:"市场来源"}]},
      {key:"target_ref",label:"市场来源",type:"select",required:true,when:d=>d.target_kind==="marketplace-source",options:namedOptions(workspace,"marketplace-sources")},
      ...([['host','hosts','主机'],['host-group','host-groups','主机组'],['pipeline','pipelines','流水线'],['deployment','deployments','应用']] as const).map(([kind,resource,label])=>({key:"target_ref",label:`目标${label}`,type:"select" as const,required:true,when:(d:Partial<CronJob>)=>d.target_kind===kind,options:namedOptions(workspace,resource)})),
      {key:"command",label:"命令",type:"code",span:2,required:true,when:d=>d.scope==="host",help:"通过所选主机通道执行；主机组逐台执行，失败即停。不要把密钥写进脚本。"},
      {key:"timeout_seconds",label:"超时（秒）",type:"number",required:true,help:"1–86400；下游执行等待超时不会强制停止远端，会标记需核实。"},
      {key:"max_retries",label:"失败重试次数",type:"number",required:true,help:"默认0；仅对确定失败重试，最多10次。命令必须允许重复执行。"},
      {key:"retry_delay_seconds",label:"重试间隔（秒）",type:"number",required:true},
      {key:"run_as_name",label:"执行身份",type:"text",immutable:true,when:d=>!!d.run_as_name,help:"保存后使用保存者身份；每次执行重新核实账号和权限。"},
      {key:"description",label:"说明",type:"textarea",span:2}
    ]}]}),
    "cron-runs": form<CronRun>({resource:"cron-runs",heading:r=>r.job_name,blank:{},readOnly:()=>true,pollMs:r=>r.state==="queued"||r.state==="running"?1000:0,viewActions:cronLinks(workspace),sections:[{fields:[{key:"job_name",label:"任务",type:"text"},{key:"state",label:"状态",type:"select",options:["queued","running","succeeded","failed","skipped","needs-review"].map(value=>({value,label:cronState(value)}))},{key:"attempt",label:"尝试次数",type:"number"},{key:"downstream_id",label:"下游执行 ID",type:"text"},{key:"output",label:"执行输出",type:"code",span:2},{key:"config",label:"本次配置快照",type:"code",span:2}]}]}),
    settings: form<Setting>({resource:"settings",heading:r=>r.name,blank:{},sections:[{fields:[{key:"name",label:"配置项",type:"text",immutable:true},{key:"key",label:"键",type:"text",immutable:true},{key:"type",label:"类型",type:"text",immutable:true},{key:"value",label:"值",type:"number",required:true,help:"分页范围 1–200；心跳超时 40–600 秒；历史保留 0 为永久，1–36500 为天数。历史清理保留应用当前版本，不删除运行中的记录。"}]}]}),
    dictionaries: form<Dictionary>({resource:"dictionaries",heading:r=>r.name,blank:{key:"",name:"",description:"",items:JSON.stringify([{value:"value",label:"显示名称"}],null,2)},copy:r=>({key:r.key+"-copy",name:r.name+"（副本）",description:r.description,items:r.items}),sections:[{fields:[{key:"key",label:"标识",type:"text",required:true,immutable:true},{key:"name",label:"名称",type:"text",required:true},{key:"description",label:"说明",type:"textarea",span:2},{key:"items",label:"字典项（JSON 数组）",type:"code",required:true,span:2,help:"每项填写唯一 value 和显示 label；数组顺序即选项顺序。主机状态的值由系统控制，只能修改标签与排序。"}]}]}),
    "config-templates": templateForm("config-templates", false),
    "app-templates": templateForm("app-templates", true),
    "pipeline-templates": templateForm("pipeline-templates", false),
    members: form({
      resource: "members", heading: (r: Member) => r.name, blank: {}, readOnly: r => !!r.owner,
      sections: [{ fields: [
        { key: "name", label: "姓名", type: "text", immutable: true },
        { key: "email", label: "邮箱", type: "text", immutable: true },
        { key: "role_name", label: "当前角色", type: "text", immutable: true },
        { key: "role_id", label: "角色", type: "select", required: true, when: r => !r.owner, options: async () => {
          if (permissions && !permissions.has("members.update")) return [];
          const result = await api<{items: Role[]}>(`/workspaces/${workspace}/members/role-options`);
          return result.items.map(r => ({ value: r.id, label: r.name }));
        } },
      ] }],
    }),
    roles: form({
      resource: "roles", heading: (r: Role) => r.name, blank: { name: "", permissions: [] }, readOnly: r => !!r.builtin,
      copy: r => ({ name: `${r.name}（副本）`, permissions: [...r.permissions] }),
      sections: [{ fields: [
        { key: "name", label: "名称", type: "text", required: true },
        { key: "permissions", label: "权限", type: "multiselect", span: 2, help: "勾选操作会自动包含依赖权限；取消依赖权限会同时取消依赖它的操作。", options: async () => {
          const result = await api<{items: {key: string; group: string; action: string; requires: string[]}[]}>("/permissions");
          return result.items.map(p => ({ value: p.key, label: p.action, group: p.group, requires: p.requires, disabled: permissions ? !permissions.has(p.key) : false }));
        } },
      ] }],
    }),
    deployments: form({
      ...(permissions?.has("app-templates.read") ? { generator: templateGenerator(workspace, "app-templates", "manifest") } : {}),
      resource: "deployments", heading: (r: Deployment) => r.name,
      blank: { name: "", mode: "compose", target_kind: "host", target_ref: "", workdir: "/opt/deployments", description: "", manifest: "apiVersion: ferry/v1\nkind: Application\nmetadata:\n  name: web\nservices:\n  - name: web\n    image: nginx:alpine\n    ports:\n      - container: 80\n        published: 8080\n" },
      sections: [{ fields: [
        { key: "name", label: "名称", type: "text", required: true },
        { key: "mode", label: "部署模式", type: "select", options: [{ value: "compose", label: "Docker Compose" }, { value: "docker", label: "Docker" }, { value: "native", label: "Native（systemd）" }, { value: "stack", label: "Docker Swarm" }], immutable: true },
        { key: "target_kind", label: "目标类型", type: "select", options: [{ value: "host", label: "单主机" }, { value: "host-group", label: "主机组（逐台执行）" }], immutable: true },
        { key: "target_ref", label: "目标主机", type: "select", options: namedOptions(workspace, "hosts"), required: true, immutable: true, when: (d) => d.target_kind === "host" },
        { key: "target_ref", label: "目标主机组", type: "select", options: namedOptions(workspace, "host-groups"), required: true, immutable: true, when: (d) => d.target_kind === "host-group", help: "部署全部直接成员，按主机 ID 排序；失败停止，已成功主机不自动回滚。" },
        { key: "workdir", label: "部署工作目录", type: "text", required: true, immutable: true, help: "目标 Linux 主机上的可写绝对路径；自动按工作空间和应用隔离子目录。容器模式需要 Docker，Compose 另需 Compose v2+；Native 需要 systemd 及管理服务的权限。所有模式需要 flock、sha256sum。" },
        { key: "manifest", label: "应用 manifest（YAML / JSON）", type: "code", required: true, span: 2, help: "支持镜像、多服务、端口、环境变量、命名卷、依赖、HTTP健康检查和CPU/内存资源限制；环境项可使用 valueFrom: variable/变量名 引用普通或密钥变量，按全局→部署模块→应用专属覆盖。files 可引用配置模板；Native 使用 artifact: {source: pipeline, pipeline: 流水线ID, path: 产物相对路径}，可选 run 固定构建执行；Swarm 支持 strategy.type: rolling 和 maxUnavailable。不要填写密钥明文。" },
        { key: "description", label: "备注", type: "textarea", span: 2 },
      ] }],
    }),
    repositories: form({
      resource: "repositories", heading: (r: Repository) => r.name,
      blank: { name: "", url: "", default_branch: "main", credential_id: "0", description: "" },
      sections: [{ fields: [
        { key: "name", label: "名称", type: "text", required: true },
        { key: "url", label: "URL", type: "text", required: true },
        { key: "default_branch", label: "默认分支", type: "text", required: true },
        { key: "credential_id", label: "凭据", type: "select", options: async () => [
          { value: "0", label: "无凭据" }, ...await namedOptions(workspace, "credentials")(),
        ] },
        { key: "description", label: "备注", type: "textarea", span: 2 },
      ] }],
    }),
    pipelines: form({
      ...(permissions?.has("pipeline-templates.read") ? { generator: templateGenerator(workspace, "pipeline-templates", "definition") } : {}),
      resource: "pipelines", heading: (r: Pipeline) => r.name,
      blank: { name: "", repository_id: "", target_kind: "host", target_ref: "", description: "",
        definition: JSON.stringify({ workdir: "/tmp/build-{{run}}", steps: [{ name: "构建", run: "echo hello", timeout_seconds: 1800 }] }, null, 2) },
      sections: [{ fields: [
        { key: "name", label: "名称", type: "text", required: true },
        { key: "repository_id", label: "仓库", type: "select", options: namedOptions(workspace, "repositories") },
        { key: "target_kind", label: "目标类型", type: "select", options: TARGETS, resetOnChange: ["target_ref"] },
        { key: "target_ref", label: "目标主机", type: "select", required: true,
          options: namedOptions(workspace, "hosts"), when: (d) => d.target_kind === "host" },
        { key: "target_ref", label: "目标主机组", type: "select", required: true,
          options: namedOptions(workspace, "host-groups"), when: (d) => d.target_kind === "host-group" },
        { key: "definition", label: "定义（JSON）", type: "code", required: true, span: 2,
          help: "逐步执行；服务端会检查定义并返回具体错误。" },
        { key: "description", label: "备注", type: "textarea", span: 2 },
      ] }],
    }),
    "host-groups": form({
      resource: "host-groups",
      heading: (row: HostGroup) => row.name,
      blank: { name: "", description: "", parent_id: "" },
      sections: [
        {
          fields: [
            { key: "name", label: "名称", type: "text", required: true, placeholder: "例如 生产-华东" },
            { key: "parent_id", label: "上级主机组", type: "select", options: groupOptions(workspace), help: "留空表示顶级" },
            { key: "description", label: "说明", type: "textarea", span: 2 },
          ],
        },
      ],
    }),

    hosts: form({
      resource: "hosts",
      heading: (row: Host) => row.name,
      blank: { name: "", connection: "runner", address: "", port: 22, username: "", group_id: "", tag_ids: [], credential_id: "", description: "" },
      sections: [
        {
          title: "基本信息",
          fields: [
            { key: "name", label: "名称", type: "text", required: true, placeholder: "例如 web-01" },
            { key: "group_id", label: "主机组", type: "select", options: groupOptions(workspace) },
            { key: "tag_ids", label: "标签", type: "multiselect", options: tagOptions(workspace), span: 2, help: "可选择多个标签；取消全部选择会清空关联。" },
            {
              key: "connection",
              label: "接入方式",
              type: "select",
              required: true,
              options: CONNECTIONS,
              immutable: true,
              help: "创建后不可更改：改了会让已注册的代理密钥或 SSH 凭据失效。",
            },
          ],
        },
        {
          title: "SSH 连接",
          fields: [
            {
              key: "address",
              label: "主机地址",
              type: "text",
              required: true,
              placeholder: "10.0.0.11 或 host.example.com",
              when: (draft: Partial<Host>) => draft.connection === "ssh",
            },
            { key: "port", label: "端口", type: "number", required: true, when: (draft: Partial<Host>) => draft.connection === "ssh" },
            { key: "username", label: "登录账号", type: "text", required: true, placeholder: "root", when: (draft: Partial<Host>) => draft.connection === "ssh" },
            {
              key: "credential_id",
              label: "登录凭据",
              type: "select",
              required: true,
              options: sshCredentialOptions(workspace),
              span: 2,
              help: "在「构建 → 授权认证」里维护。密钥与密码只存在服务端，连接时才解密。",
              when: (draft: Partial<Host>) => draft.connection === "ssh",
            },
          ],
        },
        {
          fields: [{ key: "description", label: "备注", type: "textarea", span: 2 }],
        },
      ],
    }),

    variables: form({
      resource: "variables", heading: (row: Variable) => row.name,
      blank: { name: "", scope: "global", scope_ref: "0", kind: "secret", value: "", description: "" },
      sections: [{ fields: [
        { key: "name", label: "名称", type: "text", required: true, immutable: true, help: "96个以内字母、数字、下划线或连字符；引用写作 variable/名称。" },
        { key: "kind", label: "类型", type: "select", options: VARIABLE_KINDS, immutable: true },
        { key: "scope", label: "作用域", type: "select", options: VARIABLE_SCOPES, immutable: true, resetOnChange: ["scope_ref"] },
        { key: "scope_ref", label: "对象 ID", type: "text", immutable: true, when: (row) => row.scope !== "global", help: "0 或留空为模块默认；部署范围填应用 ID，构建范围填流水线 ID。具体对象覆盖模块，模块覆盖全局。" },
        { key: "value", label: "变量值", type: "textarea", span: 2, when: (row) => row.kind === "plain", help: "空白和换行会原样保存；留空表示空字符串。" },
        { key: "value", label: "密钥值", type: "secret", stored: "has_value", replaceOnEdit: true, span: 2, when: (row) => row.kind === "secret",
          help: "加密保存。编辑时勾选替换后留空，表示设置为空字符串；查看明文会记录审计。",
          reveal: { permission: "variables.reveal", action: async (row) => (await runItemAction<{value: string}>(workspace, "variables", String(row.id), "reveal", {})).value } },
        { key: "description", label: "备注", type: "textarea", span: 2 },
      ] }],
    }),

    credentials: form({
      resource: "credentials",
      heading: (r: Credential) => r.name,
      blank: { name: "", kind: "password", username: "", description: "", secret: "" },
      sections: [
        {
          fields: [
            { key: "name", label: "名称", type: "text", required: true, placeholder: "例如 生产机 root" },
            {
              key: "kind",
              label: "类型",
              type: "select",
              required: true,
              options: CREDENTIAL_KINDS,
              immutable: true,
              help: "创建后不可更改：改类型等于换一条凭据。",
            },
            { key: "username", label: "账号", type: "text", placeholder: "root" },
            {
              key: "secret",
              label: "内容",
              type: "secret",
              span: 2,
              help: "密码、私钥全文或令牌。保存后服务端加密存放，任何接口都不会再返回它；编辑时留空表示不修改。",
            },
            { key: "description", label: "备注", type: "textarea", span: 2 },
          ],
        },
      ],
    }),

    tags: form({
      resource: "tags",
      heading: (row: Tag) => row.name,
      blank: { name: "", color: "" },
      sections: [
        {
          fields: [
            { key: "name", label: "名称", type: "text", required: true },
            { key: "color", label: "颜色", type: "text", placeholder: "#4f9cf9" },
          ],
        },
      ],
    }),
  };
}

interface Role { id: string; name: string; builtin: boolean; permissions: string[]; version: number }
interface Member { id: string; name: string; email: string; role_id: string; role_name: string; owner: boolean; active: boolean; state: string; version: number }

interface Setting { id:string; key:string; name:string; type:string; value:number; default:number; version:number }
interface Dictionary {id:string; key:string; name:string; description:string; builtin:boolean; items:string; version:number}
