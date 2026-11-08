import { LogIn } from "lucide-react";
import { useEffect,type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { type Identity } from "../shared/api/api";
import { commonText } from "../shared/i18n/i18n";
import type { FormConfig,ListConfig } from "../shared/resources/types";
import { Button } from "../shared/ui/button";
import { nav } from "./nav";
import { ResourceForm } from "./resources/ResourceForm";
import { ResourceTable } from "./resources/ResourceTable";
import { PlatformServerNodes } from "./PlatformServerNodes";
import { PlatformActivity } from "./PlatformActivity";
import { PlatformMonitor, monitorSections } from "./PlatformMonitor";
import {FOOTER_UPDATED, PlatformFooterEditor} from "./PlatformFooter";
import {PlatformMailTest} from "./PlatformMailTest";
import {PlatformMailTemplates} from "./PlatformMailTemplates";
import {AutoField} from "../shared/forms/auto-form";
import { href,navigate,useRoute } from "./router";

type Row = Record<string, unknown> & { id: string; name: string; email: string; slug: string; active: boolean; is_superadmin: boolean; relation?: string; my_role?: string; last_login_at?:number|null;last_login_ip?:string|null;created_at?:number };
/** Whether the user created the workspace, joined it, or only reaches it as a platform administrator. */
export const myRole = (relation: string | undefined, role: string | undefined) =>
  relation === "owner" ? commonText("resource-domain:platform.ownerSelf") : relation === "member" ? commonText("resource-domain:platform.joined", { role }) : commonText("resource-domain:platform.notMember");
const labels: Record<string, string> = { workspaces: "resource-domain:platform.workspaces", users: "resource-domain:platform.users", monitor: "resource-domain:monitor.title", settings: "resource-domain:platform.settings" };
const permissions = new Set(["users.update", "workspaces.create", "workspaces.update", "settings.update"]);
const edit = { label: "编辑", labelKey: "common-domain:common.edit", to: (r: Row) => ({ id: r.id, mode: "edit" as const }) };
const workspaceEntry = (id: string) => ({ workspace: id, group: nav[0]!.key, item: nav[0]!.items[0]!.key, mode: "list" as const });
function WorkspaceOverview({draft}:{draft:Partial<Row>}) {
  const values: [string, ReactNode][] = [
    ["workspaceId",draft.id], ["workspaceOwner",String(draft.owner_name??"—")],
    ["myRole",myRole(draft.relation,draft.my_role)],
    ["workspaceCreated",draft.created_at?new Date(draft.created_at).toLocaleString():"—"],
    ["workspaceUpdated",draft.version?new Date(Number(draft.version)).toLocaleString():"—"],
  ];
  const counts=draft.resource_counts as Record<string,number>|undefined;
  return <div className="min-w-0 space-y-6"><dl className="grid gap-4 sm:grid-cols-2">{values.map(([key,value])=><div key={key} className="min-w-0"><dt className="text-sm text-muted-foreground">{commonText(`resource-domain:platform.${key}`)}</dt><dd className="mt-1 break-words text-sm">{value}</dd></div>)}</dl>
    {counts&&<div><h3 className="mb-3 font-medium">{commonText("resource-domain:platform.workspaceResources")}</h3><dl className="grid grid-cols-2 gap-4 sm:grid-cols-3">{["members","hosts","repository_connections","credentials","pipelines","workflows","assets"].filter(key=>key in counts).map(key=><div key={key}><dt className="text-sm text-muted-foreground">{commonText(`resource-domain:platform.workspaceCount.${key}`)}</dt><dd className="mt-1 text-lg tabular-nums">{counts[key]!.toLocaleString()}</dd></div>)}</dl></div>}
  </div>;
}
const lists: Record<string, ListConfig<Row>> = {
  users: {
    resource: "users", endpoint: "/platform/users", rowKey: r => r.id,
    filters: [
      { key: "q", label: "姓名或邮箱", labelKey: "resource-domain:platform.nameEmail", type: "text" },
      { key: "active", label: "状态", labelKey: "resource-domain:status.title", type: "select", options: [
        { value: "1", label: "启用", labelKey: "resource-domain:status.enabled" },
        { value: "0", label: "停用", labelKey: "resource-domain:status.disabled" },
      ] },
      { key: "is_superadmin", label: "平台角色", labelKey: "resource-domain:platform.role", type: "select", options: [
        { value: "1", label: "平台超级管理员", labelKey: "resource-domain:platform.superadmin" },
        { value: "0", label: "普通用户", labelKey: "resource-domain:platform.regularUser" },
      ] },
    ],
    columns: [{ key: "name", label: "姓名", labelKey: "common-domain:auth.name", sortable: true }, { key: "email", label: "邮箱", labelKey: "common-domain:auth.email", sortable: true },
      { key: "active", label: "状态", labelKey: "resource-domain:status.title", render: r => r.active ? commonText("resource-domain:status.enabled") : commonText("resource-domain:status.disabled") },
      { key: "is_superadmin", label: "平台角色", labelKey: "resource-domain:platform.role", render: r => r.is_superadmin ? commonText("resource-domain:platform.superadmin") : commonText("resource-domain:platform.regularUser") },
      {key:"last_login_ip",label:"最近登录 IP",labelKey:"resource-domain:platform.lastLoginIp",render:r=>r.last_login_ip || "—"},
      {key:"last_login_at",label:"最后登录时间",labelKey:"resource-domain:platform.lastLoginAt",sortable:true,render:r=>r.last_login_at?new Date(r.last_login_at).toLocaleString():"—"},
      {key:"created_at",label:"注册时间",labelKey:"resource-domain:platform.registeredAt",sortable:true,render:r=>r.created_at?new Date(r.created_at).toLocaleString():"—"}],
    toolbar: { create: false, import: false, export: false },
    actions: [edit], empty: "暂无平台用户", emptyKey: "resource-domain:platform.noUsers",
  },
  workspaces: {
    resource: "workspaces", endpoint: "/platform/workspaces", rowKey: r => r.id,
    filters: [{ key: "q", label: "名称或标识", labelKey: "resource-domain:platform.nameSlug", type: "text" }],
    columns: [{ key: "name", label: "名称", labelKey: "common-domain:common.name", sortable: true }, { key: "slug", label: "标识", labelKey: "common-domain:auth.slug", sortable: true },
      { key: "my_role", label: "我的角色", labelKey: "resource-domain:platform.myRole", render: r => myRole(r.relation, r.my_role) }],
    actions: [edit, { label: "进入工作空间", labelKey: "resource-domain:platform.enter", icon: <LogIn className="size-4" />, to: r => workspaceEntry(r.id) }],
    toolbar: { create: { label: "创建工作空间", labelKey: "common-domain:auth.createWorkspace" }, import: false, export: false }, empty: "还没有工作空间，创建后即可配置主机和部署。", emptyKey: "resource-domain:platform.noWorkspaces",
  },
};
const forms: Record<string, FormConfig<Row>> = {
  users: { resource: "users", endpoint: "/platform/users", heading: r => r.name, blank: {}, sections: [{ fields: [
    { key: "email", label: "邮箱", labelKey: "common-domain:auth.email", type: "text", immutable: true },
    { key: "name", label: "姓名", labelKey: "common-domain:auth.name", type: "text", immutable: true },
    { key: "active", label: "启用账号", labelKey: "resource-domain:platform.enableAccount", type: "switch", help: "停用后该账号的所有会话立即失效。", helpKey: "resource-domain:platform.disableHelp" },
    { key: "is_superadmin", label: "平台超级管理员", labelKey: "resource-domain:platform.platformAdmin", type: "switch", help: "可管理平台及全部工作空间。变更后需要重新登录；必须保留一名启用的超管。", helpKey: "resource-domain:platform.adminHelp" },
  ] }] },
  workspaces: { resource: "workspaces", endpoint: "/platform/workspaces", heading: r => r.name, blank: { name: "", slug: "" }, sections: [{ fields: [
    { key: "name", label: "名称", labelKey: "common-domain:common.name", type: "text", required: true },
    { key: "slug", label: "标识", labelKey: "common-domain:auth.slug", type: "text", required: true, immutable: true, help: "2–40 位小写字母、数字、中划线；创建后不可修改。", helpKey: "resource-domain:platform.slugHelp" },
  ] }, {title:"空间信息",titleKey:"resource-domain:platform.workspaceInfo",fields:[{key:"workspace_overview",label:"空间信息",labelKey:"resource-domain:platform.workspaceInfo",type:"custom",span:2,when:d=>!!d.id,render:({draft})=><WorkspaceOverview draft={draft}/>}]}] },
  settings: { resource: "settings", endpoint: "/platform/settings", heading: () => commonText("resource-domain:platform.settings"), blank: {}, sections: [
{ title: "账号注册", titleKey: "resource-domain:platform.registration", group: "账号注册", groupKey: "resource-domain:platform.registration", tab:"registration", tabKey:"resource-domain:platform.registration", fields: [
    { key: "registration_enabled", label: "允许注册新账号", labelKey: "resource-domain:platform.allowRegistration", type: "switch", help: "关闭后已有账号仍可登录，公开注册入口将关闭。", helpKey: "resource-domain:platform.registrationHelp" },
  ] },
{title:"邮件模板",titleKey:"resource-domain:platform.mailTemplates",group:"账号注册",groupKey:"resource-domain:platform.registration",tab:"templates",tabKey:"resource-domain:platform.mailTemplates",fields:[
    {key:"mail_templates",label:"邮件模板",labelKey:"resource-domain:platform.mailTemplates",type:"custom",span:2,render:({draft,readOnly,onPatch})=><PlatformMailTemplates draft={draft} readOnly={readOnly} onPatch={onPatch}/>},
  ]},
{ title: "登录防护", titleKey:"resource-domain:loginGuard.settings",group:"登录防护",groupKey:"resource-domain:loginGuard.settings",fields:[
    {key:"login_guard_mode",label:"验证码方式",labelKey:"resource-domain:loginGuard.mode",type:"select",options:[{value:"local",label:"本地随机验证码",labelKey:"resource-domain:loginGuard.local"},{value:"turnstile",label:"Cloudflare Turnstile"}],helpKey:"resource-domain:loginGuard.help"},
    {key:"login_turnstile_site_key",label:"Site Key",type:"text",when:d=>d.login_guard_mode==="turnstile"},
    {key:"login_turnstile_secret_key",label:"Secret Key",type:"secret",stored:"login_turnstile_has_secret",replaceOnEdit:true,when:d=>d.login_guard_mode==="turnstile"},
    {key:"login_turnstile_hostname",label:"Turnstile 站点域名",labelKey:"resource-domain:loginGuard.hostname",type:"text",when:d=>d.login_guard_mode==="turnstile",helpKey:"resource-domain:loginGuard.hostnameHelp"},
  ] },
{title:"监控采集",titleKey:"resource-domain:platform.monitorCollection",group:"监控采集",groupKey:"resource-domain:platform.monitorCollection",fields:[
    {key:"monitor_retention_days",label:"明细保留天数",labelKey:"resource-domain:platform.monitorRetentionDays",type:"number",min:1,max:365,helpKey:"resource-domain:platform.monitorRetentionHelp"},
    {key:"monitor_max_rows",label:"每节点明细条数上限",labelKey:"resource-domain:platform.monitorMaxRows",type:"number",min:1000,max:1000000,helpKey:"resource-domain:platform.monitorRowsHelp"},
  ]},
{ title: "HostAgent", titleKey: "resource-domain:runner.settings", group: "HostAgent", groupKey: "resource-domain:runner.settings", fields: [
    { key: "runner_auto_update", label: "Runner 默认自动升级", labelKey: "resource-domain:runner.defaultAutoUpdate", type: "switch", help: "空闲时升级到当前服务版本；主机可单独覆盖此默认值。", helpKey: "resource-domain:runner.defaultAutoHelp" },
  ] },
{title:"邮箱配置",titleKey:"resource-domain:platform.mailSettings",group:"邮箱配置",groupKey:"resource-domain:platform.mailSettings",tab:"connection",tabKey:"resource-domain:platform.mailConnection",fields:[
    {key:"mail_enabled",label:"启用邮箱",labelKey:"resource-domain:platform.mailEnabled",type:"switch",helpKey:"resource-domain:platform.mailHelp"},
    {key:"mail_security",label:"传输加密",labelKey:"resource-domain:platform.mailSecurity",type:"select",options:[{value:"starttls",label:"STARTTLS (587)"},{value:"tls",label:"TLS (465)"},{value:"none",label:"不加密",labelKey:"resource-domain:platform.mailPlain"}]},
    {key:"mail_host",label:"SMTP 服务器",labelKey:"resource-domain:platform.mailHost",type:"text",placeholder:"smtp.example.com"},
    {key:"mail_port",label:"SMTP 端口",labelKey:"resource-domain:platform.mailPort",type:"number",min:1,max:65535},
    {key:"mail_username",label:"SMTP 用户名",labelKey:"resource-domain:platform.mailUsername",type:"text"},
    {key:"mail_password",label:"SMTP 密码 / 授权码",labelKey:"resource-domain:platform.mailPassword",type:"secret",stored:"mail_has_password",replaceOnEdit:true,hideSecretStatus:true},
    {key:"mail_from",label:"发件人邮箱",labelKey:"resource-domain:platform.mailFrom",type:"text",placeholder:"noreply@example.com"},
    {key:"mail_verified_at",label:"发送验证时间",labelKey:"resource-domain:platform.mailVerified",type:"custom",render:({draft})=><AutoField label={commonText("resource-domain:platform.mailVerified")}><p role="status" className="text-sm text-muted-foreground">{draft.mail_verified_at?new Date(Number(draft.mail_verified_at)).toLocaleString():commonText("resource-domain:platform.mailUnverified")}</p></AutoField>},
    {key:"mail_test",label:"发送验证",labelKey:"resource-domain:platform.mailTest",type:"custom",span:2,render:({draft,readOnly,onPatch})=><PlatformMailTest draft={draft} readOnly={readOnly} onVerified={onPatch}/>},
  ]},
{ title: "文件存储", titleKey: "resource-domain:platform.fileStorage", group: "文件存储", groupKey: "resource-domain:platform.fileStorage", fields: [
    {key:"storage_kind",label:"文件存储",labelKey:"resource-domain:platform.fileStorage",type:"select",options:[{value:"local",label:"本地文件（仅单机）",labelKey:"resource-domain:platform.storageLocal"},{value:"s3",label:"S3（单机或集群）",labelKey:"resource-domain:platform.storageS3"}],helpKey:"resource-domain:platform.storageHelp",disabled:d=>d.storage_source==="environment"},
    {key:"storage_source",label:"配置来源",labelKey:"resource-domain:platform.configSource",type:"text",immutable:true},
    {key:"storage_local_path",label:"本机存储路径",labelKey:"resource-domain:platform.storageLocalPath",type:"custom",span:2,when:d=>d.storage_kind==="local",render:({draft})=><code className="block select-all whitespace-normal break-all rounded border bg-muted/30 p-3 text-sm">{String(draft.storage_local_path??"")}</code>,helpKey:"resource-domain:platform.storageLocalPathHelp"},
    ...([ ["endpoint","S3 Endpoint"], ["bucket","Bucket"], ["region","Region"], ["access_key","Access Key ID"] ] as const).map(([key,label])=>({key:`storage_${key}`,label,type:"text" as const,when:(d:Partial<Row>)=>d.storage_kind==="s3",disabled:(d:Partial<Row>)=>d.storage_source==="environment"})),
    {key:"storage_secret_key",label:"Secret Access Key",type:"secret",stored:"storage_has_secret_key",replaceOnEdit:true,when:d=>d.storage_kind==="s3",disabled:d=>d.storage_source==="environment",helpKey:"resource-domain:platform.storageSecretHelp"},
  ] },
{ title: "公共配置", titleKey: "resource-domain:platform.publicConfig", group: "仓库关联", groupKey: "resource-domain:platform.repositoryIntegration", fields: [
    { key: "oauth_public_url", label: "平台公开访问地址", labelKey: "resource-domain:platform.publicUrl", type: "text", placeholder: "https://ops.example.com", span: 2, help: "填写用户浏览器可访问的平台地址。保存后立即生效，无需重启；各工作空间仍分别授权自己的账号。", helpKey: "resource-domain:platform.publicUrlHelp" },
    { key: "oauth_callback_url", label: "当前回调地址", labelKey: "resource-domain:platform.callbackUrl", type: "text", immutable: true, copyable: true, span: 2, help: "将此地址登记到代码平台的 OAuth 应用。修改公开访问地址并保存后，这里会显示新的回调地址。", helpKey: "resource-domain:platform.callbackHelp" },
    { key: "oauth_source", label: "配置来源", labelKey: "resource-domain:platform.configSource", type: "text", immutable: true, span: 2 },
  ] },
...([ ["github", "GitHub"], ["gitlab", "GitLab"], ["gitee", "Gitee"], ["gitea", "Gitea"] ] as const).map(([key, label]) => ({ title: `${label} OAuth 应用`, titleKey: `resource-domain:platform.${key}App`, group: "仓库关联", groupKey: "resource-domain:platform.repositoryIntegration", tab: label, fields: [
    { key: `oauth_${key}_enabled`, label: `启用 ${label} 网页授权`, labelKey: `resource-domain:platform.${key}Enable`, type: "switch" as const, span: 2 as const, help: "关闭后不再使用该平台的网页授权凭据；已保存的应用配置保留。Codeup 使用个人访问令牌，无需配置 OAuth 应用。", helpKey: "resource-domain:platform.oauthDisableHelp" },
    { key: `oauth_${key}_client_id`, label: `${label} ${key === "gitlab" ? "Application ID" : "Client ID"}`, labelKey: `resource-domain:platform.${key}Id`, type: "text" as const },
    { key: `oauth_${key}_client_secret`, label: `${label} ${key === "gitlab" ? "Secret" : "Client Secret"}`, labelKey: `resource-domain:platform.${key}Secret`, type: "secret" as const, stored: `oauth_${key}_has_client_secret`, replaceOnEdit: true, placeholder: "留空保留已有应用密钥", placeholderKey: "resource-domain:platform.keepSecret", help: "加密保存，不回显；留空保留已有密钥。修改应用 ID 或接入点后，原账号需重新授权。", helpKey: "resource-domain:platform.secretHelp" },
    { key: `oauth_${key}_base_url`, label: `${label} 网站地址`, labelKey: `resource-domain:platform.${key}Website`, type: "text" as const, placeholder: key === "gitea" ? "https://git.example.com" : `https://${key}.com`, help: "留空使用官方地址；自建 GitLab／GitHub Enterprise 可填写自己的地址。", helpKey: "resource-domain:platform.baseUrlHelp" },
    { key: `oauth_${key}_api_url`, label: `${label} API 地址`, labelKey: `resource-domain:platform.${key}Api`, type: "text" as const, help: "通常留空，由网站地址自动推导；仅在使用自定义 API 接入点时填写。", helpKey: "resource-domain:platform.apiUrlHelp" },
  ] })),
{title:"页脚设置",titleKey:"resource-domain:platform.footerSettings",group:"页脚设置",groupKey:"resource-domain:platform.footerSettings",fields:[
    {key:"footer_html",label:"页脚内容",labelKey:"resource-domain:platform.footerContent",type:"custom",span:2,render:({draft,readOnly,onPatch})=><PlatformFooterEditor value={String(draft.footer_html??"")} readOnly={readOnly} onChange={value=>onPatch({footer_html:value})}/>},
  ]},
{title:"页面脚本",titleKey:"resource-domain:platform.scripts",group:"页面脚本",groupKey:"resource-domain:platform.scripts",fields:[
    {key:"analytics_enabled",label:"启用页面脚本",labelKey:"resource-domain:platform.scriptsEnabled",type:"switch",helpKey:"resource-domain:platform.scriptsHelp"},
    {key:"analytics_ga_id",label:"Google Analytics 测量 ID（可选）",labelKey:"resource-domain:platform.gaId",type:"text",placeholder:"G-XXXXXXXXXX",helpKey:"resource-domain:platform.gaHelp"},
    {key:"analytics_js",label:"自定义 JavaScript",labelKey:"resource-domain:platform.customJs",type:"textarea",span:2,helpKey:"resource-domain:platform.jsHelp"},
    {key:"analytics_origins",label:"允许访问的域名",labelKey:"resource-domain:platform.scriptOrigins",type:"textarea",span:2,placeholder:"https://analytics.example.com",helpKey:"resource-domain:platform.originsHelp"},
  ]}
] },
};

export function Platform({ identity, reload, headerActions }: { identity: Identity; reload: () => void; onSignedOut: () => void; headerActions?: ReactNode }) {
  useTranslation("resource-domain");
  const route = useRoute();
  const legacyServerNodes = route.item === "settings" && new URLSearchParams(location.search).get("section") === "server-nodes";
  const serverNodes = route.item === "monitor" && (!route.id || route.id === "server-nodes");
  const item = route.workspace === "_platform" ? route.item : "workspaces";
  useEffect(() => {
    if (route.workspace !== "_platform") navigate({ workspace: "_platform", group: "platform", item: "workspaces", mode: "list" }, {}, { replace: true });
    else if (item === "server-nodes" || legacyServerNodes) navigate({ workspace: "_platform", item: "monitor", id: "server-nodes", mode: "view" }, {}, { replace: true });
    else if (item === "monitor" && !route.id) navigate({ ...route, id: "server-nodes", mode: "view" }, {}, { replace: true });
    else if (item === "settings" && !route.id) navigate({ ...route, id: "1", mode: "edit" }, {}, { replace: true });
  }, [route.workspace, route.id, item, legacyServerNodes]);
  if (!identity.user.is_superadmin) return <div className="p-6 space-y-4"><div className="flex justify-end">{headerActions}</div><p>{commonText("resource-domain:platform.forbidden")}</p><Button onClick={() => navigate({ workspace: identity.workspaces[0]?.id ?? "", mode: "list" }, {})}>{commonText("common-domain:shell.backWorkspace")}</Button></div>;
  if (item === "server-nodes" || legacyServerNodes) return null;
  const form = forms[item]!;
  return <div className="flex h-full flex-col">
    <header className="flex flex-wrap items-center gap-4 border-b px-5 py-3">
      <h1 className="font-semibold">{commonText("resource-domain:platform.management")}</h1>
      <nav aria-label={commonText("resource-domain:platform.management")} className="flex flex-wrap gap-2">
        {Object.entries(labels).map(([key, label]) => <a key={key} href={href({ workspace: "_platform", item: key, mode: key === "settings" ? "edit" : key === "monitor" ? "view" : "list", ...(key === "settings" ? { id: "1" } : key === "monitor" ? { id: "server-nodes" } : {}) }, {})}
          aria-current={key === item ? "page" : undefined} className={`rounded px-3 py-2 text-sm ${key === item ? "bg-accent" : "text-muted-foreground"}`}
          onClick={event => { if (event.button || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return; event.preventDefault(); navigate({ workspace: "_platform", item: key, mode: key === "settings" ? "edit" : key === "monitor" ? "view" : "list", ...(key === "settings" ? { id: "1" } : key === "monitor" ? { id: "server-nodes" } : {}) }, {}); }}>{commonText(label)}</a>)}
      </nav>
      <span className="flex-1" />
      {headerActions}
    </header>
    <div className="flex shrink-0 items-center justify-between gap-3 border-b px-5 py-3">
      <nav aria-label={commonText("common-domain:shell.breadcrumbs")}><ol className="flex flex-wrap items-center gap-2 text-sm text-muted-foreground">
        <li>{commonText("resource-domain:platform.management")}</li><li aria-hidden="true">/</li>
        <li><a className="hover:text-foreground" aria-current={!route.id?"page":undefined} href={href({workspace:"_platform",item,mode:item==="settings"?"edit":item==="monitor"?"view":"list",...(item==="settings"?{id:"1"}:item==="monitor"?{id:"server-nodes"}:{})},{})} onClick={event=>{if(event.button||event.metaKey||event.ctrlKey||event.shiftKey||event.altKey)return;event.preventDefault();navigate({workspace:"_platform",item,mode:item==="settings"?"edit":item==="monitor"?"view":"list",...(item==="settings"?{id:"1"}:item==="monitor"?{id:"server-nodes"}:{})},{});}}>{commonText(labels[item]!)}</a></li>
        {(item==="monitor"||serverNodes||(route.id&&item!=="settings"))&&<><li aria-hidden="true">/</li><li aria-current="page" className="text-foreground">{serverNodes?commonText("resource-domain:platform.serverNodes"):item==="monitor"?commonText(`resource-domain:monitor.${route.id??"database"}`):commonText(`common-domain:common.${route.mode==="edit"?"edit":"details"}`)}</li></>}
      </ol></nav>
      {item === "workspaces" && route.id && <Button asChild variant="outline"><a href={href(workspaceEntry(route.id), {})} onClick={event => { if (event.button || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return; event.preventDefault(); navigate(workspaceEntry(route.id!), {}); }}><LogIn className="size-4" />{commonText("resource-domain:platform.enter")}</a></Button>}
    </div>
    <div className="flex min-h-0 flex-1 flex-col sm:flex-row">
      {item === "monitor" && <nav aria-label={commonText("resource-domain:monitor.title")} className="flex shrink-0 gap-1 overflow-auto border-b p-3 sm:w-48 sm:flex-col sm:border-r sm:border-b-0">
        {["server-nodes", ...monitorSections, "activity"].map(section => <a key={section} className={`rounded px-3 py-2 text-sm ${(route.id ?? "server-nodes") === section ? "bg-accent font-medium" : "text-muted-foreground hover:bg-accent/50"}`} aria-current={(route.id ?? "server-nodes") === section ? "page" : undefined} href={href({workspace:"_platform",item:"monitor",id:section,mode:"view"},{})} onClick={event=>{if(event.button||event.metaKey||event.ctrlKey||event.shiftKey||event.altKey)return;event.preventDefault();navigate({workspace:"_platform",item:"monitor",id:section,mode:"view"},{});}}>{commonText(section === "server-nodes" ? "resource-domain:platform.serverNodes" : `resource-domain:monitor.${section}`)}</a>)}
      </nav>}
    <main className="flex min-h-0 min-w-0 flex-1 flex-col p-5">
      {item === "monitor" ? serverNodes ? <PlatformServerNodes /> : route.id === "activity" ? <PlatformActivity /> : <PlatformMonitor key={route.id ?? "database"} section={route.id ?? "database"} /> : route.mode === "list" && item !== "settings" ? <ResourceTable key={item} config={lists[item]!} workspace="_platform" permissions={permissions} />
        : <div className="min-h-0 space-y-8 overflow-auto"><ResourceForm key={`${item}/${route.id}/${route.mode}`} config={form} workspace="_platform" permissions={permissions} editInPlace={item === "settings"}
          onSaved={() => { if (item === "settings") {toast.success(commonText("resource-domain:platform.saved")); window.dispatchEvent(new Event(FOOTER_UPDATED));} reload(); }} /></div>}
    </main>
    </div>
  </div>;
}
