import { useEffect } from "react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { api, type Identity } from "./api";
import { ResourceForm } from "./components/ResourceForm";
import { ResourceTable } from "./components/ResourceTable";
import type { FormConfig, ListConfig } from "./components/dsl";
import { href, navigate, useRoute } from "./router";
import { nav } from "./nav";

type Row = Record<string, unknown> & { id: string; name: string; email: string; slug: string; active: boolean; is_superadmin: boolean };
const labels: Record<string, string> = { users: "平台用户", workspaces: "工作空间", settings: "平台设置" };
const permissions = new Set(["users.update", "workspaces.create", "workspaces.update", "settings.update"]);
const edit = { label: "编辑", to: (r: Row) => ({ id: r.id, mode: "edit" as const }) };
const lists: Record<string, ListConfig<Row>> = {
  users: {
    resource: "users", endpoint: "/platform/users", rowKey: r => r.id,
    filters: [{ key: "q", label: "姓名或邮箱", type: "text" }],
    columns: [{ key: "name", label: "姓名", sortable: true }, { key: "email", label: "邮箱", sortable: true },
      { key: "active", label: "状态", render: r => r.active ? "启用" : "停用" },
      { key: "is_superadmin", label: "平台角色", render: r => r.is_superadmin ? "超级管理员" : "普通用户" }],
    toolbar: { create: false, import: false, export: false },
    actions: [edit], empty: "暂无平台用户",
  },
  workspaces: {
    resource: "workspaces", endpoint: "/platform/workspaces", rowKey: r => r.id,
    filters: [{ key: "q", label: "名称或标识", type: "text" }],
    columns: [{ key: "name", label: "名称", sortable: true }, { key: "slug", label: "标识", sortable: true }],
    actions: [edit, { label: "进入工作空间", to: r => ({ workspace: r.slug, group: nav[0]!.key, item: nav[0]!.items[0]!.key, mode: "list" }) }],
    toolbar: { create: { label: "创建工作空间" }, import: false, export: false }, empty: "还没有工作空间，创建后即可配置主机和部署。",
  },
};
const forms: Record<string, FormConfig<Row>> = {
  users: { resource: "users", endpoint: "/platform/users", heading: r => r.name, blank: {}, sections: [{ fields: [
    { key: "email", label: "邮箱", type: "text", immutable: true },
    { key: "name", label: "姓名", type: "text", immutable: true },
    { key: "active", label: "启用账号", type: "switch", help: "停用后该账号的所有会话立即失效。" },
    { key: "is_superadmin", label: "平台超级管理员", type: "switch", help: "可管理平台及全部工作空间。变更后需要重新登录；必须保留一名启用的超管。" },
  ] }] },
  workspaces: { resource: "workspaces", endpoint: "/platform/workspaces", heading: r => r.name, blank: { name: "", slug: "" }, sections: [{ fields: [
    { key: "name", label: "名称", type: "text", required: true },
    { key: "slug", label: "标识", type: "text", required: true, immutable: true, help: "2–40 位小写字母、数字、中划线；创建后不可修改。" },
  ] }] },
  settings: { resource: "settings", endpoint: "/platform/settings", heading: () => "平台设置", blank: {}, sections: [{ title: "账号注册", fields: [
    { key: "registration_enabled", label: "允许注册新账号", type: "switch", help: "关闭后已有账号仍可登录，公开注册入口将关闭。" },
  ] }, { title: "仓库网页授权 · 公共配置", fields: [
    { key: "oauth_public_url", label: "平台公开访问地址", type: "text", placeholder: "https://ops.example.com", span: 2, help: "填写用户浏览器可访问的平台地址。保存后立即生效，无需重启；各工作空间仍分别授权自己的账号。" },
    { key: "oauth_callback_url", label: "当前回调地址", type: "text", immutable: true, span: 2, help: "将此地址登记到代码平台的 OAuth 应用。修改公开访问地址并保存后，这里会显示新的回调地址。" },
    { key: "oauth_source", label: "配置来源", type: "text", immutable: true, span: 2 },
  ] }, ...([ ["github", "GitHub"], ["gitlab", "GitLab"], ["gitee", "Gitee"], ["gitea", "Gitea"] ] as const).map(([key, label]) => ({ title: `${label} OAuth 应用`, fields: [
    { key: `oauth_${key}_enabled`, label: `启用 ${label} 网页授权`, type: "switch" as const, span: 2 as const, help: "关闭后不再使用该平台的网页授权凭据；已保存的应用配置保留。Codeup 使用个人访问令牌，无需配置 OAuth 应用。" },
    { key: `oauth_${key}_client_id`, label: `${label} 应用 ID`, type: "text" as const },
    { key: `oauth_${key}_client_secret`, label: `${label} 应用密钥`, type: "secret" as const, stored: `oauth_${key}_has_client_secret`, replaceOnEdit: true, placeholder: "留空保留已有应用密钥", help: "加密保存，不回显；留空保留已有密钥。修改应用 ID 或接入点后，原账号需重新授权。" },
    { key: `oauth_${key}_base_url`, label: `${label} 网站地址`, type: "text" as const, placeholder: key === "gitea" ? "https://git.example.com" : `https://${key}.com`, help: "留空使用官方地址；自建 GitLab／GitHub Enterprise 可填写自己的地址。" },
    { key: `oauth_${key}_api_url`, label: `${label} API 地址`, type: "text" as const, help: "通常留空，由网站地址自动推导；仅在使用自定义 API 接入点时填写。" },
  ] }))] },
};

export function Platform({ identity, reload }: { identity: Identity; reload: () => void }) {
  const route = useRoute();
  const item = route.workspace === "_platform" ? route.item : "users";
  useEffect(() => {
    if (route.workspace !== "_platform") navigate({ workspace: "_platform", group: "platform", item: "users", mode: "list" }, {}, { replace: true });
    else if (item === "settings" && !route.id) navigate({ ...route, id: "1", mode: "edit" }, {}, { replace: true });
  }, [route.workspace, route.id, item]);
  if (!identity.user.is_superadmin) return <div className="p-6 space-y-4"><p>当前账号没有平台管理权限。</p><Button onClick={() => navigate({ workspace: identity.workspaces[0]?.slug ?? "", mode: "list" }, {})}>返回工作空间</Button></div>;
  const form = forms[item]!;
  return <div className="flex h-full flex-col">
    <header className="flex flex-wrap items-center gap-4 border-b px-5 py-3">
      <h1 className="font-semibold">平台管理</h1>
      <nav aria-label="平台管理" className="flex flex-wrap gap-2">
        {Object.entries(labels).map(([key, label]) => <a key={key} href={href({ workspace: "_platform", item: key, mode: key === "settings" ? "edit" : "list", ...(key === "settings" ? { id: "1" } : {}) }, {})}
          aria-current={key === item ? "page" : undefined} className={`rounded px-3 py-2 text-sm ${key === item ? "bg-accent" : "text-muted-foreground"}`}
          onClick={event => { if (event.button || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return; event.preventDefault(); navigate({ workspace: "_platform", item: key, mode: key === "settings" ? "edit" : "list", ...(key === "settings" ? { id: "1" } : {}) }, {}); }}>{label}</a>)}
      </nav>
      <span className="flex-1" /><span className="text-sm text-muted-foreground">{identity.user.name}</span>
      <Button variant="ghost" onClick={() => { void api("/sessions", { method: "DELETE" }).then(reload); }}>退出登录</Button>
    </header>
    <main className="flex min-h-0 min-w-0 flex-1 flex-col p-5">
      <h2 className="mb-4 shrink-0 text-lg font-semibold">{labels[item]}</h2>
      {route.mode === "list" && item !== "settings" ? <ResourceTable key={item} config={lists[item]!} workspace="_platform" permissions={permissions} />
        : <div className="min-h-0 overflow-auto"><ResourceForm key={`${item}/${route.id}/${route.mode}`} config={form} workspace="_platform" permissions={permissions} onSaved={() => { if (item === "settings") toast.success("平台设置已保存，立即生效。"); reload(); }} /></div>}
    </main>
  </div>;
}
