import { commonText, configText } from "./i18n";
import { currentLocale } from "./locale";
import { importResources, exportResources } from "./components/ResourceTransfer";
import { DEFAULT_ACTIONS, dispatchInputs, parseActions } from "./actions-workflow";
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
import { navigate } from "./router";
import { Badge } from "@/components/ui/badge";
import { FolderOpen, SquareTerminal } from "lucide-react";
import { createResource, deleteResource, listResource, named, runAction, runItemAction } from "./resource";
import type { FormConfig, ListConfig, Option, RowAction } from "./components/dsl";
import { repositoryEventForm, repositoryEventList } from "./components/RepositoryMonitor";
import { codeInsightForm, codeInsightList } from "./components/CodeInsight";
import { typedResourceForm, typedResourceList } from "./components/typed-resources";

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
  runner_auto_update: "inherit" | "on" | "off";
  state: string;
  group_id: string;
  tag_ids: string[];
  tags: { id: string; name: string; color: string }[];
  credential_id: string;
  os?: string;
  jump_host_id: string;
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
  scope: string;
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
const VARIABLE_SCOPES = [{ value: "global", label: "全局", labelKey: "resource-domain:variable.global" }, { value: "deploy", label: "部署", labelKey: "resource-domain:variable.deploy" }, { value: "build", label: "构建", labelKey: "common-domain:nav.build" }, { value: "environment", label: "执行环境", labelKey: "resource-domain:pipeline.environment" }];
const VARIABLE_KINDS = [{ value: "secret", label: "密钥", labelKey: "resource-domain:variable.secret" }, { value: "plain", label: "普通", labelKey: "resource-domain:variable.plain" }];
const variableSource = (row: Variable) => row.scope === "global" ? commonText("resource-domain:variable.global") : `${label(VARIABLE_SCOPES, row.scope)} · ${row.scope_ref === "0" ? commonText("resource-domain:variable.moduleDefault") : commonText("resource-domain:variable.object", { id: row.scope_ref })}`;
export interface Tag {
  id: string;
  name: string;
  color: string;
  version: number;
}

const CREDENTIAL_KINDS: Option[] = [
  { value: "password", label: "密码", labelKey: "common-domain:auth.password" },
  { value: "ssh-key", label: "SSH 私钥", labelKey: "resource-domain:credential.sshKey" },
  { value: "token", label: "访问令牌", labelKey: "resource-domain:credential.token" },
];
const CONNECTIONS: Option[] = [
  { value: "runner", label: "代理（runner）", labelKey: "resource-domain:host.runner" },
  { value: "ssh", label: "SSH 直连", labelKey: "resource-domain:host.directSsh" },
];
const STATES: Option[] = [
  { value: "online", label: "在线", labelKey: "common-domain:dashboard.online" },
  { value: "offline", label: "离线", labelKey: "common-domain:dashboard.offline" },
  { value: "unknown", label: "未知", labelKey: "common-domain:dashboard.unknown" },
];

const label = (options: Option[], value: string) => {
  const option = options.find(option => option.value === value);
  return option ? configText(option.label, option.labelKey) : value;
};

/** Relative time; absolute dates make an operator do arithmetic. */
function ago(at: number) {
  if (!at) return "—";
  const seconds = Math.max(0, Math.round((Date.now() - at) / 1000));
  if (seconds < 60) return commonText("resource-domain:relative.seconds", { count: seconds });
  if (seconds < 3600) return commonText("resource-domain:relative.minutes", { count: Math.floor(seconds / 60) });
  if (seconds < 86400) return commonText("resource-domain:relative.hours", { count: Math.floor(seconds / 3600) });
  return commonText("resource-domain:relative.days", { count: Math.floor(seconds / 86400) });
}

/** Group options for the filter and the form, loaded once per render pass. */
/** SSH login needs a password or a key; a token is for Git, not for sshd. */
const sshCredentialOptions = (workspace: string) => async (): Promise<Option[]> => {
  const credentials: Credential[] = [];
  for (let page = 1; ; page++) {
    const result = await listResource<Credential>(workspace, "credentials", { page, page_size: 200, scope: "host" });
    credentials.push(...result.items);
    if (page * result.page_size >= result.total) break;
  }
  return credentials
    .filter((c) => c.kind === "password" || c.kind === "ssh-key")
    .map((c) => ({ value: c.id, label: `${c.name}（${label(CREDENTIAL_KINDS, c.kind)}）` }));
};
const sshJumpOptions = (workspace: string) => async (): Promise<Option[]> => {
  const choices: Option[] = [];
  for (let page = 1; ; page++) {
    const result = await listResource<Host>(workspace, "hosts", { page, page_size: 200, connection: "ssh" });
    choices.push(...result.items.filter(host => !host.jump_host_id || host.jump_host_id === "0").map(host => ({ value: host.id, label: host.name })));
    if (page * result.page_size >= result.total) break;
  }
  return choices;
};

const groupOptions = (workspace: string) => namedOptions(workspace, "host-groups");
const tagOptions = (workspace: string) => namedOptions(workspace, "tags");

interface CodeConnection { id:string; name:string; provider:string; credential_id:string; account:string; state:string; authorization:string; description:string; base_url:string; api_url:string; organization:string; url?:string; version:number }
interface Pipeline {
  id: string; name: string; repository_id: string; target_kind: string;
  group_id: string; group_name: string; application_id: string; application_name: string; tag_ids: string[]; environment_ids: string[]; tags: {id:string;name:string}[]; environments: {id:string;name:string}[];
  target_ref: string; definition: string; description: string; definition_format: string; executor_bindings: unknown[]; checkout_bindings: unknown[]; graph_layout: Record<string, unknown>;
}
const namedOptions = (workspace: string, resource: string) => async (): Promise<Option[]> => {
  const items: Option[] = [];
  for (let page = 1; ; page += 1) {
    const result = await listResource<{ id: string; name: string }>(workspace, resource, { page, page_size: 200 });
    items.push(...result.items.map((row) => ({ value: row.id, label: row.name })));
    if (!result.items.length || page * result.page_size >= result.total) return items;
  }
};
interface PipelineContext {id:string;name:string;description:string;target_kind:string;target_ref:string;requires_approval?:boolean;version:number}
// workflow_dispatch inputs become form fields named input:<name>.
const inputsOf = (row: Pipeline) => row.definition_format === "github-actions" ? dispatchInputs(row.definition) : [];
const pipelineRunAction = (workspace:string): RowAction<Pipeline> => ({
 label:"触发", labelKey: "resource-domain:pipeline.trigger",permission:"pipelines.run",
 parameters: row=>[
  ...(row.environments?.length ? [{key:"environment_id",label:"执行环境", labelKey: "resource-domain:pipeline.environment",type:"select" as const,required:true,options:row.environments.map(environment=>({value:environment.id,label:environment.name})),help:"环境默认执行器替代流水线默认值；任务单独指定的执行器优先。环境变量覆盖流水线默认变量。", helpKey: "resource-domain:pipeline.environmentHelp"}] : []),
  ...inputsOf(row).map(input=>({key:`input:${input.name}`,label:input.name,required:input.required&&input.type!=="boolean",...(input.description?{help:input.description}:{}),
   ...(input.type==="boolean"?{type:"switch" as const}:input.type==="choice"?{type:"select" as const,options:input.options.map(option=>({value:option,label:option}))}:input.type==="number"?{type:"number" as const}:{type:"text" as const})})),
 ],
 initialValues: row=>Object.fromEntries(inputsOf(row).filter(input=>input.default!==undefined).map(input=>[`input:${input.name}`,input.type==="boolean"?input.default===true||input.default==="true":input.default])),
 action:async(row,values)=>{
  const body: Record<string, unknown> = {}, inputs: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(values ?? {})) {
   if (key.startsWith("input:")) { if (value !== "" && value !== undefined) inputs[key.slice(6)] = value; }
   else body[key] = value;
  }
  if (Object.keys(inputs).length) body.inputs = inputs;
  const run=await runItemAction<{id:string}>(workspace,"pipelines",row.id,"run",body);
  navigate({workspace,group:"build",item:"runs",id:run.id,mode:"view"},{});
 }
});
const TARGETS = [{ value: "host", label: "主机", labelKey: "common-domain:nav.hosts" }, { value: "host-group", label: "主机组", labelKey: "common-domain:nav.hostGroups" }];

interface Template { id: string; name: string; description: string; content: string; parameters: string; version: number }
interface TemplateParameter { name: string; label?: string; type: "string" | "integer" | "boolean"; default?: unknown; enum?: string[]; min?: number; max?: number; min_length?: number; max_length?: number; secret?: boolean }
const templateList = (workspace: string, resource: string, target: string, group: string, item: string) => list<Template>({
  resource, rowKey: r => r.id, filters: [{ key: "q", label: "名称 / 说明", labelKey: "resource-domain:resource.nameDescription", type: "text" }],
  columns: [{ key: "name", label: "名称", labelKey: "common-domain:common.name", sortable: true }, { key: "description", label: "说明", labelKey: "common-domain:common.description" }, { key: "version", label: "版本", labelKey: "common-domain:common.version" }],
  actions: [
    { label: "查看", labelKey: "common-domain:common.view", to: r => ({ id: r.id, mode: "view" }) },
    { label: "使用模板", labelKey: "resource-domain:template.use", permission: `${target}.create`, to: () => ({ group, item, mode: "new" }), query: r => ({ template: r.id }) },
    { label: "复制", labelKey: "resource-domain:resource.copy", permission: `${resource}.create`, to: () => ({ mode: "new" }), query: r => ({ copy: r.id }) },
    { label: "编辑", labelKey: "common-domain:common.edit", permission: `${resource}.update`, to: r => ({ id: r.id, mode: "edit" }) },
    { label: "删除模板", labelKey: "resource-domain:template.delete", permission: `${resource}.delete`, destructive: true, action: r => deleteResource(workspace, resource, r.id) },
  ], toolbar: { create: true, import: false, export: false },
});
const templateForm = (resource: string, application: boolean) => form<Template>({
  resource, heading: r => r.name,
  blank: { name: "", description: "", content: resource === "config-templates" ? "port={{ .Values.port }}\n" : application ? JSON.stringify({apiVersion:"ferry/v1",kind:"Application",metadata:{name:"web"},services:[{name:"web",image:"{{ .Values.image }}"}]},null,2) : JSON.stringify({engine:"github-actions",workflow:DEFAULT_ACTIONS.replace("echo hello", "{{ .Values.command }}"),executor_bindings:[],checkout_bindings:[],graph_layout:{}},null,2),
    parameters: resource === "config-templates" ? JSON.stringify([{name:"port",type:"integer",default:8080,min:1,max:65535}],null,2) : JSON.stringify([{name: application ? "image" : "command",label: application ? commonText("resource-domain:template.image") : commonText("resource-domain:template.buildCommand"),type:"string",default:application ? "nginx:alpine" : "echo build",min_length:1}],null,2) },
  copy: r => ({ name: commonText("resource-domain:resource.copyName", { name: r.name }), description: r.description, content: r.content, parameters: r.parameters }),
  sections: [{ fields: [
    { key: "name", label: "名称", labelKey: "common-domain:common.name", type: "text", required: true },
    { key: "description", label: "说明", labelKey: "common-domain:common.description", type: "textarea" },
    { key: "parameters", label: "参数定义（JSON 数组）", labelKey: "resource-domain:template.parametersJson", type: "code", span: 2, required: true, help: '每项包含 name、type（string/integer/boolean），可选 label、default、enum（字符串）、min/max（整数）、min_length/max_length。无 default 时必填。', helpKey: "resource-domain:template.parameterHelp" },
    { key: "content", label: resource === "config-templates" ? "应用配置文件内容" : application ? "应用模板（YAML / JSON）" : "流水线模板（JSON / YAML）", labelKey: resource === "config-templates" ? "resource-domain:template.fileContent" : application ? "resource-domain:template.appContent" : "resource-domain:template.pipelineContent", type: "code", span: 2, required: true, help: resource === "config-templates" ? '例如 nginx.conf、application.yml 或 .toml 的内容，使用 {{ .Values.参数名 }} 替换参数。应用服务的 files 中填写 path（目标文件路径）、fromTemplate（此配置名称）和 values（参数）。秘密参数只填写变量名；回滚恢复当时的文件内容。' : application ? '使用 "{{ .Values.image }}" 等占位符；secret: true 参数填写变量名，只能作为 valueFrom 的完整值。保存校验模板语法，生成时校验完整配置。' : '使用 "{{ .Values.command }}" 等创建参数；保留 GitHub Actions ${{ ... }} 运行时表达式。secret 参数填写变量名，作为 Actions env 的完整值生成 secrets 引用。', helpKey: resource === "config-templates" ? "resource-domain:template.fileHelp" : application ? "resource-domain:template.appHelp" : "resource-domain:template.pipelineHelp" },
  ] }],
});
const templateGenerator = (workspace: string, resource: string, field: string): NonNullable<FormConfig<AnyRow>["generator"]> => ({
  options: namedOptions(workspace, resource),
  load: async id => {
    const row = await api<Template>(`/workspaces/${workspace}/${resource}/${id}`);
    const parameters = JSON.parse(row.parameters) as TemplateParameter[];
    return { name: row.name, version: row.version, values: Object.fromEntries(parameters.filter(p => p.default !== undefined).map(p => [p.name, p.type === "boolean" ? String(p.default) : p.default])),
      fields: parameters.map(p => ({ key: p.name, get label() { return `${p.label || p.name}${p.secret ? commonText("resource-domain:template.secretNameSuffix") : ""}`; }, type: p.enum || p.type === "boolean" ? "select" as const : p.type === "integer" ? "number" as const : "text" as const,
        required: p.default === undefined,
        ...(p.enum ? { options: p.enum.map(value => ({value,label:value})) } : p.type === "boolean" ? { options: [{value:"true",label:"是", labelKey: "common-domain:common.yes"},{value:"false",label:"否", labelKey: "common-domain:common.no"}] } : {}),
        get help() { return p.secret ? commonText("resource-domain:template.secretHelp") : `${p.name}${p.min !== undefined ? commonText("resource-domain:parameter.minimum", { value: p.min }) : ""}${p.max !== undefined ? commonText("resource-domain:parameter.maximum", { value: p.max }) : ""}${p.min_length !== undefined ? commonText("resource-domain:parameter.minLength", { value: p.min_length }) : ""}${p.max_length !== undefined ? commonText("resource-domain:parameter.maxLength", { value: p.max_length }) : ""}`; },
      })),
    };
  },
  generate: async (id, version, values) => {
    // The displayed template version is checked by the server; metadata fetch
    // here is only for typed controls, never an implicit version upgrade.
    const row = await api<Template>(`/workspaces/${workspace}/${resource}/${id}`);
    const parameters = JSON.parse(row.parameters) as TemplateParameter[];
    const typed = Object.fromEntries(Object.entries(values).filter(([key,value]) => !(parameters.find(p => p.name === key)?.type === "integer" && value === "")).map(([key,value]) => [key, parameters.find(p => p.name === key)?.type === "boolean" ? value === "true" : value]));
    const result = await api<Record<string,string>>(`/workspaces/${workspace}/${resource}/${id}/renderings`, { method: "POST", body: JSON.stringify({ version, values: typed }) });
    if (resource === "pipeline-templates") {
      const rendered = JSON.parse(result[field] ?? "{}") as Record<string, unknown>;
      if ((rendered.engine === "github-actions" || rendered.engine === "actions") && typeof rendered.workflow === "string") {
        parseActions(rendered.workflow);
        return { definition: rendered.workflow, definition_format: "github-actions", executor_bindings: rendered.executor_bindings ?? [], checkout_bindings: rendered.checkout_bindings ?? [], graph_layout: rendered.graph_layout ?? {} };
      }
      throw new Error(commonText("resource-domain:template.invalidActions"));
    }
    return { [field]: result[field] };
  },
});

interface MarketSource {id:string;name:string;repository_id:string;provider:string;api_base:string;project:string;organization:string;branch:string;index_path:string;state:string;last_error:string;synced_at:number;version:number}
interface MarketEntry {id:string;source_id:string;entry_key:string;name:string;description:string;content:string;parameters:string;version:number}
const marketProviders=[{value:"github",label:"GitHub"},{value:"gitlab",label:"GitLab"},{value:"gitee",label:"Gitee"},{value:"codeup",label:"Codeup"}];
const marketInstall=(workspace:string)=>({label:"导入应用模板", labelKey: "resource-domain:template.importApp",permission:"marketplace.install",action:async(r:MarketEntry)=>{const created=await runItemAction<{id:string}>(workspace,"marketplace",r.id,"install",{version:r.version});navigate({group:"deploy",item:"app-templates",id:created.id,mode:"view"},{});}});
interface AlertRule {id:string;name:string;description:string;target_kind:string;target_ref:string;metric:string;threshold:number;enabled:boolean;has_webhook:boolean;webhook?:string;clear_webhook?:boolean;last_error:string;version:number}
interface AlertEvent {id:string;rule_id:string;rule_name:string;state:string;message:string;triggered_at:number;recovered_at:number;trigger_delivery:string;recovery_delivery:string;version:number}
const alertTargets=[{value:"host",label:"主机", labelKey: "common-domain:nav.hosts"},{value:"pipeline",label:"流水线", labelKey: "common-domain:shell.pipeline"},{value:"deployment",label:"应用", labelKey: "common-domain:nav.applications"},{value:"cron-job",label:"定时任务", labelKey: "common-domain:nav.cronJobs"}];
const alertMetrics=[{value:"offline",label:"离线", labelKey: "common-domain:dashboard.offline"},{value:"load1",label:"一分钟负载", labelKey: "resource-domain:alert.load"},{value:"cpu-percent",label:"CPU 使用率（%）", labelKey: "resource-domain:alert.cpu"},{value:"memory-percent",label:"内存使用率（%）", labelKey: "resource-domain:alert.memory"},{value:"disk-percent",label:"磁盘使用率（%）", labelKey: "resource-domain:alert.disk"},{value:"execution-failed",label:"最近执行失败", labelKey: "resource-domain:alert.recentFailure"}];
const alertEventLink={label:"告警事件", labelKey: "common-domain:nav.alertEvents",to:()=>({group:"deploy",item:"alert-events",mode:"list" as const})};
interface CronJob {id:string;name:string;description:string;scope:string;schedule:string;timezone:string;enabled:boolean;action:string;target_kind:string;target_ref:string;environment_id:string;command:string;timeout_seconds:number;max_retries:number;retry_delay_seconds:number;next_run_at:number;run_as_name:string;version:number}
interface CronRun {id:string;job_id:string;job_name:string;scope:string;state:string;scheduled_at:number;started_at:number;finished_at:number;attempt:number;output:string;config:string;downstream_id:string;downstream_kind:string;version:number;trigger_kind:string}
const cronState=(state:string)=>({queued:commonText("common-domain:dashboard.queued"),running:commonText("resource-domain:cron.running"),succeeded:commonText("common-domain:dashboard.succeeded"),failed:commonText("common-domain:dashboard.failed"),skipped:commonText("common-domain:dashboard.skipped"),"needs-review":commonText("resource-domain:cron.needsReview")}[state]??state);
const cronDate=(value:number)=>value?new Date(value).toLocaleString(currentLocale()):"—";
const cronReview=(workspace:string)=>async(row:CronRun)=>{const note=prompt(commonText("resource-domain:cron.unblockHelp"));if(note===null)return;await runItemAction(workspace,"cron-runs",row.id,"acknowledge",{version:row.version,note});};
const cronLinks=(workspace:string)=>[
 {label:"返回任务", labelKey: "resource-domain:cron.backTask",to:(r:CronRun)=>({group:r.scope,item:"cron-jobs",id:r.job_id,mode:"view" as const})},
 {label:"查看下游执行", labelKey: "resource-domain:cron.downstream",when:(r:CronRun)=>r.downstream_id!=="0",to:(r:CronRun)=>({group:r.downstream_kind==="pipeline-runs"?"build":"deploy",item:r.downstream_kind==="pipeline-runs"?"runs":"revisions",id:r.downstream_id,mode:"view" as const})},
 {label:"核实并解除阻塞", labelKey: "resource-domain:cron.verifyUnblock",permission:"cron-jobs.run",when:(r:CronRun)=>r.state==="needs-review",action:cronReview(workspace)},
];

export function listConfigs(workspace: string, permissions?: Set<string>, onChanged?: () => void, hostStates: Option[] = STATES): Record<string, ListConfig<AnyRow>> {
  return {
    "marketplace-sources": list<MarketSource>({resource:"marketplace-sources",rowKey:r=>r.id,filters:[{key:"q",label:"名称", labelKey: "common-domain:common.name",type:"text"}],columns:[{key:"name",label:"名称", labelKey: "common-domain:common.name",sortable:true},{key:"provider",label:"平台", labelKey: "resource-domain:repository.provider"},{key:"state",label:"同步状态", labelKey: "resource-domain:market.syncState",render:r=>({idle:commonText("resource-domain:market.idle"),syncing:commonText("resource-domain:market.syncing"),failed:commonText("common-domain:dashboard.failed")}[r.state]??r.state)},{key:"synced_at",label:"上次成功", labelKey: "resource-domain:market.lastSuccess",render:r=>cronDate(r.synced_at)},{key:"last_error",label:"同步错误", labelKey: "resource-domain:market.syncError"}],actions:[{label:"查看", labelKey: "common-domain:common.view",to:r=>({id:r.id,mode:"view"})},{label:"编辑", labelKey: "common-domain:common.edit",permission:"marketplace-sources.update",to:r=>({id:r.id,mode:"edit"})},{label:"立即同步", labelKey: "resource-domain:market.syncNow",permission:"marketplace-sources.sync",action:async r=>{await runItemAction(workspace,"marketplace-sources",r.id,"sync",{});}},{label:"目录", labelKey: "resource-domain:market.catalog",to:()=>({group:"deploy",item:"marketplace",mode:"list"}),query:r=>({source_id:r.id})},{label:"删除", labelKey: "common-domain:common.delete",permission:"marketplace-sources.delete",destructive:true,action:r=>deleteResource(workspace,"marketplace-sources",r.id)}],toolbar:{import:false,export:false}}),
    marketplace: list<MarketEntry>({resource:"marketplace",rowKey:r=>r.id,filters:[{key:"q",label:"名称 / 说明", labelKey: "resource-domain:resource.nameDescription",type:"text"},!permissions||permissions.has("marketplace-sources.read")?{key:"source_id",label:"来源", labelKey: "resource-domain:market.source",type:"select",options:namedOptions(workspace,"marketplace-sources")}:{key:"source_id",label:"来源 ID", labelKey: "resource-domain:market.sourceId",type:"text"}],columns:[{key:"name",label:"应用模板", labelKey: "resource-domain:market.appTemplate",sortable:true},{key:"description",label:"说明", labelKey: "common-domain:common.description"},{key:"entry_key",label:"目录标识", labelKey: "resource-domain:market.catalogId"}],actions:[{label:"查看", labelKey: "common-domain:common.view",to:r=>({id:r.id,mode:"view"})},marketInstall(workspace)],toolbar:{create:false,import:false,export:false}}),
    "alert-rules": list<AlertRule>({resource:"alert-rules",permissionResource:"alerts",rowKey:r=>r.id,filters:[{key:"q",label:"名称", labelKey: "common-domain:common.name",type:"text"},{key:"target_kind",label:"目标类型", labelKey: "resource-domain:resource.targetType",type:"select",options:alertTargets}],columns:[{key:"name",label:"名称", labelKey: "common-domain:common.name",sortable:true},{key:"metric",label:"指标", labelKey: "resource-domain:alert.metric",render:r=>label(alertMetrics, r.metric)},{key:"threshold",label:"阈值", labelKey: "resource-domain:alert.threshold"},{key:"enabled",label:"启用", labelKey: "resource-domain:status.enabled",render:r=>r.enabled?commonText("common-domain:common.yes"):commonText("common-domain:common.no")},{key:"has_webhook",label:"Webhook",render:r=>r.has_webhook?commonText("resource-domain:alert.configured"):commonText("resource-domain:alert.notConfigured")},{key:"last_error",label:"检查异常", labelKey: "resource-domain:alert.checkError"}],actions:[{label:"查看", labelKey: "common-domain:common.view",to:r=>({id:r.id,mode:"view"})},{label:"编辑", labelKey: "common-domain:common.edit",permission:"alerts.update",to:r=>({id:r.id,mode:"edit"})},{...alertEventLink,query:(r:AlertRule)=>({rule_id:r.id})},{label:"删除", labelKey: "common-domain:common.delete",permission:"alerts.delete",destructive:true,action:r=>deleteResource(workspace,"alert-rules",r.id)}],toolbar:{create:true,import:false,export:false}}),
    "repository-events": repositoryEventList(workspace),
    "alert-events": list<AlertEvent>({resource:"alert-events",rowKey:r=>r.id,filters:[{key:"rule_id",label:"规则", labelKey: "resource-domain:alert.rule",type:"select",options:namedOptions(workspace,"alert-rules")},{key:"state",label:"状态", labelKey: "resource-domain:status.title",type:"select",options:[{value:"firing",label:"未恢复", labelKey: "common-domain:dashboard.firing"},{value:"recovered",label:"已恢复", labelKey: "common-domain:dashboard.recovered"}]}],columns:[{key:"rule_name",label:"规则", labelKey: "resource-domain:alert.rule"},{key:"state",label:"状态", labelKey: "resource-domain:status.title",render:r=>r.state==="firing"?commonText("common-domain:dashboard.firing"):commonText("common-domain:dashboard.recovered")},{key:"triggered_at",label:"触发时间", labelKey: "resource-domain:alert.triggeredAt",sortable:true,render:r=>cronDate(r.triggered_at)},{key:"recovered_at",label:"恢复时间", labelKey: "resource-domain:alert.recoveredAt",render:r=>cronDate(r.recovered_at)},{key:"trigger_delivery",label:"触发通知", labelKey: "resource-domain:alert.triggerDelivery"},{key:"recovery_delivery",label:"恢复通知", labelKey: "resource-domain:alert.recoveryDelivery"}],actions:[{label:"查看", labelKey: "common-domain:common.view",to:r=>({id:r.id,mode:"view"})}],toolbar:{create:false,import:false,export:false}}),
    "cron-jobs": list<CronJob>({resource:"cron-jobs",rowKey:r=>r.id,filters:[{key:"q",label:"名称 / 说明", labelKey: "resource-domain:resource.nameDescription",type:"text"}],columns:[{key:"name",label:"名称", labelKey: "common-domain:common.name",sortable:true},{key:"schedule",label:"Cron 表达式", labelKey: "resource-domain:cron.expression"},{key:"timezone",label:"时区", labelKey: "resource-domain:cron.timezone"},{key:"enabled",label:"启用", labelKey: "resource-domain:status.enabled",render:r=>r.enabled?commonText("common-domain:common.yes"):commonText("common-domain:common.no")},{key:"next_run_at",label:"下次执行", labelKey: "resource-domain:cron.nextRun",sortable:true,render:r=>cronDate(r.next_run_at)},{key:"run_as_name",label:"执行身份", labelKey: "resource-domain:cron.runAs"}],actions:[
      {label:"查看", labelKey: "common-domain:common.view",to:r=>({id:r.id,mode:"view"})},{label:"编辑", labelKey: "common-domain:common.edit",permission:"cron-jobs.update",to:r=>({id:r.id,mode:"edit"})},
      {label:"立即执行", labelKey: "resource-domain:cron.runNow",permission:"cron-jobs.run",action:async r=>{const result=await runItemAction<{id:string}>(workspace,"cron-jobs",r.id,"run-now",{});navigate({group:r.scope,item:"cron-runs",id:result.id,mode:"view"},{});}},
      {label:"执行历史", labelKey: "common-domain:nav.runHistory",to:r=>({group:r.scope,item:"cron-runs",mode:"list"}),query:r=>({job_id:r.id})},
      {label:"删除", labelKey: "common-domain:common.delete",permission:"cron-jobs.delete",destructive:true,action:r=>deleteResource(workspace,"cron-jobs",r.id)}
    ],toolbar:{import:false,export:false}}),
    "cron-runs": list<CronRun>({resource:"cron-runs",rowKey:r=>r.id,filters:[{key:"job_id",label:"任务", labelKey: "resource-domain:cron.task",type:"select",options:namedOptions(workspace,"cron-jobs")},{key:"state",label:"状态", labelKey: "resource-domain:status.title",type:"select",options:["queued","running","succeeded","failed","skipped","needs-review"].map(value=>({value,get label() { return cronState(value); }}))}],columns:[{key:"job_name",label:"任务", labelKey: "resource-domain:cron.task"},{key:"state",label:"状态", labelKey: "resource-domain:status.title",render:r=>cronState(r.state)},{key:"scheduled_at",label:"计划时间", labelKey: "resource-domain:cron.scheduledAt",sortable:true,render:r=>cronDate(r.scheduled_at)},{key:"attempt",label:"尝试次数", labelKey: "resource-domain:cron.attempt"},{key:"trigger_kind",label:"触发方式", labelKey: "resource-domain:cron.triggerType",render:r=>r.trigger_kind==="manual"?commonText("resource-domain:cron.manual"):commonText("resource-domain:cron.scheduled")}],actions:[{label:"查看", labelKey: "common-domain:common.view",to:r=>({id:r.id,mode:"view"})},...cronLinks(workspace)],toolbar:{create:false,import:false,export:false}}),
    settings: list(typedResourceList(workspace, "settings")),
    dictionaries: list(typedResourceList(workspace, "dictionaries")),
    "config-templates": { ...templateList(workspace, "config-templates", "deployments", "deploy", "apps"), actions: templateList(workspace, "config-templates", "deployments", "deploy", "apps").actions?.filter(a => a.label !== "使用模板") ?? [] },
    "app-templates": templateList(workspace, "app-templates", "deployments", "deploy", "apps"),
    "pipeline-templates": templateList(workspace, "pipeline-templates", "pipelines", "build", "pipelines"),
    members: list({
      resource: "members", rowKey: (r: Member) => r.id,
      filters: [{ key: "q", label: "姓名或邮箱", labelKey: "resource-domain:platform.nameEmail", type: "text" }],
      columns: [{ key: "name", label: "姓名", labelKey: "common-domain:auth.name" }, { key: "email", label: "邮箱", labelKey: "common-domain:auth.email" }, { key: "role_name", label: "角色", labelKey: "common-domain:dashboard.roles" }, { key: "state", label: "状态", labelKey: "resource-domain:status.title", render: r => r.owner ? commonText("resource-domain:member.owner") : !r.active ? commonText("resource-domain:member.disabled") : r.state === "active" ? commonText("resource-domain:member.active") : r.state }],
      actions: [
        { label: "查看", labelKey: "common-domain:common.view", to: r => ({ id: r.id, mode: "view" }) },
        { label: "调整角色", labelKey: "resource-domain:member.changeRole", permission: "members.update", when: r => !r.owner, to: r => ({ id: r.id, mode: "edit" }) },
        { label: "移除", labelKey: "resource-domain:member.remove", permission: "members.delete", when: r => !r.owner, destructive: true, action: async r => { await deleteResource(workspace, "members", r.id); onChanged?.(); } },
      ], toolbar: { create: false, import: false, export: false, refresh: [0, 30] },
    }),
    roles: list({
      resource: "roles", rowKey: (r: Role) => r.id,
      filters: [{ key: "q", label: "名称", labelKey: "common-domain:common.name", type: "text" }],
      columns: [{ key: "name", label: "名称", labelKey: "common-domain:common.name", sortable: true }, { key: "builtin", label: "类型", labelKey: "resource-domain:resource.type", render: r => r.builtin ? commonText("resource-domain:role.builtin") : commonText("resource-domain:role.custom") }, { key: "permissions", label: "权限数", labelKey: "resource-domain:role.permissionCount", render: r => r.permissions.length }],
      actions: [
        { label: "查看", labelKey: "common-domain:common.view", to: r => ({ id: r.id, mode: "view" }) },
        { label: "复制", labelKey: "resource-domain:resource.copy", permission: "roles.create", when: r => r.permissions.every(p => permissions?.has(p) ?? true), to: () => ({ mode: "new" }), query: r => ({ copy: r.id }) },
        { label: "编辑", labelKey: "common-domain:common.edit", permission: "roles.update", when: r => !r.builtin, to: r => ({ id: r.id, mode: "edit" }) },
        { label: "删除", labelKey: "common-domain:common.delete", permission: "roles.delete", when: r => !r.builtin, destructive: true, action: async r => { await deleteResource(workspace, "roles", r.id); onChanged?.(); } },
      ], toolbar: { create: true, import: false, export: false, refresh: [0, 30] },
    }),
    deployments: list({
      resource: "deployments", rowKey: (r: Deployment) => r.id,
      filters: [{ key: "q", label: "名称", labelKey: "common-domain:common.name", type: "text" }],
      columns: [{ key: "name", label: "名称", labelKey: "common-domain:common.name", sortable: true }, { key: "state", label: "状态", labelKey: "resource-domain:status.title", render: (r) => deploymentState(r.state) }, { key: "target_name", label: "目标", labelKey: "resource-domain:resource.target", render: (r) => `${label(TARGETS, r.target_kind)} · ${named((r as AnyRow).target_name as string, r.target_ref)}` }, { key: "description", label: "备注", labelKey: "resource-domain:resource.notes" }],
      actions: [
        { label: "查看", labelKey: "common-domain:common.view", permission: "deployments.read", to: (r) => ({ id: r.id, mode: "view" }) },
        { label: "编辑", labelKey: "common-domain:common.edit", permission: "deployments.update", to: (r) => ({ id: r.id, mode: "edit" }) },
        { label: "删除记录（保留容器和卷）", labelKey: "resource-domain:deployment.deleteRecord", permission: "deployments.delete", destructive: true, action: (r) => deleteResource(workspace, "deployments", r.id) },
      ],
    }),
    "deployment-revisions": list({
      resource: "deployment-revisions", rowKey: (r: Revision) => r.id,
      filters: [{ key: "deployment_id", label: "应用", labelKey: "common-domain:nav.applications", type: "select", options: namedOptions(workspace, "deployments") }],
      columns: [{ key: "number", label: "Revision", sortable: true }, { key: "deployment_name", label: "应用", labelKey: "common-domain:nav.applications", render: (r) => named((r as AnyRow).deployment_name as string, r.deployment_id) }, { key: "action", label: "操作", labelKey: "common-domain:table.actions", render: (r) => r.action === "rollback" ? commonText("resource-domain:deployment.rollback") : commonText("resource-domain:variable.deploy") }, { key: "state", label: "状态", labelKey: "resource-domain:status.title", render: (r) => deploymentState(r.state) }, { key: "started_at", label: "开始时间", labelKey: "resource-domain:execution.startedAt", render: (r) => r.started_at ? new Date(r.started_at).toLocaleString(currentLocale()) : "—" }],
      actions: [{ label: "查看", labelKey: "common-domain:common.view", permission: "deployments.read", to: (r) => ({ id: r.id, mode: "view" }) }],
      toolbar: { create: false, import: false, export: false },
    }),
    "code-insights": list(codeInsightList(workspace)),
    "code-connections": list<CodeConnection>({
      resource: "code-connections", permissionResource:"repositories", rowKey: row => row.id,
      filters: [{ key: "q", label: "名称", labelKey: "common-domain:common.name", type: "text" }],
      columns: [{key:"name",label:"连接名称", labelKey: "resource-domain:repository.connectionName"},{key:"provider",label:"平台", labelKey: "resource-domain:repository.provider"},{key:"account",label:"账号", labelKey: "resource-domain:credential.account"},{key:"state",label:"连接状态", labelKey: "resource-domain:repository.connectionState",render:row=>({connected:commonText("resource-domain:runner.connected"),expired:commonText("resource-domain:repository.authExpired"),refreshable:commonText("resource-domain:repository.authRefreshable"),failed:commonText("resource-domain:repository.connectionFailed")}[row.state]??row.state)},{key:"description",label:"备注", labelKey: "resource-domain:resource.notes"}],
      actions: [{label:"查看", labelKey: "common-domain:common.view",permission:"repositories.read",to:row=>({id:row.id,mode:"view"})},{label:"编辑", labelKey: "common-domain:common.edit",permission:"repositories.update",to:row=>({id:row.id,mode:"edit"})},{label:"删除", labelKey: "common-domain:common.delete",permission:"repositories.delete",destructive:true,action:row=>deleteResource(workspace,"code-connections",row.id)}],
      toolbar:{create:{label:"创建连接", labelKey: "resource-domain:repository.createConnection"}},empty:"尚无仓库连接。创建连接后，在流水线的拉取代码节点选择仓库和分支。", emptyKey: "resource-domain:repository.noConnections",
    }),
    ...Object.fromEntries((["pipeline-groups","pipeline-environments"] as const).map(resource=>[resource,list<PipelineContext>({
      resource,rowKey:row=>row.id,filters:[{key:"q",label:"名称", labelKey: "common-domain:common.name",type:"text"}],
      columns:[{key:"name",label:"名称", labelKey: "common-domain:common.name"},{key:"description",label:"说明", labelKey: "common-domain:common.description"}],
      actions:[{label:"查看", labelKey: "common-domain:common.view",permission:resource+".read",to:row=>({id:row.id,mode:"view"})},{label:"编辑", labelKey: "common-domain:common.edit",permission:resource+".update",to:row=>({id:row.id,mode:"edit"})},{label:"删除", labelKey: "common-domain:common.delete",permission:resource+".delete",destructive:true,action:row=>deleteResource(workspace,resource,row.id)}]
    })])),
    pipelines: list({
      resource: "pipelines", rowKey: (r: Pipeline) => r.id,
      toolbar: { create: { label: "创建流水线", labelKey: "resource-domain:pipeline.create" } },
      filters: [{ key: "q", label: "名称", labelKey: "common-domain:common.name", type: "text" },
        ...(!permissions || permissions.has("pipeline-groups.read") ? [{key:"group_id",label:"分组", labelKey: "common-domain:table.groups",type:"select" as const,options:namedOptions(workspace,"pipeline-groups")}] : []),
        ...(!permissions || permissions.has("deployments.read") ? [{key:"application_id",label:"应用", labelKey: "common-domain:nav.applications",type:"select" as const,options:namedOptions(workspace,"deployments")}] : []),
        ...(!permissions || permissions.has("tags.read") ? [{key:"tag_id",label:"标签", labelKey: "common-domain:nav.tags",type:"select" as const,options:tagOptions(workspace)}] : [])],
      columns: [
        { key: "name", label: "名称", labelKey: "common-domain:common.name" },
        {key:"group_name",label:"分组", labelKey: "common-domain:table.groups"},{key:"application_name",label:"应用", labelKey: "common-domain:nav.applications"},
        {key:"tags",label:"标签", labelKey: "common-domain:nav.tags",render:row=><span className="flex flex-wrap gap-1">{row.tags?.map(tag=><Badge key={tag.id} variant="secondary">{tag.name}</Badge>)}</span>},
        { key: "repository_name", label: "仓库", labelKey: "common-domain:dashboard.repositories", render: (r: AnyRow) => named(r.repository_name as string, r.repository_id as string) },
        { key: "target_kind", label: "目标类型", labelKey: "resource-domain:resource.targetType", render: (r) => label(TARGETS, r.target_kind) },
        { key: "target_name", label: "目标", labelKey: "resource-domain:resource.target", render: (r: AnyRow) => named(r.target_name as string, r.target_ref as string) }, { key: "description", label: "备注", labelKey: "resource-domain:resource.notes" },
      ],
      actions: [
        { label: "查看", labelKey: "common-domain:common.view", permission: "pipelines.read", to: (r) => ({ id: r.id, mode: "view" }) },
        pipelineRunAction(workspace),
        { label: "编辑", labelKey: "common-domain:common.edit", permission: "pipelines.update", to: (r) => ({ id: r.id, mode: "edit" }) },
        { label: "删除", labelKey: "common-domain:common.delete", permission: "pipelines.delete", destructive: true,
          action: (r) => deleteResource(workspace, "pipelines", r.id) },
      ],
    }),
    "pipeline-artifacts": list({
      resource: "pipeline-artifacts", permissionResource: "pipelines", rowKey: (r: { id: string }) => r.id,
      filters: [{ key: "q", label: "文件路径", labelKey: "resource-domain:artifact.path", type: "text" }, { key: "run_id", label: "执行 ID", labelKey: "resource-domain:execution.id", type: "text" }, { key: "pipeline_id", label: "流水线", labelKey: "common-domain:shell.pipeline", type: "select", options: namedOptions(workspace, "pipelines") }],
      columns: [
        { key: "path", label: "文件路径", labelKey: "resource-domain:artifact.path" }, { key: "run_id", label: "执行", labelKey: "resource-domain:execution.title", render: (r: AnyRow) => r.run_number ? `${named(r.pipeline_name as string, r.pipeline_id as string)} #${r.run_number}` : named("", r.run_id as string) },
        { key: "size", label: "大小（字节）", labelKey: "resource-domain:artifact.bytes", sortable: true }, { key: "sha256", label: "SHA-256" },
        { key: "download", label: "下载", labelKey: "resource-domain:artifact.download", render: (r: { id: string; path: string }) => <a className="text-primary underline" href={`/api/workspaces/${encodeURIComponent(workspace)}/pipeline-artifacts/${r.id}/content`} download>{commonText("resource-domain:artifact.download")}{r.path}</a> },
      ],
      toolbar: { create: false, import: false, export: false },
      empty: "还没有构建产物。在流水线定义的 artifacts 中声明相对文件路径，构建成功后会自动归档。", emptyKey: "resource-domain:artifact.empty",
    }),
    "pipeline-runs": list({
      resource: "pipeline-runs", rowKey: (r: PipelineRun) => r.id,
      filters: [{ key: "state", label: "状态", labelKey: "resource-domain:status.title", type: "select", options:
        ["queued", "running", "cancelling", "succeeded", "failed", "cancelled"].map((value) => ({ value, get label() { return stateBadge(value).label; } })) }],
      columns: [
        { key: "number", label: "执行序号", labelKey: "resource-domain:execution.number", sortable: true, render: (r) => `#${r.number}` },
        { key: "pipeline_name", label: "流水线", labelKey: "common-domain:shell.pipeline", render: (r) => named(r.pipeline_name, r.pipeline_id) },
        { key: "state", label: "状态", labelKey: "resource-domain:status.title", render: (r) => {
          const { label, ...props } = stateBadge(r.state);
          return <Badge {...props}>{label}</Badge>;
        } },
        { key: "triggered_by_name", label: "触发人", labelKey: "resource-domain:execution.triggeredBy", render: (r) => named(r.triggered_by_name, r.triggered_by) },
        { key: "duration", label: "耗时", labelKey: "resource-domain:execution.duration", render: (r) => duration(r.started_at, r.finished_at) },
        { key: "started_at", label: "开始时间", labelKey: "resource-domain:execution.startedAt", render: (r) => r.started_at ? new Date(r.started_at).toLocaleString(currentLocale()) : "—" },
      ],
      actions: [{ label: "查看", labelKey: "common-domain:common.view", permission: "pipelines.read", to: (r) => ({ id: r.id, mode: "view" }) }],
      toolbar: { create: false, import: false, export: false },
    }),
    "host-groups": list({
      resource: "host-groups",
      rowKey: (row: HostGroup) => row.id,
      filters: [{ key: "q", label: "名称/说明", labelKey: "resource-domain:resource.searchNameDescription", type: "text", placeholder: "输入关键字", placeholderKey: "resource-domain:resource.keyword" }],
      columns: [
        { key: "name", label: "名称", labelKey: "common-domain:common.name" },
        { key: "description", label: "说明", labelKey: "common-domain:common.description" },
      ],
      actions: [
        { label: "查看", labelKey: "common-domain:common.view", permission: "host-groups.read", to: (row: HostGroup) => ({ id: row.id, mode: "view" }) },
        { label: "编辑", labelKey: "common-domain:common.edit", permission: "host-groups.update", to: (row: HostGroup) => ({ id: row.id, mode: "edit" }) },
        {
          label: "删除", labelKey: "common-domain:common.delete",
          permission: "host-groups.delete",
          destructive: true,
          action: (row: HostGroup) => deleteResource(workspace, "host-groups", row.id),
        },
      ],
      empty: "还没有主机组。先建一个，再把主机归进去。", emptyKey: "resource-domain:hostGroup.empty",
    }),

    hosts: list({
      resource: "hosts",
      defaultRefresh: 10,
      toolbar: { create: { label: "添加主机", labelKey: "common-domain:form.addHost" } },
      rowKey: (row: Host) => row.id,
      filters: [
        { key: "tag_id", label: "标签", labelKey: "common-domain:nav.tags", type: "select", options: tagOptions(workspace) },
        { key: "q", label: "名称/地址/账号", labelKey: "resource-domain:host.search", type: "text", placeholder: "输入关键字", placeholderKey: "resource-domain:resource.keyword" },
        { key: "group_id", label: "主机组", labelKey: "common-domain:nav.hostGroups", type: "select", options: groupOptions(workspace) },
        { key: "connection", label: "接入方式", labelKey: "common-domain:form.connection", type: "select", options: CONNECTIONS },
        { key: "state", label: "状态", labelKey: "resource-domain:status.title", type: "select", options: hostStates },
      ],
      columns: [
        { key: "name", label: "名称", labelKey: "common-domain:common.name" },
        { key: "tags", label: "标签", labelKey: "common-domain:nav.tags", render: (row: Host) => <span className="flex flex-wrap gap-1">{row.tags?.map(tag => <Badge variant="secondary" key={tag.id}>{tag.name}</Badge>)}</span> },
        { key: "address", label: "地址", labelKey: "resource-domain:host.address", width: 180 },
        {
          key: "connection",
          label: "接入方式", labelKey: "common-domain:form.connection",
          width: 130,
          render: (row: Host) => label(CONNECTIONS, row.connection),
        },
        {
          key: "state",
          label: "状态", labelKey: "resource-domain:status.title",
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
              {row.connection === "runner" && row.state === "offline" ? commonText("resource-domain:runner.disconnected") : label(hostStates, row.state)}
            </Badge>
          ),
        },
        { key: "seen_at", label: "最近在线", labelKey: "resource-domain:host.recentlyOnline", width: 150, sortable: true, render: (row: Host) => row.seen_at ? `${row.state === "online" ? commonText("resource-domain:host.updatedAt") : commonText("resource-domain:host.lastOnline")} ${ago(row.seen_at)}` : commonText("resource-domain:host.neverConnected") },
      ],
      actions: [
        { label: "查看", labelKey: "common-domain:common.view", permission: "hosts.read", to: (row: Host) => ({ id: row.id, mode: "view" }) },
        { label: "打开终端", labelKey: "common-domain:shell.openTerminal", icon: <SquareTerminal className="size-4" />, newTab: true, permission: "hosts.terminal", to: (row: Host) => ({ id: row.id, mode: "workspace" }), query: () => ({ tool: "terminal" }) },
        { label: "打开文件", labelKey: "common-domain:shell.openFiles", icon: <FolderOpen className="size-4" />, newTab: true, permission: "hosts.files", to: (row: Host) => ({ id: row.id, mode: "workspace" }), query: () => ({ tool: "files" }) },
        {
          label: "测试连接", labelKey: "resource-domain:host.testConnection",
          permission: "hosts.read",
          action: async (row: Host) => {
            const result = await runItemAction<{ ok: boolean; stderr: string }>(
              workspace, "hosts", row.id, "test-connection", {},
            );
            if (!result.ok) throw new Error(result.stderr || commonText("resource-domain:host.testFailed"));
          },
        },
        { label: "编辑", labelKey: "common-domain:common.edit", permission: "hosts.update", to: (row: Host) => ({ id: row.id, mode: "edit" }) },
        {
          label: "删除", labelKey: "common-domain:common.delete",
          permission: "hosts.delete",
          destructive: true,
          action: (row: Host) => deleteResource(workspace, "hosts", row.id),
        },
      ],
      bulk: [
        {
          label: "批量删除", labelKey: "resource-domain:resource.bulkDelete",
          permission: "hosts.delete",
          destructive: true,
          action: async (ids) => {
            await runAction(workspace, "hosts", "bulk-delete", { ids });
          },
        },
      ],
      empty: "还没有主机。用代理一条命令接入，或填 SSH 账号直连。", emptyKey: "resource-domain:host.empty",
    }),

    tags: list({
      resource: "tags",
      rowKey: (row: Tag) => row.id,
      filters: [{ key: "q", label: "名称", labelKey: "common-domain:common.name", type: "text", placeholder: "输入关键字", placeholderKey: "resource-domain:resource.keyword" }],
      columns: [
        { key: "name", label: "名称", labelKey: "common-domain:common.name" },
        {
          key: "color",
          label: "颜色", labelKey: "resource-domain:tag.color",
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
        { label: "编辑", labelKey: "common-domain:common.edit", permission: "tags.update", to: (row: Tag) => ({ id: row.id, mode: "edit" }) },
        {
          label: "删除", labelKey: "common-domain:common.delete",
          permission: "tags.delete",
          destructive: true,
          action: (row: Tag) => deleteResource(workspace, "tags", row.id),
        },
      ],
      empty: "还没有标签。", emptyKey: "resource-domain:tag.empty",
    }),

    variables: list({
      resource: "variables", rowKey: (row: Variable) => row.id,
      filters: [{ key: "q", label: "名称 / 备注", labelKey: "resource-domain:resource.nameNotes", type: "text" }, { key: "kind", label: "类型", labelKey: "resource-domain:resource.type", type: "select", options: VARIABLE_KINDS }, { key: "scope_ref", label: "对象 ID", labelKey: "resource-domain:variable.objectId", type: "text", placeholder: "0 表示模块默认", placeholderKey: "resource-domain:variable.zeroDefault" }],
      columns: [
        { key: "name", label: "名称", labelKey: "common-domain:common.name", sortable: true },
        { key: "kind", label: "类型", labelKey: "resource-domain:resource.type", render: (row) => label(VARIABLE_KINDS, row.kind) },
        { key: "scope", label: "来源层", labelKey: "resource-domain:variable.sourceLevel", render: variableSource },
        { key: "value", label: "值", labelKey: "resource-domain:variable.value", render: (row) => row.kind === "secret" ? (row.has_value ? commonText("common-domain:common.set") : commonText("common-domain:common.notSet")) : <span className="whitespace-pre-wrap break-all">{row.value === "" ? commonText("common-domain:form.emptyString") : row.value}</span> },
        { key: "description", label: "备注", labelKey: "resource-domain:resource.notes" },
      ],
      actions: [
        { label: "查看", labelKey: "common-domain:common.view", permission: "variables.read", to: (row) => ({ id: row.id, mode: "view" }) },
        { label: "编辑", labelKey: "common-domain:common.edit", permission: "variables.update", to: (row) => ({ id: row.id, mode: "edit" }) },
        { label: "删除", labelKey: "common-domain:common.delete", permission: "variables.delete", destructive: true, action: (row) => deleteResource(workspace, "variables", row.id) },
      ],
      toolbar: { import: false, export: false },
      empty: "当前作用域没有变量。全局、模块和对象同名时，具体对象优先。", emptyKey: "resource-domain:variable.empty",
    }),

    credentials: list({
      resource: "credentials",
      rowKey: (row: Credential) => row.id,
      filters: [
        { key: "q", label: "名称/账号", labelKey: "resource-domain:credential.search", type: "text", placeholder: "输入关键字", placeholderKey: "resource-domain:resource.keyword" },
        { key: "kind", label: "类型", labelKey: "resource-domain:resource.type", type: "select", options: CREDENTIAL_KINDS },
      ],
      columns: [
        { key: "name", label: "名称", labelKey: "common-domain:common.name" },
        { key: "kind", label: "类型", labelKey: "resource-domain:resource.type", width: 120, render: (r: Credential) => label(CREDENTIAL_KINDS, r.kind) },
        { key: "username", label: "账号", labelKey: "resource-domain:credential.account", width: 160 },
        {
          key: "has_secret",
          label: "内容", labelKey: "common-domain:common.content",
          width: 100,
          render: (r: Credential) =>
            r.has_secret ? (
              <Badge variant="secondary">{commonText("common-domain:common.set")}</Badge>
            ) : (
              <Badge variant="destructive">{commonText("common-domain:common.notSet")}</Badge>
            ),
        },
      ],
      actions: [
        { label: "编辑", labelKey: "common-domain:common.edit", permission: "credentials.update", to: (r: Credential) => ({ id: r.id, mode: "edit" }) },
        {
          label: "删除", labelKey: "common-domain:common.delete",
          permission: "credentials.delete",
          destructive: true,
          action: (r: Credential) => deleteResource(workspace, "credentials", r.id),
        },
      ],
      toolbar: { import: false, export: false },
      empty: "当前模块还没有凭据，添加后可在本模块中选择使用。", emptyKey: "resource-domain:credential.empty",
    }),

    activity: list({
      resource: "activity",
      rowKey: (row: Activity) => row.id,
      filters: [
        { key: "q", label: "操作人/资源", labelKey: "resource-domain:activity.search", type: "text", placeholder: "输入关键字", placeholderKey: "resource-domain:resource.keyword" },
        {
          key: "action",
          label: "动作", labelKey: "resource-domain:resource.action",
          type: "select",
          options: [
            { value: "create", label: "新增", labelKey: "resource-domain:activity.create" },
            { value: "update", label: "修改", labelKey: "resource-domain:activity.update" },
            { value: "delete", label: "删除", labelKey: "common-domain:common.delete" },
            { value: "bulk-delete", label: "批量删除", labelKey: "resource-domain:resource.bulkDelete" },
          ],
        },
      ],
      columns: [
        { key: "created_at", label: "时间", labelKey: "resource-domain:activity.time", width: 180, sortable: true, render: (row: Activity) => new Date(row.created_at).toLocaleString(currentLocale()) },
        { key: "actor_name", label: "操作人", labelKey: "resource-domain:activity.actor", width: 120 },
        { key: "action", label: "动作", labelKey: "resource-domain:resource.action", width: 100 },
        { key: "resource", label: "资源", labelKey: "resource-domain:activity.resource", width: 140 },
        {
          key: "details",
          label: "详情", labelKey: "common-domain:common.details",
          render: (row: Activity) => (
            <code className="text-xs text-muted-foreground">{JSON.stringify(row.details)}</code>
          ),
        },
      ],
      toolbar: { create: false, import: false },
      empty: "还没有操作记录。", emptyKey: "resource-domain:activity.empty",
    }),
  };
}

export function formConfigs(workspace: string, permissions?: Set<string>): Record<string, FormConfig<AnyRow>> {
  return {
    "marketplace-sources": form<MarketSource>({resource:"marketplace-sources",heading:r=>r.name,blank:{name:"",repository_id:"",provider:"github",api_base:"https://api.github.com",project:"",organization:"",branch:"main",index_path:"marketplace.json"},sections:[{fields:[{key:"name",label:"名称", labelKey: "common-domain:common.name",type:"text",required:true},{key:"repository_id",label:"外部仓库", labelKey: "resource-domain:market.externalRepository",type:"marketplace-repository",span:2,options:namedOptions(workspace,"repositories"),help:"从代码连接选择仓库，或保留已有仓库绑定；同步平台与 API 配置在下方填写。", helpKey: "resource-domain:market.repositoryHelp"},{key:"provider",label:"平台", labelKey: "resource-domain:repository.provider",type:"select",required:true,options:marketProviders},{key:"api_base",label:"API 接入点", labelKey: "resource-domain:repository.apiEndpoint",type:"text",required:true,help:"GitHub: https://api.github.com；GitLab: https://gitlab.com/api/v4；Gitee: https://gitee.com/api/v5；Codeup中心版: https://openapi-rdc.aliyuncs.com。自托管使用所选仓库同主机的API。", helpKey: "resource-domain:market.apiHelp"},{key:"project",label:"项目路径 / ID", labelKey: "resource-domain:repository.project",type:"text",required:true,help:"GitHub/Gitee填写owner/repository；GitLab/Codeup支持项目完整路径或ID。", helpKey: "resource-domain:market.projectHelp"},{key:"organization",label:"组织 ID", labelKey: "resource-domain:repository.organization",type:"text",when:d=>d.provider==="codeup",help:"Codeup中心版需要组织ID；Region版可留空。", helpKey: "resource-domain:market.organizationHelp"},{key:"branch",label:"分支 / 标签 / Commit", labelKey: "resource-domain:repository.ref",type:"text",required:true},{key:"index_path",label:"索引文件路径", labelKey: "resource-domain:market.indexPath",type:"text",required:true,help:"仓库内JSON文件，格式为version:1及templates数组；所有模板验证成功后才替换目录。", helpKey: "resource-domain:market.indexHelp"}]}]}),
    marketplace: form<MarketEntry>({resource:"marketplace",heading:r=>r.name,blank:{},readOnly:()=>true,viewActions:[marketInstall(workspace)],sections:[{fields:[{key:"name",label:"名称", labelKey: "common-domain:common.name",type:"text"},{key:"description",label:"说明", labelKey: "common-domain:common.description",type:"textarea",span:2},{key:"content",label:"应用配置", labelKey: "resource-domain:deployment.config",type:"code",span:2},{key:"parameters",label:"模板参数", labelKey: "resource-domain:template.parameters",type:"code",span:2}]}]}),
    "alert-rules": form<AlertRule>({resource:"alert-rules",permissionResource:"alerts",heading:r=>r.name,blank:{name:"",description:"",target_kind:"host",target_ref:"",metric:"offline",threshold:0,enabled:false,webhook:"",clear_webhook:false},viewActions:[alertEventLink],sections:[{fields:[
      {key:"name",label:"名称", labelKey: "common-domain:common.name",type:"text",required:true},{key:"enabled",label:"启用", labelKey: "resource-domain:status.enabled",type:"switch"},
      {key:"target_kind",label:"目标类型", labelKey: "resource-domain:resource.targetType",type:"select",required:true,immutable:true,options:alertTargets,resetOnChange:["target_ref","metric"]},
      ...([['host','hosts','主机'],['pipeline','pipelines','流水线'],['deployment','deployments','应用'],['cron-job','cron-jobs','定时任务']] as const).map(([kind,resource,label])=>({key:"target_ref",label:`目标${label}`,labelKey:`resource-domain:target.${kind}`,type:"select" as const,required:true,immutable:true,when:(d:Partial<AlertRule>)=>d.target_kind===kind,options:namedOptions(workspace,resource)})),
      {key:"metric",label:"指标", labelKey: "resource-domain:alert.metric",type:"select",required:true,immutable:true,when:d=>d.target_kind==="host",options:alertMetrics.filter(m=>m.value!=="execution-failed")},
      {key:"metric",label:"指标", labelKey: "resource-domain:alert.metric",type:"select",required:true,immutable:true,when:d=>d.target_kind!=="host",options:alertMetrics.filter(m=>m.value==="execution-failed")},
      {key:"threshold",label:"阈值", labelKey: "resource-domain:alert.threshold",type:"number",required:true,when:d=>!!d.metric&&!['offline','execution-failed'].includes(d.metric),help:"指标严格大于阈值时触发；内存和磁盘范围0–100，负载为非负整数。", helpKey: "resource-domain:alert.thresholdHelp"},
      {key:"webhook",label:"Webhook 地址", labelKey: "resource-domain:alert.webhook",type:"password",stored:"has_webhook",help:"可选HTTP(S)地址；加密保存、不回显。编辑留空保留原地址。触发和恢复各尝试一次，失败不自动重试。", helpKey: "resource-domain:alert.webhookHelp"},
      {key:"clear_webhook",label:"清除已有 Webhook", labelKey: "resource-domain:alert.clearWebhook",type:"switch",when:d=>!!d.has_webhook},
      {key:"description",label:"说明", labelKey: "common-domain:common.description",type:"textarea",span:2},
      {key:"last_error",label:"检查异常", labelKey: "resource-domain:alert.checkError",type:"text",immutable:true,when:d=>!!d.last_error,help:"检查执行身份的账号权限和目标是否仍然有效。", helpKey: "resource-domain:alert.identityHelp"}
    ]}]}),
    "repository-events": repositoryEventForm(),
    "alert-events": form<AlertEvent>({resource:"alert-events",heading:r=>r.rule_name,blank:{},readOnly:()=>true,pollMs:r=>r.state==="firing"||r.trigger_delivery==="投递中"||r.recovery_delivery==="投递中"?2000:0,viewActions:[{label:"返回规则", labelKey: "resource-domain:alert.backRule",to:()=>({group:"deploy",item:"alerts",mode:"list"})}],sections:[{fields:[{key:"rule_name",label:"规则", labelKey: "resource-domain:alert.rule",type:"text"},{key:"state",label:"状态", labelKey: "resource-domain:status.title",type:"select",options:[{value:"firing",label:"未恢复", labelKey: "common-domain:dashboard.firing"},{value:"recovered",label:"已恢复", labelKey: "common-domain:dashboard.recovered"}]},{key:"message",label:"触发信息", labelKey: "resource-domain:alert.message",type:"textarea",span:2},{key:"trigger_delivery",label:"触发通知", labelKey: "resource-domain:alert.triggerDelivery",type:"text"},{key:"recovery_delivery",label:"恢复通知", labelKey: "resource-domain:alert.recoveryDelivery",type:"text"}]}]}),
    "cron-jobs": form<CronJob>({resource:"cron-jobs",heading:r=>r.name,blank:{name:"",description:"",scope:"host",schedule:"0 * * * *",timezone:"",enabled:false,action:"",target_kind:"",target_ref:"",command:"",timeout_seconds:1800,max_retries:0,retry_delay_seconds:30},viewActions:[{label:"执行历史", labelKey: "common-domain:nav.runHistory",to:r=>({group:r.scope,item:"cron-runs",mode:"list"}),query:r=>({job_id:r.id})}],sections:[{fields:[
      {key:"name",label:"名称", labelKey: "common-domain:common.name",type:"text",required:true},{key:"scope",label:"所属模块", labelKey: "resource-domain:cron.module",type:"select",resetOnChange:["action","target_kind","target_ref","command"],options:[{value:"host",label:"主机", labelKey: "common-domain:nav.hosts"},{value:"build",label:"构建", labelKey: "common-domain:nav.build"},{value:"deploy",label:"部署", labelKey: "resource-domain:variable.deploy"}],immutable:true},
      {key:"enabled",label:"启用定时执行", labelKey: "resource-domain:cron.enable",type:"switch"},{key:"schedule",label:"执行周期", labelKey: "resource-domain:cron.schedule",type:"cron",required:true,span:2,help:"停机错过的触发不补跑，同任务重叠时跳过并记录。", helpKey: "resource-domain:cron.scheduleHelp"},
      {key:"timezone",label:"时区", labelKey: "resource-domain:cron.timezone",type:"text",required:true,placeholder:"Asia/Shanghai",help:"必须明确填写 IANA 时区，例如 Asia/Shanghai、Asia/Singapore、UTC。", helpKey: "resource-domain:cron.timezoneHelp"},
      {key:"action",label:"动作", labelKey: "resource-domain:resource.action",type:"select",required:true,when:d=>d.scope==="host",options:[{value:"command",label:"执行命令", labelKey: "resource-domain:cron.commandAction"}]},
      {key:"action",label:"动作", labelKey: "resource-domain:resource.action",type:"select",required:true,when:d=>d.scope==="build",options:[{value:"pipeline",label:"触发流水线", labelKey: "resource-domain:cron.pipelineAction"}]},
      {key:"action",label:"动作", labelKey: "resource-domain:resource.action",type:"select",required:true,when:d=>d.scope==="deploy",options:[{value:"redeploy",label:"重新部署当前配置", labelKey: "resource-domain:cron.redeployAction"},{value:"restart",label:"重启当前成功版本", labelKey: "resource-domain:cron.restartAction"},{value:"sync-marketplace",label:"同步市场目录", labelKey: "resource-domain:cron.syncAction"}]},
      {key:"target_kind",label:"目标类型", labelKey: "resource-domain:resource.targetType",type:"select",required:true,resetOnChange:["target_ref"],when:d=>d.scope==="host",options:TARGETS},
      {key:"target_kind",label:"目标类型", labelKey: "resource-domain:resource.targetType",type:"select",required:true,when:d=>d.scope==="build",options:[{value:"pipeline",label:"流水线", labelKey: "common-domain:shell.pipeline"}]},
      {key:"target_kind",label:"目标类型", labelKey: "resource-domain:resource.targetType",type:"select",required:true,when:d=>d.scope==="deploy"&&d.action!=="sync-marketplace",options:[{value:"deployment",label:"应用", labelKey: "common-domain:nav.applications"}]},
      {key:"target_kind",label:"目标类型", labelKey: "resource-domain:resource.targetType",type:"select",required:true,when:d=>d.action==="sync-marketplace",options:[{value:"marketplace-source",label:"市场来源", labelKey: "common-domain:nav.marketplaceSources"}]},
      {key:"target_ref",label:"市场来源", labelKey: "common-domain:nav.marketplaceSources",type:"select",required:true,when:d=>d.target_kind==="marketplace-source",options:namedOptions(workspace,"marketplace-sources")},
      ...([['host','hosts','主机'],['host-group','host-groups','主机组'],['pipeline','pipelines','流水线'],['deployment','deployments','应用']] as const).map(([kind,resource,label])=>({key:"target_ref",label:`目标${label}`,labelKey:`resource-domain:target.${kind}`,type:"select" as const,required:true,when:(d:Partial<CronJob>)=>d.target_kind===kind,options:namedOptions(workspace,resource)})),
      {key:"environment_id",label:"执行环境", labelKey: "resource-domain:pipeline.environment",type:"select",emptyValues:["0",0],emptyLabelKey:"resource-domain:reference.noEnvironment",options:namedOptions(workspace,"pipeline-environments"),when:d=>d.scope==="build",help:"流水线绑定环境时必须选择其中一个；不绑定环境时留空。", helpKey: "resource-domain:cron.environmentHelp"},
      {key:"command",label:"命令", labelKey: "resource-domain:cron.command",type:"code",span:2,required:true,when:d=>d.scope==="host",help:"通过所选主机通道执行；主机组逐台执行，失败即停。不要把密钥写进脚本。", helpKey: "resource-domain:cron.commandHelp"},
      {key:"timeout_seconds",label:"超时（秒）", labelKey: "resource-domain:cron.timeout",type:"number",required:true,help:"1–86400；下游执行等待超时不会强制停止远端，会标记需核实。", helpKey: "resource-domain:cron.timeoutHelp"},
      {key:"max_retries",label:"失败重试次数", labelKey: "resource-domain:cron.retries",type:"number",required:true,help:"默认0；仅对确定失败重试，最多10次。命令必须允许重复执行。", helpKey: "resource-domain:cron.retriesHelp"},
      {key:"retry_delay_seconds",label:"重试间隔（秒）", labelKey: "resource-domain:cron.retryDelay",type:"number",required:true},
      {key:"run_as_name",label:"执行身份", labelKey: "resource-domain:cron.runAs",type:"text",immutable:true,when:d=>!!d.run_as_name,help:"保存后使用保存者身份；每次执行重新核实账号和权限。", helpKey: "resource-domain:cron.runAsHelp"},
      {key:"description",label:"说明", labelKey: "common-domain:common.description",type:"textarea",span:2}
    ]}]}),
    "cron-runs": form<CronRun>({resource:"cron-runs",heading:r=>r.job_name,blank:{},readOnly:()=>true,pollMs:r=>r.state==="queued"||r.state==="running"?1000:0,viewActions:cronLinks(workspace),sections:[{fields:[{key:"job_name",label:"任务", labelKey: "resource-domain:cron.task",type:"text"},{key:"state",label:"状态", labelKey: "resource-domain:status.title",type:"select",options:["queued","running","succeeded","failed","skipped","needs-review"].map(value=>({value,get label() { return cronState(value); }}))},{key:"attempt",label:"尝试次数", labelKey: "resource-domain:cron.attempt",type:"number"},{key:"downstream_id",label:"下游执行 ID", labelKey: "resource-domain:cron.downstreamId",type:"text"},{key:"output",label:"执行输出", labelKey: "resource-domain:cron.output",type:"code",span:2},{key:"config",label:"本次配置快照", labelKey: "resource-domain:cron.snapshot",type:"code",span:2}]}]}),
    settings: form(typedResourceForm("settings")),
    dictionaries: form(typedResourceForm("dictionaries")),
    "config-templates": templateForm("config-templates", false),
    "app-templates": templateForm("app-templates", true),
    "pipeline-templates": templateForm("pipeline-templates", false),
    members: form({
      resource: "members", heading: (r: Member) => r.name, blank: {}, readOnly: r => !!r.owner,
      sections: [{ fields: [
        { key: "name", label: "姓名", labelKey: "common-domain:auth.name", type: "text", immutable: true },
        { key: "email", label: "邮箱", labelKey: "common-domain:auth.email", type: "text", immutable: true },
        { key: "role_name", label: "当前角色", labelKey: "resource-domain:member.currentRole", type: "text", immutable: true },
        { key: "role_id", label: "角色", labelKey: "common-domain:dashboard.roles", type: "select", required: true, when: r => !r.owner, options: async () => {
          if (permissions && !permissions.has("members.update")) return [];
          const result = await api<{items: Role[]}>(`/workspaces/${workspace}/members/role-options`);
          return result.items.map(r => ({ value: r.id, label: r.name }));
        } },
      ] }],
    }),
    roles: form({
      resource: "roles", heading: (r: Role) => r.name, blank: { name: "", permissions: [] }, readOnly: r => !!r.builtin,
      copy: r => ({ name: commonText("resource-domain:resource.copyName", { name: r.name }), permissions: [...r.permissions] }),
      sections: [{ fields: [
        { key: "name", label: "名称", labelKey: "common-domain:common.name", type: "text", required: true },
        { key: "permissions", label: "权限", labelKey: "resource-domain:role.permissions", type: "multiselect", span: 2, help: "勾选操作会自动包含依赖权限；取消依赖权限会同时取消依赖它的操作。", helpKey: "resource-domain:role.permissionsHelp", options: async () => {
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
        { key: "name", label: "名称", labelKey: "common-domain:common.name", type: "text", required: true },
        { key: "mode", label: "部署模式", labelKey: "resource-domain:deployment.mode", type: "select", options: [{ value: "native", label: "Native（systemd）" }, { value: "docker", label: "Docker" }, { value: "compose", label: "Docker Compose" }, { value: "stack", label: "Docker Swarm" }], immutable: true },
        { key: "target_kind", label: "目标类型", labelKey: "resource-domain:resource.targetType", type: "select", options: [{ value: "host", label: "单主机", labelKey: "resource-domain:deployment.singleHost" }, { value: "host-group", label: "主机组（逐台执行）", labelKey: "resource-domain:deployment.hostGroup" }], immutable: true },
        { key: "target_ref", label: "目标主机", labelKey: "resource-domain:deployment.targetHost", type: "select", options: namedOptions(workspace, "hosts"), required: true, immutable: true, when: (d) => d.target_kind === "host" },
        { key: "target_ref", label: "目标主机组", labelKey: "resource-domain:deployment.targetGroup", type: "select", options: namedOptions(workspace, "host-groups"), required: true, immutable: true, when: (d) => d.target_kind === "host-group", help: "部署全部直接成员，按主机 ID 排序；失败停止，已成功主机不自动回滚。", helpKey: "resource-domain:deployment.groupHelp" },
        { key: "workdir", label: "部署工作目录", labelKey: "resource-domain:deployment.workdir", type: "text", required: true, immutable: true, help: "目标 Linux 主机上的可写绝对路径；自动按工作空间和应用隔离子目录。容器模式需要 Docker，Compose 另需 Compose v2+；Native 需要 systemd 及管理服务的权限。所有模式需要 flock、sha256sum。", helpKey: "resource-domain:deployment.workdirHelp" },
        { key: "manifest", label: "应用配置", labelKey: "resource-domain:deployment.config", type: "manifest", required: true, span: 2 },
        { key: "description", label: "备注", labelKey: "resource-domain:resource.notes", type: "textarea", span: 2 },
      ] }],
    }),
    "code-insights": form(codeInsightForm(workspace)),
    "code-connections": form<CodeConnection>({
      resource:"code-connections",permissionResource:"repositories",heading:row=>row.name,
      blank:{name:"",provider:"",credential_id:"0",base_url:"",api_url:"",organization:"",description:""},
      sections:[{fields:[{key:"name",label:"连接名称", labelKey: "resource-domain:repository.connectionName",type:"text",required:true},{key:"source",label:"连接平台", labelKey: "resource-domain:repository.connectProvider",type:"repository-source",span:2},{key:"description",label:"备注", labelKey: "resource-domain:resource.notes",type:"textarea",span:2}]}],
    }),
    "pipeline-groups":form<PipelineContext>({resource:"pipeline-groups",heading:row=>row.name,blank:{name:"",description:""},sections:[{fields:[{key:"name",label:"名称", labelKey: "common-domain:common.name",type:"text",required:true},{key:"description",label:"说明", labelKey: "common-domain:common.description",type:"textarea"}]}]}),
    "pipeline-environments":form<PipelineContext>({resource:"pipeline-environments",heading:row=>row.name,blank:{name:"",description:"",target_kind:"host",target_ref:""},
      viewActions:[{label:"管理环境变量", labelKey: "resource-domain:pipeline.manageVariables",permission:"variables.read",to:()=>({group:"build",item:"environment-variables",mode:"list",id:""}),query:row=>({scope_ref:row.id})}],
      sections:[{fields:[{key:"name",label:"名称", labelKey: "common-domain:common.name",type:"text",required:true},{key:"description",label:"说明", labelKey: "common-domain:common.description",type:"textarea"},
        {key:"target_kind",label:"默认执行器类型", labelKey: "resource-domain:pipeline.defaultExecutor",type:"select",options:TARGETS,resetOnChange:["target_ref"]},
        {key:"target_ref",label:"默认主机", labelKey: "resource-domain:pipeline.defaultHost",type:"select",required:true,options:namedOptions(workspace,"hosts"),when:row=>row.target_kind==="host"},
        {key:"target_ref",label:"默认主机组", labelKey: "resource-domain:pipeline.defaultGroup",type:"select",required:true,options:namedOptions(workspace,"host-groups"),when:row=>row.target_kind==="host-group"},
        {key:"requires_approval",label:"需要审批", labelKey: "resource-domain:pipeline.approval",type:"switch",help:"Actions 任务声明 environment 为此环境名称时，开始执行前需要有执行权限的成员批准（相当于 GitHub 环境的必需审阅者）。", helpKey: "resource-domain:pipeline.approvalHelp"}
      ]}]}),
    pipelines: form({
      ...(permissions?.has("pipeline-templates.read") ? { generator: templateGenerator(workspace, "pipeline-templates", "definition") } : {}),
      resource: "pipelines", width: "full", heading: (r: Pipeline) => r.name,
      viewActions:[pipelineRunAction(workspace)],
      blank: { group_id:"",application_id:"",tag_ids:[],environment_ids:[],name: "", repository_id: "", target_kind: "host", target_ref: "", description: "",
        definition_format: "github-actions", executor_bindings: [], checkout_bindings: [], graph_layout: {}, definition: DEFAULT_ACTIONS },
      sections: [{ fields: [
        { key: "name", label: "名称", labelKey: "common-domain:common.name", type: "text", required: true },
        {key:"group_id",when:()=>!permissions||permissions.has("pipeline-groups.read"),label:"分组", labelKey: "common-domain:table.groups",type:"select",emptyValues:["0",0],emptyLabelKey:"resource-domain:reference.ungrouped",options:namedOptions(workspace,"pipeline-groups")},
        {key:"tag_ids",when:()=>!permissions||permissions.has("tags.read"),label:"标签", labelKey: "common-domain:nav.tags",type:"multiselect",options:tagOptions(workspace),span:2,createOption:{label:"新建标签", labelKey: "resource-domain:tag.create",permission:"tags.create",fields:[{key:"name",label:"名称", labelKey: "common-domain:common.name",type:"text",required:true}],create:async values=>{const row=await createResource<{id:string}>(workspace,"tags",values);return {value:row.id,label:String(values.name)};}}},
        { key: "definition", label: "Actions 工作流", labelKey: "resource-domain:pipeline.workflow", type: "actions-editor", required: true, span: 2,
          options: async () => (await Promise.all(["hosts", "host-groups"].filter(resource=>!permissions||permissions.has(`${resource}.read`)).map(async (resource) => (await namedOptions(workspace, resource)()).map(option => ({ value: `${resource === "hosts" ? "host" : "host-group"}:${option.value}`, get label() { return `${resource === "hosts" ? commonText("common-domain:nav.hosts") : commonText("common-domain:nav.hostGroups")} · ${option.label}`; } }))))).flat() },
      ] }],
    }),
    "host-groups": form({
      resource: "host-groups",
      heading: (row: HostGroup) => row.name,
      blank: { name: "", description: "", parent_id: "" },
      sections: [
        {
          fields: [
            { key: "name", label: "名称", labelKey: "common-domain:common.name", type: "text", required: true, placeholder: "例如 生产-华东", placeholderKey: "resource-domain:hostGroup.example" },
            { key: "parent_id", label: "上级主机组", labelKey: "resource-domain:hostGroup.parent", type: "select", emptyValues: ["0", 0], emptyLabelKey: "resource-domain:reference.topLevel", options: groupOptions(workspace), help: "留空表示顶级", helpKey: "resource-domain:hostGroup.parentHelp" },
            { key: "description", label: "说明", labelKey: "common-domain:common.description", type: "textarea", span: 2 },
          ],
        },
      ],
    }),

    hosts: form({
      resource: "hosts",
      heading: (row: Host) => row.name,
      blank: { name: "", connection: "runner", runner_auto_update: "inherit", address: "", port: 22, username: "", group_id: "", tag_ids: [], credential_id: "", jump_host_id: "", description: "" },
      sections: [
        {
          title: "基本信息", titleKey: "common-domain:common.basic",
          fields: [
            { key: "name", label: "名称", labelKey: "common-domain:common.name", type: "text", required: true, placeholder: "例如 web-01", placeholderKey: "resource-domain:host.nameExample" },
            { key: "runner_auto_update", label: "Runner 自动升级", labelKey: "resource-domain:runner.autoUpdate", type: "select", options: [{ value: "inherit", label: "跟随平台默认", labelKey: "resource-domain:runner.followDefault" }, { value: "on", label: "启用", labelKey: "resource-domain:status.enabled" }, { value: "off", label: "关闭", labelKey: "resource-domain:status.off" }], when: (row: Partial<Host>) => row.connection === "runner", help: "等待完整任务结束后升级到当前服务版本，期间暂停新任务。", helpKey: "resource-domain:runner.updateWaitHelp" },
            { key: "group_id", label: "主机组", labelKey: "common-domain:nav.hostGroups", type: "select", emptyValues: ["0", 0], emptyLabelKey: "resource-domain:reference.ungrouped", options: groupOptions(workspace) },
            { key: "tag_ids", label: "标签", labelKey: "common-domain:nav.tags", type: "multiselect", options: tagOptions(workspace), span: 2, help: "可选择多个标签；取消全部选择会清空关联。", helpKey: "resource-domain:host.tagsHelp" },
            {
              key: "connection",
              label: "接入方式", labelKey: "common-domain:form.connection",
              type: "select",
              required: true,
              options: CONNECTIONS,
              immutable: true,
              help: "创建后不可更改：改了会让已注册的代理密钥或 SSH 凭据失效。", helpKey: "resource-domain:host.connectionImmutable",
            },
          ],
        },
        {
          title: "SSH 连接", titleKey: "resource-domain:host.sshSection",
          fields: [
            {
              key: "address",
              label: "主机地址", labelKey: "resource-domain:host.hostAddress",
              type: "text",
              required: true,
              placeholder: "10.0.0.11 或 host.example.com", placeholderKey: "resource-domain:host.addressExample",
              when: (draft: Partial<Host>) => draft.connection === "ssh",
            },
            { key: "port", label: "端口", labelKey: "resource-domain:host.port", type: "number", required: true, when: (draft: Partial<Host>) => draft.connection === "ssh" },
            { key: "username", label: "登录账号", labelKey: "resource-domain:host.loginAccount", type: "text", required: true, placeholder: "root", when: (draft: Partial<Host>) => draft.connection === "ssh" },
            { key: "jump_host_id", label: "跳板主机", labelKey: "resource-domain:host.jumpHost", type: "select", emptyValues: ["0", 0], emptyLabelKey: "resource-domain:reference.noJumpHost", options: sshJumpOptions(workspace),
              help: "可选。先添加一台可直连的 SSH 主机，多个目标主机可共用它。目标主机仍使用自己的登录凭据。", helpKey: "resource-domain:host.jumpHelp",
              when: (draft: Partial<Host>) => draft.connection === "ssh" },
            {
              key: "credential_id",
              label: "登录凭据", labelKey: "resource-domain:host.loginCredential",
              type: "select",
              required: true,
              options: sshCredentialOptions(workspace),
              span: 2,
              help: "密码与私钥由服务端加密保存，连接时才解密。", helpKey: "resource-domain:host.credentialHelp",
              createOption: { label: "添加登录凭据", labelKey: "resource-domain:credential.addLogin", permission: "credentials.create", fields: [
                { key: "name", label: "凭据名称", labelKey: "resource-domain:credential.name", type: "text", required: true },
                { key: "kind", label: "类型", labelKey: "resource-domain:resource.type", type: "select", required: true, options: CREDENTIAL_KINDS.filter(option => option.value === "password" || option.value === "ssh-key") },
                { key: "secret", label: "密码", labelKey: "common-domain:auth.password", type: "password", required: true, span: 2, when: values => values.kind === "password" },
                { key: "secret", label: "SSH 私钥", labelKey: "resource-domain:credential.sshKey", type: "secret", required: true, span: 2, when: values => values.kind === "ssh-key" },
              ], create: async values => { const row = await createResource<{ id: string }>(workspace, "credentials", { ...values, scope: "host", username: "", description: "" }); return { value: row.id, label: String(values.name) }; } },
              when: (draft: Partial<Host>) => draft.connection === "ssh",
            },
          ],
        },
        {
          fields: [{ key: "description", label: "备注", labelKey: "resource-domain:resource.notes", type: "textarea", span: 2 }],
        },
      ],
    }),

    variables: form({
      resource: "variables", heading: (row: Variable) => row.name,
      blank: { name: "", scope: "global", scope_ref: "0", kind: "secret", value: "", description: "" },
      sections: [{ fields: [
        { key: "name", label: "名称", labelKey: "common-domain:common.name", type: "text", required: true, immutable: true, help: "96个以内字母、数字、下划线或连字符；引用写作 variable/名称。", helpKey: "resource-domain:variable.nameHelp" },
        { key: "kind", label: "类型", labelKey: "resource-domain:resource.type", type: "select", options: VARIABLE_KINDS, immutable: true },
        { key: "scope", label: "作用域", labelKey: "resource-domain:variable.scope", type: "select", options: VARIABLE_SCOPES, immutable: true, resetOnChange: ["scope_ref"] },
        {key:"scope_ref",label:"执行环境", labelKey: "resource-domain:pipeline.environment",type:"select",options:namedOptions(workspace,"pipeline-environments"),required:true,immutable:true,when:row=>row.scope==="environment",help:"只用于选中该环境的流水线执行；同名值覆盖全局、构建和流水线变量。", helpKey: "resource-domain:variable.environmentHelp"},
        { key: "scope_ref", label: "对象 ID", labelKey: "resource-domain:variable.objectId", type: "text", immutable: true, when: (row) => row.scope !== "global" && row.scope !== "environment", help: "0 或留空为模块默认；部署范围填应用 ID，构建范围填流水线 ID。具体对象覆盖模块，模块覆盖全局。", helpKey: "resource-domain:variable.objectHelp" },
        { key: "value", label: "变量值", labelKey: "resource-domain:variable.plainValue", type: "textarea", span: 2, when: (row) => row.kind === "plain", help: "空白和换行会原样保存；留空表示空字符串。", helpKey: "resource-domain:variable.whitespaceHelp" },
        { key: "value", label: "密钥值", labelKey: "resource-domain:variable.secretValue", type: "secret", stored: "has_value", replaceOnEdit: true, span: 2, when: (row) => row.kind === "secret",
          help: "加密保存。编辑时勾选替换后留空，表示设置为空字符串；查看明文会记录审计。", helpKey: "resource-domain:variable.secretEditHelp",
          reveal: { permission: "variables.reveal", action: async (row) => (await runItemAction<{value: string}>(workspace, "variables", String(row.id), "reveal", {})).value } },
        { key: "description", label: "备注", labelKey: "resource-domain:resource.notes", type: "textarea", span: 2 },
      ] }],
    }),

    credentials: form({
      resource: "credentials",
      heading: (r: Credential) => r.name,
      blank: { name: "", scope: "build", kind: "password", username: "", description: "", secret: "" },
      sections: [
        {
          fields: [
            { key: "name", label: "名称", labelKey: "common-domain:common.name", type: "text", required: true, placeholder: "输入便于识别的凭据名称", placeholderKey: "resource-domain:credential.namePlaceholder" },
            {
              key: "kind",
              label: "类型", labelKey: "resource-domain:resource.type",
              type: "select",
              required: true,
              options: CREDENTIAL_KINDS,
              immutable: true,
              help: "创建后不可更改：改类型等于换一条凭据。", helpKey: "resource-domain:credential.typeImmutable",
            },
            { key: "username", label: "账号", labelKey: "resource-domain:credential.account", type: "text", placeholder: "填写目标服务的登录账号", placeholderKey: "resource-domain:credential.accountPlaceholder" },
            {
              key: "secret",
              label: "内容", labelKey: "common-domain:common.content",
              type: "secret",
              span: 2,
              help: "密码、私钥全文或令牌。保存后服务端加密存放，任何接口都不会再返回它；编辑时留空表示不修改。", helpKey: "resource-domain:credential.secretHelp",
            },
            { key: "description", label: "备注", labelKey: "resource-domain:resource.notes", type: "textarea", span: 2 },
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
            { key: "name", label: "名称", labelKey: "common-domain:common.name", type: "text", required: true },
            { key: "color", label: "颜色", labelKey: "resource-domain:tag.color", type: "text", placeholder: "#4f9cf9" },
          ],
        },
      ],
    }),
  };
}

interface Role { id: string; name: string; builtin: boolean; permissions: string[]; version: number }
interface Member { id: string; name: string; email: string; role_id: string; role_name: string; owner: boolean; active: boolean; state: string; version: number }
