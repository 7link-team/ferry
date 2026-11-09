import { LoadingSkeleton } from "../../shared/ui/loading-skeleton";
import { useEffect,useRef,useState } from "react";
import { api,errorText } from "../../shared/api/api";
import { named } from "../../shared/api/resource";
import { AutoField,selectDisplay } from "../../shared/forms/auto-form";
import { SelectRefresh } from "../../shared/forms/SelectRefresh";
import { hostRepositoryText,useCommonTranslation } from "../../shared/i18n/i18n";
import type { Navigation } from "../../shared/routing/types";
import { Button } from "../../shared/ui/button";
import { Input } from "../../shared/ui/input";
import type { Credential } from "../credentials/types";

const platforms = [["github", "GitHub"], ["gitlab", "GitLab"], ["gitee", "Gitee"], ["gitea", "Gitea"], ["codeup", "Codeup"], ["git", "Git"]] as const;
export const repositoryProviderLabel = (value: string) => selectDisplay(value, { options: platforms.map(([value, label]) => ({ value, label: value === "git" ? hostRepositoryText("repository.001") : label })) });
type PlatformAuthorization = { id: string; web_available: boolean; setup?: string };
const selectClass = "h-10 w-full rounded-md border border-input bg-background px-3 text-sm";

/** Named platform authorization; repository and branch selection belongs to checkout nodes. */
export function RepositorySource({ navigation, workspace, draft, readOnly, onChange, onValidityChange, onAuthorized, credentialHref, fixedProvider }: { navigation?: Navigation;
  workspace: string; draft: Record<string, unknown>; readOnly: boolean;
  onChange: (patch: Record<string, unknown>) => void; onValidityChange: (blocked: boolean) => void;
  onAuthorized?: (id: string) => void;
  credentialHref?: string;
  fixedProvider?: boolean;
}) {
  useCommonTranslation();
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
  const [preset, setPreset] = useState("");
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
  function changeProvider(value: string) {
    closeAuthorization(); setWaiting(false); setMethod("token"); setPreset("");
    onChange({ provider: value, credential_id: "0", base_url: "", api_url: "", organization: "", url: "" });
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
      if (onAuthorized) { onAuthorized(String(data.connection_id)); return; }
      navigation?.navigate({ workspace, group: "repository", item: "repositories", id: encodeURIComponent(String(data.connection_id)), mode: "view" }, {});
    } else setAuthorizationError(data.status === "connected" ? hostRepositoryText("repository.034") : hostRepositoryText("repository.035"));
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
    if (!opened) { setAuthorizationError(hostRepositoryText("repository.036")); return; }
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
    <div><dt className="text-muted-foreground">{hostRepositoryText("repository.037")}</dt><dd>{repositoryProviderLabel(provider)}</dd></div>
    <div><dt className="text-muted-foreground">{hostRepositoryText("repository.038")}</dt><dd>{draft.authorization === "oauth" ? hostRepositoryText("repository.039") : hasCredential ? named(typeof draft.credential_name === "string" ? draft.credential_name : undefined, credentialId) : hostRepositoryText("repository.040")}</dd></div>
    {([["base_url", hostRepositoryText("repository.041")], ["api_url", hostRepositoryText("repository.042")], ["organization", hostRepositoryText("repository.043")], ...(provider === "git" ? [["url", hostRepositoryText("repository.023")]] : [])]).map(([key, label]) => draft[key!] ? <div key={key}><dt className="text-muted-foreground">{label}</dt><dd className="break-all">{String(draft[key!])}</dd></div> : null)}
  </dl>;

  return <div className="space-y-4">
    {!draft.id && !fixedProvider && <fieldset className="min-w-0 space-y-2">
      <legend className="text-sm font-medium">{hostRepositoryText("repository.presets.title")}</legend>
      <div className="grid gap-2 sm:grid-cols-3">
        {(["github", "codeup", "git"] as const).map(value => <Button key={value} type="button" variant={preset === value ? "secondary" : "outline"}
          className="h-auto min-h-11 min-w-0 whitespace-normal px-3 py-2" aria-pressed={preset === value} disabled={waiting}
          onClick={() => { changeProvider(value); setPreset(value); }}>
          {hostRepositoryText(`repository.presets.${value}`)}
        </Button>)}
      </div>
      <p role="status" className="text-xs leading-relaxed text-muted-foreground">{hostRepositoryText(preset ? `repository.presets.${preset}Help` : "repository.presets.help")}</p>
    </fieldset>}
    <AutoField label={<span className="text-sm font-medium">{hostRepositoryText("repository.037")}</span>} className="block space-y-2"><select aria-label={hostRepositoryText("repository.037")} className={selectClass} value={provider} disabled={waiting || existingOAuth || fixedProvider} onChange={event => {
      changeProvider(event.target.value);
    }}><option value="">{hostRepositoryText("repository.044")}</option>{provider && !platforms.some(([key]) => key === provider) && <option value={provider}>{repositoryProviderLabel(provider)}</option>}{platforms.map(([key]) => <option key={key} value={key}>{repositoryProviderLabel(key)}</option>)}</select></AutoField>
    {provider && provider !== "git" && <AutoField label={<span className="text-sm font-medium">{hostRepositoryText("repository.045")}</span>} className="block space-y-2"><select aria-label={hostRepositoryText("repository.045")} className={selectClass} value={method} disabled={waiting || !!draft.id} onChange={event => { closeAuthorization(); setWaiting(false); setMethod(event.target.value); setAuthorizationError(""); }}><option value="token">{hostRepositoryText("repository.046")}</option>{provider !== "codeup" && <option value="web" disabled={!platform?.web_available && !existingOAuth}>{hostRepositoryText("repository.039")}</option>}</select></AutoField>}
    {provider && (provider === "git" || method === "token") && <>
      <AutoField label={<span className="text-sm font-medium">{hostRepositoryText("repository.038")}</span>} className="block space-y-2"><SelectRefresh label={hostRepositoryText("repository.052")} loading={loadingCredentials} onRefresh={() => setRefresh(value => value + 1)}><select aria-label={hostRepositoryText("repository.038")} className={selectClass} value={credentialId} disabled={loadingCredentials} onChange={event => onChange({ credential_id: event.target.value })}><option value="0">{provider === "git" ? hostRepositoryText("repository.040") : hostRepositoryText("repository.047")}</option>{hasCredential && !availableCredentials.some(row => row.id === credentialId) && <option value={credentialId}>{hostRepositoryText("repository.048")}{credentialId}</option>}{availableCredentials.map(row => <option key={row.id} value={row.id}>{row.name}</option>)}</select></SelectRefresh></AutoField>
      {loadingCredentials ? !availableCredentials.length && <LoadingSkeleton variant="inline" label={hostRepositoryText("repository.049")} /> : !availableCredentials.length && !credentialError && <p role="status" className="text-sm text-muted-foreground">{hostRepositoryText("repository.050")}</p>}
      <a className="text-sm text-primary underline underline-offset-4" href={credentialHref ?? navigation?.href({ workspace, group: "repository", item: "credentials", mode: "new" }, { scope: "build" })} target="_blank" rel="noopener noreferrer">{hostRepositoryText("repository.051")}</a>
      <p className="text-xs text-muted-foreground">{provider === "git" ? hostRepositoryText("repository.053") : hostRepositoryText("repository.054")}</p>
      {provider === "git" ? <AutoField label={<span className="text-sm font-medium">{hostRepositoryText("repository.055")}</span>} className="block space-y-2"><Input aria-label={hostRepositoryText("repository.055")} value={String(draft.url ?? "")} placeholder={hostRepositoryText("repository.056")} onChange={event => onChange({ url: event.target.value })} /></AutoField> : <>
        {provider === "codeup" && <AutoField label={<span className="text-sm font-medium">{hostRepositoryText("repository.057")}</span>} className="block space-y-2"><Input aria-label={hostRepositoryText("repository.057")} value={String(draft.organization ?? "")} onChange={event => onChange({ organization: event.target.value })} /><span className="block text-xs text-muted-foreground">{hostRepositoryText("repository.058")}</span></AutoField>}
        <details open={provider === "gitea" || !!draft.base_url || !!draft.api_url}><summary className="cursor-pointer text-sm">{hostRepositoryText("repository.059")}{provider === "gitea" ? hostRepositoryText("repository.060") : hostRepositoryText("repository.061")}</summary><div className="mt-3 grid gap-4 sm:grid-cols-2">
          <AutoField label={<span className="text-sm">{hostRepositoryText("repository.041")}</span>} className="space-y-2"><Input aria-label={hostRepositoryText("repository.041")} required={provider === "gitea"} placeholder={hostRepositoryText("repository.062")} value={String(draft.base_url ?? "")} onChange={event => onChange({ base_url: event.target.value })} /></AutoField>
          <AutoField label={<span className="text-sm">{hostRepositoryText("repository.042")}</span>} className="space-y-2"><Input aria-label={hostRepositoryText("repository.042")} placeholder={hostRepositoryText("repository.063")} value={String(draft.api_url ?? "")} onChange={event => onChange({ api_url: event.target.value })} /></AutoField>
        </div></details>
      </>}
    </>}
    {provider && provider !== "git" && method === "web" && <div className="space-y-3">
      <p className="text-sm text-muted-foreground">{hostRepositoryText("repository.064")}</p>
      {!String(draft.name ?? "").trim() && <p className="text-sm text-muted-foreground">{hostRepositoryText("repository.065")}</p>}
      <Button type="button" variant="outline" disabled={loadingPlatform || waiting || !platform?.web_available || !String(draft.name ?? "").trim()} onClick={() => void authorize()}>{waiting ? hostRepositoryText("repository.066") : existingOAuth ? hostRepositoryText("repository.067") : hostRepositoryText("repository.068")}</Button>
      {waiting && <Button type="button" variant="ghost" onClick={() => { closeAuthorization(); setWaiting(false); }}>{hostRepositoryText("repository.069")}</Button>}
    </div>}
    {provider && provider !== "git" && loadingPlatform && <LoadingSkeleton variant="inline" label={hostRepositoryText("repository.070")} />}
    {provider && provider !== "git" && !loadingPlatform && !platform?.web_available && !platformError && <p className="text-xs text-muted-foreground">{platform?.setup || hostRepositoryText("repository.071")}</p>}
    {[credentialError, platformError, authorizationError].filter(Boolean).map((error, index) => <p key={index} role="alert" className="text-sm text-destructive">{error}</p>)}
  </div>;
}
