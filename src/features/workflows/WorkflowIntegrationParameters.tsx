import { FieldCaption } from "../../shared/forms/FieldHelp";
import { Dialog } from "radix-ui";
import { RefreshCw, X } from "lucide-react";
import { useContext, useEffect, useRef, useState } from "react";
import { api, errorText } from "../../shared/api/api";
import { listResource } from "../../shared/api/resource";
import { AutoForm } from "../../shared/forms/auto-form";
import { commonText, workflowText as t } from "../../shared/i18n/i18n";
import { Button } from "../../shared/ui/button";
import { Input } from "../../shared/ui/input";
import { RepositoryConnectionSelect, useConnectionRepositories } from "../repositories/ConnectionRepositoryFields";
import { WorkflowAssetContext } from "./WorkflowAssets";
import { RepositoryMultiSelect, type CloneRepository } from "../repositories/RepositoryMultiSelect";
import {
  integrationText,
  type IntegrationField,
  type IntegrationManifest,
} from "./integrations";

const control =
  "h-10 w-full min-w-0 rounded-md border border-input bg-background px-3 text-sm";
type Params = Record<string, unknown>;
type Credential = { id: string; name: string; kind: string; username: string };

function Field({
  name,
  spec,
  value,
  required,
  credential,
  readOnly,
  change,
}: {
  name: string;
  spec: IntegrationField;
  value: unknown;
  required: boolean;
  credential: string;
  readOnly: boolean;
  change: (value: unknown) => void;
}) {
  const { workspace } = useContext(WorkflowAssetContext);
  const [options, setOptions] = useState<{ value: string; label: string }[]>(
    [],
  );
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const lookup = useRef<AbortController | null>(null);
  const label = integrationText(spec.title_key);
  useEffect(() => {
    lookup.current?.abort();
    setOptions([]);
    setError("");
    setBusy(false);
    return () => lookup.current?.abort();
  }, [credential, name]);
  const update = (raw: string) =>
    change(
      raw === "" && !required
        ? undefined
        : ["integer", "number"].includes(spec.type) &&
            !raw.startsWith("=") &&
            raw !== ""
          ? Number(raw)
          : raw,
    );
  return (
    <div className="min-w-0 space-y-2">
      <label className="block space-y-2 text-sm">
        <span className="block font-medium"><FieldCaption label={<>
          {label}
          {required ? ` · ${t("required")}` : ""}
        </>} help={name === "directory" ? t("integration.gitDirectoryHelp") : undefined} /></span>
        {spec.type === "boolean" ? (
          <select className={control} aria-label={label} disabled={readOnly} value={value === undefined ? "" : String(value)} onChange={event => change(event.target.value === "" ? undefined : event.target.value === "true")}>
            <option value="">{t("selectValue")}</option>
            <option value="true">{t("integration.enum.true")}</option>
            <option value="false">{t("integration.enum.false")}</option>
          </select>
        ) : spec.enum ? (
          <select
            className={control}
            aria-label={label}
            disabled={readOnly}
            value={String(value ?? "")}
            onChange={(event) => update(event.target.value)}
          >
            <option value="">{t("selectValue")}</option>
            {spec.enum.map((option) => (
              <option key={option} value={option}>
                {spec.title_key.startsWith("literal:") ? option : integrationText(`integration.enum.${option}`)}
              </option>
            ))}
          </select>
        ) : spec.control === "textarea" ? (
          <textarea
            className={`${control} min-h-28 py-2`}
            aria-label={label}
            value={String(value ?? "")}
            required={required}
            readOnly={readOnly}
            maxLength={spec.maxLength}
            onChange={(event) => update(event.target.value)}
          />
        ) : (
          <Input
            aria-label={label}
            value={String(value ?? "")}
            required={required}
            readOnly={readOnly}
            maxLength={spec.maxLength}
            inputMode={spec.type === "integer" ? "numeric" : undefined}
            onChange={(event) => update(event.target.value)}
          />
        )}
      </label>
      {spec.options_source && !readOnly && (
        <div className="space-y-2">
          <div className="flex flex-wrap items-center gap-2">
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={busy || !credential || credential === "0"}
              aria-label={`${t("integration.loadOptions")} · ${label}`}
              onClick={async () => {
                const controller = new AbortController();
                lookup.current?.abort();
                lookup.current = controller;
                setBusy(true);
                setError("");
                try {
                  const source = spec.options_source!;
                  const result = await api<Params>(
                    `/workspaces/${workspace}/workflow-integration-options`,
                    {
                      method: "POST",
                      body: JSON.stringify({
                        node_type: source.type,
                        credential_id: credential,
                      }),
                      signal: controller.signal,
                    },
                  );
                  let items: unknown = result;
                  for (const segment of source.items.split("/").filter(Boolean))
                    items = (items as Params)?.[segment];
                  if (!Array.isArray(items))
                    throw new Error(t("integration.failure.remote"));
                  if (controller.signal.aborted) return;
                  setOptions(
                    items
                      .filter(
                        (item) =>
                          item &&
                          typeof item === "object" &&
                          item[source.value] !== undefined,
                      )
                      .map((item) => ({
                        value: String(item[source.value]),
                        label: String(item[source.label] ?? item[source.value]),
                      })),
                  );
                } catch (cause) {
                  if (!controller.signal.aborted) setError(errorText(cause));
                } finally {
                  if (!controller.signal.aborted) setBusy(false);
                }
              }}
            >
              <RefreshCw className={busy ? "animate-spin" : ""} />
              {busy
                ? commonText("common.loading")
                : t("integration.loadOptions")}
            </Button>
            {!!options.length && (
              <select
                className={`${control} flex-1`}
                aria-label={`${label} · ${t("selectValue")}`}
                value=""
                onChange={(event) => change(event.target.value)}
              >
                <option value="">{t("selectValue")}</option>
                {options.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label} · {option.value}
                  </option>
                ))}
              </select>
            )}
          </div>
          <p className="text-xs leading-5 text-muted-foreground">
            {t("integration.optionsHelp")}
          </p>
          {error && (
            <p role="alert" className="text-sm text-destructive">
              {error}
            </p>
          )}
        </div>
      )}
    </div>
  );
}

export function WorkflowIntegrationParameters({
  nodeId,
  spec,
  value,
  readOnly,
  change,
}: {
  nodeId?: string;
  spec: IntegrationManifest;
  value: Params;
  readOnly: boolean;
  change: (value: Params) => void;
}) {
  const { workspace, legacyGitNodes, canCreateRepositoryConnection } = useContext(WorkflowAssetContext);
  const [credentials, setCredentials] = useState<Credential[]>([]);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(false);
  const [revision, setRevision] = useState(0);
  const [open, setOpen] = useState(false);
  const [token, setToken] = useState("");
  const [name, setName] = useState("");
  const profile = spec.ui.application;
  const repositoryIntegration = ["github", "gitlab", "codeup"].includes(profile);
  const gitNetwork = !!spec.git && ["clone", "remote_refs", "fetch", "pull", "push"].includes(spec.git.operation);
  const clone = spec.git?.operation === "clone";
  const batch = clone && Array.isArray(value.repositories);
  const independent = !value.connection_id && value.credential_id !== undefined;
  const legacyGit = gitNetwork && !!nodeId && legacyGitNodes?.has(nodeId) && !value.connection_id && !value.auth_mode;
  const connectionMode = (repositoryIntegration || gitNetwork) && !independent && !legacyGit && value.auth_mode !== "anonymous";
  const repositories = useConnectionRepositories(workspace, String(value.connection_id ?? ""), connectionMode, batch);
  useEffect(() => {
    if (spec.git || connectionMode) return;
    const controller = new AbortController();
    setLoading(true);
    setError("");
    setCredentials([]);
    void (async () => {
      const rows: Credential[] = [];
      for (let page = 1; ; page++) {
        const result = await listResource<Credential>(
          workspace,
          "credentials",
          { scope: "integration", page, page_size: 200 },
          controller.signal,
        );
        rows.push(
          ...result.items.filter(
            (row) => row.kind === "token" && row.username === profile,
          ),
        );
        if (page * result.page_size >= result.total) break;
      }
      if (!controller.signal.aborted) setCredentials(rows);
    })()
      .catch((cause) => {
        if (!controller.signal.aborted) setError(errorText(cause));
      })
      .finally(() => {
        if (!controller.signal.aborted) setLoading(false);
      });
    return () => controller.abort();
  }, [workspace, profile, revision, connectionMode]);
  const patch = (key: string, next: unknown) => {
    const result = { ...value };
    if (next === undefined) delete result[key];
    else result[key] = next;
    change(result);
  };
  const field = (key: string, definition: IntegrationField) => (
    <Field
      key={`${profile}:${key}`}
      name={key}
      spec={key === "directory" && batch ? {...definition,title_key:"integration.cloneRootDirectory"} : definition}
      value={value[key]}
      required={spec.params_schema.required.includes(key) || (key === "organization_id" && repositoryIntegration && !connectionMode)}
      credential={String(value.credential_id ?? "0")}
      readOnly={readOnly || (repositoryIntegration && independent && key === "organization_id")}
      change={(next) => patch(key, next)}
    />
  );
  const fields = Object.entries(spec.params_schema.properties).filter(
    ([key]) => !["credential_id", "connection_id", "auth_mode", "repositories"].includes(key) && !(connectionMode && (key === "organization_id" || (gitNetwork && key === "url"))),
  );
  const required = fields.filter(([key]) =>
    spec.params_schema.required.includes(key),
  );
  const optional = fields.filter(
    ([key]) => !spec.params_schema.required.includes(key),
  );
  return (
    <div className="space-y-5">
      {gitNetwork && !spec.git?.provider && !readOnly && <label className="block space-y-2 text-sm">
        <span><FieldCaption label={t("integration.gitAuthentication")} /></span>
        <select className={control} aria-label={t("integration.gitAuthentication")} value={legacyGit?"legacy":value.auth_mode === "anonymous" ? "anonymous" : "connection"} onChange={event=>{
          const next={...value};delete next.connection_id;delete next.credential_id;delete next.repositories;
          next.auth_mode=event.target.value;
          change(next);
        }}>{legacyGit&&<option value="legacy" disabled>{t("integration.legacyAuth")}</option>}<option value="connection">{t("integration.useConnection")}</option><option value="anonymous">{t("integration.anonymousGit")}</option></select>
      </label>}
      {gitNetwork && value.auth_mode === "anonymous" && <p className="text-xs text-muted-foreground">{t("integration.anonymousGitHelp")}</p>}
      {connectionMode && <div className="space-y-3">
        <RepositoryConnectionSelect workspace={workspace} data={repositories} value={String(value.connection_id ?? "")} readOnly={readOnly} canCreate={canCreateRepositoryConnection!==false} {...(!gitNetwork || spec.git?.provider ? {provider:spec.git?.provider ?? profile} : {})} onChange={(id,created)=>{
          const next = {...value};
          for (const key of ["credential_id", "organization_id", "auth_mode", "owner", "repo", "project", "repository_id", ...(gitNetwork ? ["url", "repositories"] : [])]) delete next[key];
          if (id) next.connection_id = id; else delete next.connection_id;
          const connection = created??repositories.connections.find(row=>row.id===id);
          if (gitNetwork && connection?.provider==='git' && connection.url) next.url=connection.url;
          change(next);
        }}/>
        <p className="text-xs leading-5 text-muted-foreground">{t("integration.connectionHelp")}</p>
        {repositories.error && <p role="alert" className="text-sm text-destructive">{repositories.error}</p>}
        {clone && !!value.connection_id && repositories.connection?.provider !== "git" && <label className="flex min-h-11 items-center gap-2 text-sm"><input type="checkbox" className="size-4 accent-primary" disabled={readOnly} checked={batch} onChange={event => {
          const next = {...value};
          if (event.target.checked) next.repositories = repositories.repositories.filter(row => row.url === value.url).map(({url,name,full_name}) => ({url,name,full_name}));
          else delete next.repositories;
          change(next);
        }}/>{t("actions.cloneMultiple")}</label>}
        {batch && <>
          <p className="text-xs leading-5 text-muted-foreground">{t("integration.cloneBatchHelp")}</p>
          {readOnly ? <ul className="space-y-1 text-sm">{(value.repositories as CloneRepository[]).map(row => <li key={row.url} className="break-all">{row.full_name || row.name}</li>)}</ul> : <RepositoryMultiSelect rows={repositories.repositories} selected={value.repositories as CloneRepository[]} disabled={repositories.loading || !!repositories.error} change={rows => change({...value,repositories:rows.map(({url,name,full_name})=>({url,name,...(full_name?{full_name}:{})})),url:rows[0]?.url ?? ""})} />}
        </>}
        {!!value.connection_id && !batch && !readOnly && repositories.connection?.provider !== "git" && !spec.type.endsWith("repository.list") && <label className="block space-y-2 text-sm">
          <span className="block font-medium"><FieldCaption label={t("integration.chooseRepository")} /></span>
          <select className={control} aria-label={t("integration.chooseRepository")} value={gitNetwork ? String(value.url ?? "") : ""} required={gitNetwork} disabled={repositories.loading} onChange={event => {
            const remote = repositories.repositories.find(row => row.url === event.target.value);
            if (!remote) return;
            const next = {...value};
            if (gitNetwork) next.url = remote.url;
            else if (profile === "github") {
              const parts = remote.full_name.split('/'); next.owner = parts[0]; next.repo = parts.slice(1).join('/');
            } else if (profile === "gitlab") next.project = remote.remote_id || remote.full_name;
            else next.repository_id = remote.remote_id || remote.full_name;
            change(next);
          }}>
            <option value="">{repositories.loading ? commonText("common.loading") : t("integration.chooseRepository")}</option>
            {gitNetwork && !!value.url && !repositories.repositories.some(row => row.url === value.url) && <option value={String(value.url)}>{String(value.url)}</option>}
            {repositories.repositories.map(row => <option key={row.url} value={row.url}>{row.full_name || row.name}</option>)}
          </select>
          {repositories.hasMore && <Button type="button" variant="outline" disabled={repositories.loading} onClick={() => repositories.setPage(page => page + 1)}>{t("integration.loadOptions")}</Button>}
        </label>}
        {gitNetwork && !batch && !!value.url && <div className="min-w-0 space-y-1 text-sm"><span className="font-medium">{t("integration.gitUrl")}</span><p className="break-all text-muted-foreground">{String(value.url)}</p></div>}
      </div>}
      {repositoryIntegration && independent && <div className="space-y-2 rounded-md border p-3">
        <p className="text-sm text-muted-foreground">{t("integration.legacyAuth")}</p>
        {!readOnly && <Button type="button" variant="outline" onClick={()=>{const next={...value};delete next.credential_id;delete next.organization_id;change(next);}}>{t("integration.useConnection")}</Button>}
      </div>}
      {legacyGit && <p className="text-sm text-muted-foreground">{t("integration.legacyAuth")}</p>}
      {!spec.git && !connectionMode && <label className="block space-y-2 text-sm">
        <span className="block font-medium"><FieldCaption label={<>{t("integration.credential")}</>} /></span>
        <select
          className={control}
          aria-label={t("integration.credential")}
          value={String(value.credential_id ?? "0")}
          disabled={readOnly || loading || repositoryIntegration}
          onChange={(event) => patch("credential_id", event.target.value)}
        >
          <option value="0">
            {loading
              ? commonText("common.loading")
              : spec.http?.auth.optional
                ? t("integration.publicAccess")
                : t("selectValue")}
          </option>
          {value.credential_id &&
          value.credential_id !== "0" &&
          !credentials.some((row) => row.id === value.credential_id) ? (
            <option value={String(value.credential_id)}>
              {commonText("form.unavailableReference", {
                id: String(value.credential_id),
              })}
            </option>
          ) : null}
          {credentials.map((row) => (
            <option key={row.id} value={row.id}>
              {row.name}
            </option>
          ))}
        </select>
      </label>}
      {!spec.git && !repositoryIntegration && !connectionMode && !readOnly && (
        <div className="flex flex-wrap gap-2">
          <Button
            type="button"
            variant="outline"
            onClick={() => {
              setError("");
              setName(spec.ui.application_name);
              setOpen(true);
            }}
          >
            {t("integration.createCredential")}
          </Button>
          <Button
            type="button"
            variant="ghost"
            size="icon"
            disabled={loading}
            aria-label={t("integration.refresh")}
            onClick={() => setRevision(revision + 1)}
          >
            <RefreshCw className={loading ? "animate-spin" : ""} />
          </Button>
        </div>
      )}
      {!connectionMode && <p className="text-xs leading-5 text-muted-foreground">
        {integrationText(spec.ui.credential_help_key)}
      </p>}
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      {required.map(([key, definition]) => field(key, definition))}
      {!!optional.length && (
        <details className="space-y-4 border-t pt-4">
          <summary className="cursor-pointer text-sm font-medium">
            {t("integration.optional")}
          </summary>
          {optional.map(([key, definition]) => field(key, definition))}
        </details>
      )}
      {(value.page !== undefined || value.limit !== undefined) && (
        <p className="text-xs leading-5 text-muted-foreground">
          {t("integration.paginationHelp")}
        </p>
      )}
      <Dialog.Root
        open={open}
        onOpenChange={(next) => {
          if (!busy) {
            setOpen(next);
            if (!next) setToken("");
          }
        }}
      >
        <Dialog.Portal>
          <Dialog.Overlay className="fixed inset-0 z-50 bg-black/30" />
          <Dialog.Content className="fixed left-1/2 top-1/2 z-50 max-h-[90dvh] w-[calc(100%_-_2rem)] max-w-lg -translate-x-1/2 -translate-y-1/2 dialog-shell rounded-xl border bg-background p-6 shadow-xl">
            <div className="flex items-start justify-between gap-3">
              <Dialog.Title className="text-lg font-semibold">
                {spec.ui.application_name} · {t("integration.createCredential")}
              </Dialog.Title>
              <Dialog.Close asChild>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  disabled={busy}
                  aria-label={t("close")}
                >
                  <X />
                </Button>
              </Dialog.Close>
            </div>
            <Dialog.Description className="my-4 text-sm leading-6 text-muted-foreground">
              {t("integration.credentialHelp")}
            </Dialog.Description>
            <AutoForm
              className="dialog-form"
              onSubmit={async (event) => {
                event.preventDefault();
                setBusy(true);
                setError("");
                try {
                  const row = await api<{ id: string }>(
                    `/workspaces/${workspace}/credentials`,
                    {
                      method: "POST",
                      body: JSON.stringify({
                        name,
                        secret: token,
                        kind: "token",
                        scope: "integration",
                        username: profile,
                        description: spec.ui.application_name,
                      }),
                    },
                  );
                  setCredentials((current) => [
                    ...current,
                    {
                      id: row.id,
                      name: name.trim(),
                      kind: "token",
                      username: profile,
                    },
                  ]);
                  patch("credential_id", row.id);
                  setToken("");
                  setOpen(false);
                } catch (cause) {
                  setError(errorText(cause));
                } finally {
                  setBusy(false);
                }
              }}
            >
              <div className="dialog-body space-y-4">
              <label className="block space-y-2 text-sm">
                <span><FieldCaption label={<>{t("integration.name")}</>} /></span>
                <Input
                  aria-label={t("integration.name")}
                  value={name}
                  required
                  maxLength={60}
                  disabled={busy}
                  onChange={(event) => setName(event.target.value)}
                />
              </label>
              <label className="block space-y-2 text-sm">
                <span><FieldCaption label={<>{t("integration.token")}</>} /></span>
                <Input
                  type="password"
                  autoComplete="new-password"
                  aria-label={t("integration.token")}
                  value={token}
                  required
                  maxLength={65536}
                  disabled={busy}
                  onChange={(event) => setToken(event.target.value)}
                />
              </label>
              {error && (
                <p role="alert" className="text-sm text-destructive">
                  {error}
                </p>
              )}
              </div>
              <div className="dialog-footer mt-4">
                <Button
                  type="button"
                  variant="outline"
                  disabled={busy}
                  onClick={() => {
                    setToken("");
                    setOpen(false);
                  }}
                >
                  {t("cancel")}
                </Button>
                <Button type="submit" disabled={busy}>
                  {busy
                    ? commonText("common.loading")
                    : commonText("common.create")}
                </Button>
              </div>
            </AutoForm>
          </Dialog.Content>
        </Dialog.Portal>
      </Dialog.Root>
    </div>
  );
}
