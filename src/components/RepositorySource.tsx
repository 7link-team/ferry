import { useEffect, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { api, errorText } from "../api";
import { navigate } from "../router";
import type { Credential } from "../resources";

const platforms = [["github", "GitHub"], ["gitlab", "GitLab"], ["gitee", "Gitee"], ["gitea", "Gitea"], ["codeup", "Codeup"], ["git", "普通 Git"]] as const;
type PlatformAuthorization = { id: string; web_available: boolean; setup?: string };
const selectClass = "h-10 w-full rounded-md border border-input bg-background px-3 text-sm";

/** Named platform authorization; repository and branch selection belongs to checkout nodes. */
export function RepositorySource({ workspace, draft, readOnly, onChange, onValidityChange }: {
  workspace: string; draft: Record<string, unknown>; readOnly: boolean;
  onChange: (patch: Record<string, unknown>) => void; onValidityChange: (blocked: boolean) => void;
}) {
  const provider = String(draft.provider ?? "git");
  const credentialId = String(draft.credential_id ?? "0");
  const existingOAuth = !!draft.id && draft.authorization === "oauth";
  const [method, setMethod] = useState(existingOAuth ? "web" : "token");
  const [credentials, setCredentials] = useState<Credential[]>([]);
  const [platform, setPlatform] = useState<PlatformAuthorization>();
  const [loadingCredentials, setLoadingCredentials] = useState(!readOnly);
  const [loadingPlatform, setLoadingPlatform] = useState(false);
  const [refresh, setRefresh] = useState(0);
  const [credentialError, setCredentialError] = useState("");
  const [platformError, setPlatformError] = useState("");
  const [authorizationError, setAuthorizationError] = useState("");
  const [waiting, setWaiting] = useState(false);
  const popup = useRef<Window | null>(null);
  const channel = useRef<BroadcastChannel | null>(null);
  const authorizationRequest = useRef(0);
  const root = `/workspaces/${encodeURIComponent(workspace)}`;
  const hasCredential = !!credentialId && credentialId !== "0";
  const availableCredentials = credentials.filter(row => provider === "git" ? ["token", "password", "ssh-key"].includes(row.kind) : row.kind === "token");

  function closeAuthorization() {
    authorizationRequest.current++;
    popup.current?.close(); popup.current = null;
    channel.current?.close(); channel.current = null;
  }
  useEffect(() => {
    if (readOnly) return;
    let active = true;
    setLoadingCredentials(true); setCredentialError("");
    void (async () => {
      const all: Credential[] = [];
      for (let page = 1; active; page++) {
        const data = await api<{items: Credential[];total?: number;page_size?: number}>(`${root}/credentials?scope=build&page=${page}&page_size=200`);
        all.push(...data.items);
        if (!data.items.length || all.length >= (data.total ?? all.length)) break;
      }
      if (active) setCredentials(all);
    })().catch(cause => { if (active) setCredentialError(errorText(cause)); })
      .finally(() => { if (active) setLoadingCredentials(false); });
    return () => { active = false; };
  }, [root, readOnly, refresh]);
  useEffect(() => {
    let active = true;
    setPlatform(undefined); setPlatformError(""); setAuthorizationError(""); setWaiting(false);
    if (!readOnly && provider && provider !== "git") {
      setLoadingPlatform(true);
      void api<{ items: PlatformAuthorization[] }>(`${root}/repository-providers`).then(data => {
        if (active) setPlatform(data.items.find(row => row.id === provider));
      }).catch(cause => { if (active) setPlatformError(errorText(cause)); })
        .finally(() => { if (active) setLoadingPlatform(false); });
    } else setLoadingPlatform(false);
    return () => { active = false; closeAuthorization(); };
  }, [root, provider, readOnly, refresh]);
  useEffect(() => {
    const incomplete = !provider || waiting || (provider !== "git" && (
      method === "web" ? !existingOAuth : !hasCredential || (provider === "gitea" && !String(draft.base_url ?? "").trim())
    ));
    onValidityChange(!readOnly && incomplete);
    return () => onValidityChange(false);
  }, [provider, method, existingOAuth, hasCredential, draft.base_url, waiting, readOnly, onValidityChange]);

  function authorized(data: { status?: string; connection_id?: string }) {
    closeAuthorization(); setWaiting(false);
    if (data.status === "connected" && data.connection_id) {
      navigate({ workspace, group: "build", item: "repositories", id: encodeURIComponent(String(data.connection_id)), mode: "view" }, {});
    } else setAuthorizationError(data.status === "connected" ? "授权回调缺少连接标识，请刷新连接列表确认结果。" : "已取消网页授权，可重新授权或选择已有授权认证。");
  }
  useEffect(() => {
    const receive = (event: MessageEvent) => {
      if (!popup.current || event.origin !== location.origin || event.source !== popup.current || event.data?.type !== "repository-authorized" || event.data.provider !== provider) return;
      authorized(event.data);
    };
    window.addEventListener("message", receive);
    return () => window.removeEventListener("message", receive);
  }, [workspace, provider]);

  async function authorize() {
    if (!String(draft.name ?? "").trim() || !platform?.web_available) return;
    const opened = window.open("about:blank", "_blank", "popup,width=720,height=780");
    if (!opened) { setAuthorizationError("浏览器阻止了授权窗口，请允许弹窗后重试。"); return; }
    closeAuthorization(); popup.current = opened;
    const request = authorizationRequest.current;
    setWaiting(true); setAuthorizationError("");
    const key = crypto.randomUUID();
    const broadcast = new BroadcastChannel(`repository-authorized-${key}`); channel.current = broadcast;
    broadcast.onmessage = event => {
      if (request !== authorizationRequest.current || channel.current !== broadcast || event.data?.type !== "repository-authorized" || event.data.provider !== provider) return;
      authorized(event.data);
    };
    const query = new URLSearchParams({ popup: "true", channel: key, name: String(draft.name).trim(), description: String(draft.description ?? "") });
    if (existingOAuth) query.set("connection_id", String(draft.id));
    try {
      const data = await api<{ url: string }>(`${root}/repository-providers/${provider}/authorizations?${query}`, { method: "POST", body: "{}" });
      if (request === authorizationRequest.current && !opened.closed) opened.location.href = data.url;
      else opened.close();
    } catch (cause) {
      if (request === authorizationRequest.current) { closeAuthorization(); setWaiting(false); setAuthorizationError(errorText(cause)); }
    }
  }

  if (readOnly) return <dl className="space-y-3 text-sm">
    <div><dt className="text-muted-foreground">代码平台</dt><dd>{platforms.find(([key]) => key === provider)?.[1] ?? provider}</dd></div>
    <div><dt className="text-muted-foreground">授权认证</dt><dd>{draft.authorization === "oauth" ? "网页授权" : hasCredential ? `已绑定授权认证 · ${credentialId}` : "无需授权（公开仓库）"}</dd></div>
    {([["base_url", "平台网站地址"], ["api_url", "平台 API 地址"], ["organization", "组织 ID"], ...(provider === "git" ? [["url", "Git 仓库地址"]] : [])]).map(([key, label]) => draft[key!] ? <div key={key}><dt className="text-muted-foreground">{label}</dt><dd className="break-all">{String(draft[key!])}</dd></div> : null)}
  </dl>;

  return <div className="space-y-4">
    <label className="block space-y-2"><span className="text-sm font-medium">代码平台</span><select aria-label="代码平台" className={selectClass} value={provider} disabled={waiting || existingOAuth} onChange={event => {
      setMethod("token");
      onChange({ provider: event.target.value, credential_id: "0", base_url: "", api_url: "", organization: "", url: "" });
    }}><option value="">请选择平台或普通 Git</option>{platforms.map(([key, label]) => <option key={key} value={key}>{label}</option>)}</select></label>
    {provider && provider !== "git" && <label className="block space-y-2"><span className="text-sm font-medium">平台授权方式</span><select aria-label="平台授权方式" className={selectClass} value={method} disabled={waiting || !!draft.id} onChange={event => { closeAuthorization(); setWaiting(false); setMethod(event.target.value); setAuthorizationError(""); }}><option value="token">已有授权认证（个人访问令牌）</option>{provider !== "codeup" && <option value="web" disabled={!platform?.web_available && !existingOAuth}>网页授权</option>}</select></label>}
    {provider && (provider === "git" || method === "token") && <>
      <label className="block space-y-2"><span className="text-sm font-medium">授权认证</span><select aria-label="授权认证" className={selectClass} value={credentialId} disabled={loadingCredentials} onChange={event => onChange({ credential_id: event.target.value })}><option value="0">{provider === "git" ? "无需授权（公开仓库）" : "请选择个人访问令牌"}</option>{hasCredential && !availableCredentials.some(row => row.id === credentialId) && <option value={credentialId}>保留已绑定的授权认证 · {credentialId}</option>}{availableCredentials.map(row => <option key={row.id} value={row.id}>{row.name}</option>)}</select></label>
      {loadingCredentials ? <p role="status" className="text-sm text-muted-foreground">正在读取授权认证…</p> : !availableCredentials.length && !credentialError && <p role="status" className="text-sm text-muted-foreground">没有可用的授权认证，请先新建后刷新列表。</p>}
      <div className="flex flex-wrap items-center gap-3"><a className="text-sm text-primary underline underline-offset-4" href={`/workspaces/${encodeURIComponent(workspace)}/credentials/new?scope=build`} target="_blank" rel="noopener noreferrer">新建授权认证</a><Button type="button" variant="outline" disabled={loadingCredentials} onClick={() => setRefresh(value => value + 1)}>刷新授权认证</Button></div>
      <p className="text-xs text-muted-foreground">{provider === "git" ? "可复用访问令牌、账号密码或 SSH 私钥。" : "选择已保存的个人访问令牌；连接只引用授权认证，不复制密钥。"}</p>
      {provider === "git" ? <label className="block space-y-2"><span className="text-sm font-medium">Git 仓库地址（可选）</span><Input aria-label="Git 仓库地址（可选）" value={String(draft.url ?? "")} placeholder="可在流水线拉取代码步骤中填写" onChange={event => onChange({ url: event.target.value })} /></label> : <>
        {provider === "codeup" && <label className="block space-y-2"><span className="text-sm font-medium">Codeup 组织 ID</span><Input aria-label="Codeup 组织 ID" value={String(draft.organization ?? "")} onChange={event => onChange({ organization: event.target.value })} /><span className="block text-xs text-muted-foreground">中心版必填组织 ID；Region 版可留空，并在下方填写实例的网站和 API 地址。授权认证需填写 Git 用户名，令牌需具备代码仓库、分支只读权限和 Git 拉取权限。</span></label>}
        <details open={provider === "gitea" || !!draft.base_url || !!draft.api_url}><summary className="cursor-pointer text-sm">平台接入地址{provider === "gitea" ? "（网站地址必填）" : "（自建平台时填写）"}</summary><div className="mt-3 grid gap-4 sm:grid-cols-2">
          <label className="space-y-2"><span className="text-sm">平台网站地址</span><Input aria-label="平台网站地址" required={provider === "gitea"} placeholder="留空使用官方地址" value={String(draft.base_url ?? "")} onChange={event => onChange({ base_url: event.target.value })} /></label>
          <label className="space-y-2"><span className="text-sm">平台 API 地址</span><Input aria-label="平台 API 地址" placeholder="留空自动推导" value={String(draft.api_url ?? "")} onChange={event => onChange({ api_url: event.target.value })} /></label>
        </div></details>
      </>}
    </>}
    {provider && provider !== "git" && method === "web" && <div className="space-y-3">
      <p className="text-sm text-muted-foreground">授权成功后会保存此命名连接，并打开连接详情。</p>
      {!String(draft.name ?? "").trim() && <p className="text-sm text-muted-foreground">请先填写连接名称。</p>}
      <Button type="button" variant="outline" disabled={loadingPlatform || waiting || !platform?.web_available || !String(draft.name ?? "").trim()} onClick={() => void authorize()}>{waiting ? "等待网页授权…" : existingOAuth ? "重新网页授权" : "打开网页授权"}</Button>
      {waiting && <Button type="button" variant="ghost" onClick={() => { closeAuthorization(); setWaiting(false); }}>取消授权</Button>}
    </div>}
    {provider && provider !== "git" && loadingPlatform && <p role="status" className="text-sm text-muted-foreground">正在读取平台授权配置…</p>}
    {provider && provider !== "git" && !loadingPlatform && !platform?.web_available && !platformError && <p className="text-xs text-muted-foreground">{platform?.setup || "此平台尚未配置网页授权，可选择已有授权认证。"}</p>}
    {[credentialError, platformError, authorizationError].filter(Boolean).map((error, index) => <p key={index} role="alert" className="text-sm text-destructive">{error}</p>)}
  </div>;
}
