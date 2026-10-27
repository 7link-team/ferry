import { useEffect, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { api, errorText } from "../api";

const platforms = [["github", "GitHub"], ["gitlab", "GitLab"], ["gitee", "Gitee"], ["gitea", "Gitea"], ["codeup", "Codeup"], ["git", "普通 Git"]] as const;
type Connection = { id: string; web_available: boolean; credential_id: string | null; account: string; setup: string; state: string };
type Remote = { name: string; full_name: string; url: string; default_branch: string };
type Repositories = { items: Remote[]; page: number; has_more: boolean; credential_id: string; account: string };
const selectClass = "h-10 w-full rounded-md border border-input bg-background px-3 text-sm";

/** Repository connection is an input of the shared resource form, never a separate credential page. */
export function RepositorySource({ workspace, draft, readOnly, onChange, onValidityChange, platformOnly = false }: {
  workspace: string; draft: Record<string, unknown>; readOnly: boolean; platformOnly?: boolean;
  onChange: (patch: Record<string, unknown>) => void; onValidityChange: (blocked: boolean) => void;
}) {
  const provider = String(draft.provider ?? "git");
  const change = useRef(onChange); change.current = onChange;
  const popup = useRef<Window | null>(null);
  const channel = useRef<BroadcastChannel | null>(null);
  const [connection, setConnection] = useState<Connection>();
  const [method, setMethod] = useState("token");
  const [token, setToken] = useState("");
  const [base, setBase] = useState("");
  const [apiBase, setApiBase] = useState("");
  const [organization, setOrganization] = useState("");
  const [username, setUsername] = useState("");
  const [result, setResult] = useState<Repositories>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [waiting, setWaiting] = useState(false);
  const [gitMethod, setGitMethod] = useState(draft.credential_id && draft.credential_id !== "0" ? "existing" : "none");
  const requestId = useRef(0);
  const root = `/workspaces/${encodeURIComponent(workspace)}/repository-auth/${provider}`;
  const inline = (draft.inline_credential ?? {}) as Record<string, string>;

  useEffect(() => {
    onValidityChange(!readOnly && (busy || waiting || !provider || !draft.url || (provider !== "git" && (!draft.credential_id || draft.credential_id === "0"))));
    return () => onValidityChange(false);
  }, [readOnly, busy, waiting, provider, draft.url, draft.credential_id, onValidityChange]);

  useEffect(() => {
    const id = ++requestId.current;
    setConnection(undefined); setResult(undefined); setError(""); setToken(""); setBase(""); setApiBase(""); setOrganization(""); setUsername(""); setWaiting(false);
    if (readOnly || !provider || provider === "git") { setBusy(false); return; }
    setBusy(true);
    void api<{ items: Connection[] }>(`/workspaces/${encodeURIComponent(workspace)}/repository-auth`).then(data => {
      if (id !== requestId.current) return;
      const connected = data.items.find(row => row.id === provider);
      setConnection(connected); setMethod(connected?.web_available ? "web" : "token");
    }).catch(cause => { if (id === requestId.current) setError(errorText(cause)); })
      .finally(() => { if (id === requestId.current) setBusy(false); });
    return () => { requestId.current++; popup.current?.close(); popup.current = null; channel.current?.close(); channel.current = null; };
  }, [workspace, provider, readOnly]);

  async function browse(page = 1) {
    const id = requestId.current;
    setBusy(true); setError("");
    try {
      const data = await api<Repositories>(`${root}/repositories?page=${page}`);
      if (id === requestId.current) setResult(data);
    } catch (cause) { if (id === requestId.current) setError(errorText(cause)); }
    finally { if (id === requestId.current) setBusy(false); }
  }
  useEffect(() => {
    const receive = (event: MessageEvent) => {
      if (event.origin !== location.origin || event.source !== popup.current || event.data?.type !== "repository-authorized" || event.data?.provider !== provider) return;
      popup.current?.close(); popup.current = null; channel.current?.close(); channel.current = null; setWaiting(false);
      if (event.data.status === "connected") void browse();
      else setError("已取消网页授权，可重新授权或使用个人访问令牌。");
    };
    window.addEventListener("message", receive);
    return () => { window.removeEventListener("message", receive); };
  }, [root, provider]);

  async function connect() {
    change.current({url: "", credential_id: "0"});
    const id = requestId.current;
    setBusy(true); setError("");
    try {
      const data = await api<Repositories>(`${root}/actions/connect`, { method: "POST", body: JSON.stringify({ token, base_url: base, api_url: apiBase, organization, username }) });
      if (id === requestId.current) { setResult(data); setToken(""); }
    } catch (cause) { if (id === requestId.current) setError(errorText(cause)); }
    finally { if (id === requestId.current) setBusy(false); }
  }
  async function authorize() {
    change.current({url: "", credential_id: "0"});
    const opened = window.open("about:blank", "_blank", "popup,width=720,height=780");
    if (!opened) { setError("浏览器阻止了授权窗口，请允许弹窗后重试。"); return; }
    popup.current = opened; setWaiting(true); setError("");
    const id = requestId.current;
    const key = crypto.randomUUID();
    const broadcast = new BroadcastChannel(`repository-authorized-${key}`); channel.current = broadcast;
    broadcast.onmessage = event => {
      if (id !== requestId.current || channel.current !== broadcast || event.data?.type !== "repository-authorized" || event.data.provider !== provider) return;
      broadcast.close(); channel.current = null; popup.current?.close(); popup.current = null; setWaiting(false);
      if (event.data.status === "connected") void browse();
      else setError("已取消网页授权，可重新授权或使用个人访问令牌。");
    };
    try {
      const data = await api<{ url: string }>(`${root}/actions/authorize?popup=true&channel=${key}`, { method: "POST", body: "{}" });
      if (id === requestId.current && !opened.closed) opened.location.href = data.url;
      else opened.close();
    } catch (cause) { opened.close(); broadcast.close(); channel.current = null; if (id === requestId.current) { setWaiting(false); setError(errorText(cause)); } }
  }
  function selectProvider(value: string) {
    requestId.current++;
    setGitMethod("none");
    change.current({ provider: value, url: "", credential_id: "0", inline_credential: null });
  }
  if (readOnly) return <dl className="space-y-2 text-sm"><div><dt className="text-muted-foreground">代码来源</dt><dd>{platforms.find(([key]) => key === provider)?.[1] ?? provider}</dd></div><div><dt className="text-muted-foreground">仓库地址</dt><dd className="break-all">{String(draft.url ?? "")}</dd></div><div><dt className="text-muted-foreground">授权</dt><dd>{draft.credential_id && draft.credential_id !== "0" ? "已绑定加密访问凭据" : "无凭据（公开仓库）"}</dd></div></dl>;

  return <div className="space-y-4">
    <label className="block space-y-2"><span className="text-sm font-medium">代码来源</span><select aria-label="代码来源" className={selectClass} value={provider} onChange={e => selectProvider(e.target.value)}><option value="">请选择平台或普通 Git</option>{platforms.filter(([key]) => !platformOnly || key !== "git").map(([key, label]) => <option key={key} value={key}>{label}</option>)}</select></label>
    {provider === "git" ? <>
      <label className="block space-y-2"><span className="text-sm font-medium">Git 仓库地址</span><Input value={String(draft.url ?? "")} placeholder="https://git.example.com/team/repo.git 或 git@git.example.com:team/repo.git" onChange={e => change.current({ url: e.target.value })} required /></label>
      <label className="block space-y-2"><span className="text-sm font-medium">Git 授权方式</span><select aria-label="Git 授权方式" className={selectClass} value={gitMethod} onChange={e => {
        const method = e.target.value; setGitMethod(method);
        change.current({ credential_id: method === "existing" ? draft.credential_id : "0", inline_credential: ["none", "existing"].includes(method) ? null : { kind: method, username: "", secret: "" } });
      }}>{draft.id && draft.credential_id && draft.credential_id !== "0" ? <option value="existing">保持现有授权</option> : null}<option value="none">无需授权（公开仓库）</option><option value="password">账号密码</option><option value="ssh-key">SSH 私钥</option><option value="token">访问令牌</option></select></label>
      {!["none", "existing"].includes(gitMethod) && <>
        <label className="block space-y-2"><span className="text-sm font-medium">Git 账号{gitMethod !== "password" && "（可选）"}</span><Input value={inline.username ?? ""} autoComplete="off" required={gitMethod === "password"} onChange={e => change.current({ inline_credential: { ...inline, username: e.target.value } })} /></label>
        <label className="block space-y-2"><span className="text-sm font-medium">{gitMethod === "ssh-key" ? "SSH 私钥" : gitMethod === "password" ? "Git 密码" : "Git 访问令牌"}</span>{gitMethod === "ssh-key" ? <textarea className={`${selectClass} min-h-32 font-mono`} autoComplete="off" value={inline.secret ?? ""} required onChange={e => change.current({ inline_credential: { ...inline, secret: e.target.value } })} /> : <Input type="password" autoComplete="new-password" value={inline.secret ?? ""} required onChange={e => change.current({ inline_credential: { ...inline, secret: e.target.value } })} />}</label>
        <p className="text-xs text-muted-foreground">授权信息与仓库一起保存并加密，无需单独创建凭据。</p>
      </>}
    </> : provider && <>
      <label className="block space-y-2"><span className="text-sm font-medium">平台授权方式</span><select aria-label="平台授权方式" className={selectClass} value={method} disabled={busy || waiting} onChange={e => { setMethod(e.target.value); setError(""); setResult(undefined); change.current({url: "", credential_id: "0"}); }}>
        {provider !== "codeup" && <option value="web">网页授权</option>}<option value="token">个人访问令牌</option>
      </select></label>
      {method === "web" ? <>
        {!connection?.web_available && <p role="status" className="text-sm text-muted-foreground">{connection?.setup || "正在读取平台授权配置…"} 请由超管在“平台设置”中配置 OAuth 应用。</p>}
        <Button type="button" variant="outline" disabled={busy || waiting || !connection?.web_available} onClick={() => void authorize()}>{waiting ? "等待网页授权…" : "打开网页授权"}</Button>
        {waiting && <Button type="button" variant="ghost" onClick={() => { popup.current?.close(); popup.current = null; channel.current?.close(); channel.current = null; setWaiting(false); }}>取消授权</Button>}
      </> : <div className="space-y-4">
        <label className="block space-y-2"><span className="text-sm font-medium">个人访问令牌</span><Input type="password" autoComplete="new-password" value={token} onChange={e => setToken(e.target.value)} /></label>
        {provider === "codeup" && <label className="block space-y-2"><span className="text-sm font-medium">Codeup Git 用户名</span><Input value={username} onChange={e => setUsername(e.target.value)} /><span className="text-xs text-muted-foreground">Codeup 不通过令牌返回 Git 用户名；请填写后续拉取仓库使用的账号。</span></label>}
        {provider === "codeup" && <label className="block space-y-2"><span className="text-sm font-medium">Codeup 组织 ID</span><Input value={organization} onChange={e => setOrganization(e.target.value)} /><span className="text-xs text-muted-foreground">中心版必填；Region 版请在自定义接入点中填写对应网站和 API 地址。</span></label>}
        <details open={provider === "gitea"}><summary className="cursor-pointer text-sm">自定义接入点{provider === "gitea" ? "（Gitea 必填网站地址）" : "（自建平台时填写）"}</summary><div className="mt-3 grid gap-4 sm:grid-cols-2">
          <label className="space-y-2"><span className="text-sm">平台网站地址</span><Input placeholder={provider === "gitea" ? "https://git.example.com" : "留空使用官方地址"} value={base} onChange={e => setBase(e.target.value)} /></label>
          <label className="space-y-2"><span className="text-sm">平台 API 地址</span><Input placeholder="留空自动推导" value={apiBase} onChange={e => setApiBase(e.target.value)} /></label>
        </div></details>
        <Button type="button" variant="outline" disabled={busy || !token || (provider === "gitea" && !base) || (provider === "codeup" && !username)} onClick={() => void connect()}>{busy ? "正在验证连接…" : "连接并读取仓库"}</Button>
      </div>}
      {connection?.credential_id && !result && <Button type="button" variant="outline" disabled={busy || waiting} onClick={() => void browse()}>使用已有连接{connection.account ? ` · ${connection.account}` : ""}</Button>}
      {result && <div className="space-y-3">
        <p role="status" className="text-sm">已连接：{result.account}</p>
        <label className="block space-y-2"><span className="text-sm font-medium">选择平台仓库</span><select aria-label="选择平台仓库" className={selectClass} value={String(draft.url ?? "")} disabled={busy} onChange={e => {
          const repo = result.items.find(row => row.url === e.target.value);
          if (repo) change.current({ url: repo.url, credential_id: result.credential_id, default_branch: repo.default_branch || "main", ...(!draft.name ? { name: repo.name } : {}) });
          else change.current({ url: "", credential_id: "0" });
        }}><option value="">{result.items.length ? "请选择要接入的仓库" : "此账号没有可访问的仓库"}</option>{result.items.map(repo => <option key={repo.url} value={repo.url}>{repo.full_name || repo.name}</option>)}</select></label>
        <div className="flex items-center gap-3 text-sm"><Button type="button" variant="ghost" disabled={busy || result.page <= 1} onClick={() => void browse(result.page - 1)}>上一页</Button><span>第 {result.page} 页</span><Button type="button" variant="ghost" disabled={busy || !result.has_more} onClick={() => void browse(result.page + 1)}>下一页</Button></div>
      </div>}
      {draft.url && <p className="break-all text-sm text-muted-foreground">已选仓库：{String(draft.url)}</p>}
    </>}
    {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
  </div>;
}
