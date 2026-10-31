/**
 * The one form implementation, in three modes driven by the route:
 *
 *   new   create, every field editable
 *   edit  update, `immutable` fields locked, `version` sent for the
 *         optimistic-concurrency check
 *   view  the detail page — the same configuration rendered read-only
 *
 * Read-only renders values as text rather than disabled inputs: a disabled
 * input is low-contrast, awkward to read and awkward to copy out of. A
 * `secret` field never shows a stored value in any mode, only whether one is
 * set (§8 of the design doc).
 */
import { useCallback, useEffect, useState } from "react";
import { Dialog } from "radix-ui";
import { ActionsEditor } from "./ActionsEditor";
import { MarketplaceRepository, PipelineRepository } from "./PipelineRepository";
import { RepositorySource } from "./RepositorySource";
import { RunnerConnection } from "./RunnerConnection";
import { DeleteConfirmDialog } from "./DeleteConfirmDialog";
import { ManifestEditor } from "./ManifestEditor";
import { CronEditor } from "./CronEditor";
import { ArrowLeft, Download, Loader2, Play, Save, SlidersHorizontal, Upload } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { cn } from "cn";
import { errorText } from "../api";
import { createResource, getResource, patchResource } from "../resource";
import { href, navigate, useRoute } from "../router";
import type { FormConfig, FormField, Option, Options, RowAction } from "./dsl";

function useOptions(source: Options | undefined) {
  const [revision, setRevision] = useState(0);
  const reload = useCallback(() => setRevision(value => value + 1), []);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(typeof source === "function");
  const [options, setOptions] = useState<Option[]>(Array.isArray(source) ? source : []);
  useEffect(() => {
    if (typeof source !== "function") {
      setOptions(Array.isArray(source) ? source : []);
      setError(""); setLoading(false);
      return;
    }
    let live = true;
    setError(""); setLoading(true);
    void source().then((loaded) => live && setOptions(loaded)).catch(cause => live && setError(errorText(cause))).finally(() => live && setLoading(false));
    return () => {
      live = false;
    };
  }, [source, revision]);
  return { options, error, loading, reload };
}

export function ResourceForm<T extends Record<string, unknown>>({
  config,
  workspace,
  permissions,
  onSaved,
  createInPlace = false,
  onCancel,
  pipelineWorkspace = false,
  showViewNavigation = true,
}: {
  config: FormConfig<T>;
  workspace: string;
  permissions: Set<string>;
  onSaved?: () => void;
  createInPlace?: boolean;
  onCancel?: () => void;
  pipelineWorkspace?: boolean;
  showViewNavigation?: boolean;
}) {
  const pageRoute = useRoute();
  const route = createInPlace ? { ...pageRoute, mode: "new" as const } : pageRoute;
  const creating = route.mode === "new";
  const copyId = creating && !createInPlace ? new URLSearchParams(location.search).get("copy") : null;
  const blankDraft = creating && !createInPlace && config.resource === "alert-rules" && new URLSearchParams(location.search).get("target_kind") === "host"
    ? { ...config.blank, target_kind: "host", target_ref: new URLSearchParams(location.search).get("target_ref") ?? "" } as Partial<T>
    : config.blank;
  const [draft, setDraft] = useState<Partial<T>>(blankDraft);
  const protectedRow = !creating && !!config.readOnly?.(draft);
  const readOnly = route.mode === "view" || protectedRow;
  const [loading, setLoading] = useState(!creating || !!copyId);
  const [saving, setSaving] = useState(false);
  const [pendingAction, setPendingAction] = useState<RowAction<T>>();
  const [pendingDelete, setPendingDelete] = useState<RowAction<T>>();
  const [pendingActionRow, setPendingActionRow] = useState<T>();
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [templateDialog, setTemplateDialog] = useState<"import" | "export" | null>(() => new URLSearchParams(location.search).has("template") && pipelineWorkspace ? "import" : null);
  const [templateName, setTemplateName] = useState("");
  const [templateDescription, setTemplateDescription] = useState("");
  const [templateBusy, setTemplateBusy] = useState(false);
  const [templateError, setTemplateError] = useState("");
  const [editorMode, setEditorMode] = useState<"visual" | "yaml">("visual");
  const [unavailableOptions, setUnavailableOptions] = useState<Record<string, boolean>>({});
  const reportOptions = useCallback((key: string, blocked: boolean) => {
    setUnavailableOptions(current => current[key] === blocked ? current : { ...current, [key]: blocked });
  }, []);
  const optionsBlocked = Object.values(unavailableOptions).some(Boolean);
  const [error, setError] = useState("");

  useEffect(() => {
    const id = creating ? (config.copy ? copyId : null) : route.id;
    if (!id) { setDraft(blankDraft); setLoading(false); return; }
    let live = true;
    setLoading(true);
    getResource<T>(workspace, config.resource, id, config.endpoint)
      .then((row) => {
        if (config.resource === "credentials" && row.scope !== config.blank.scope) throw new Error("此授权认证属于其他模块，请从对应模块进入。");
        if (live) { setDraft(creating && config.copy ? config.copy(row) : row); setError(""); }
      })
      .catch((cause: unknown) => live && setError(errorText(cause)))
      .finally(() => live && setLoading(false));
    return () => {
      live = false;
    };
  }, [creating, copyId, route.id, workspace, config]);

  useEffect(() => {
    const delay=route.mode === "view" ? config.pollMs?.(draft) : 0;
    if (!delay || !route.id) return;
    let live=true;
    let timer:ReturnType<typeof setTimeout>;
    const load=async()=>{
      try {const value=await getResource<T>(workspace,config.resource,route.id!,config.endpoint);if(live){setDraft(value);setError("");}}
      catch(cause){if(live)setError(errorText(cause));}
      finally {if(live)timer=setTimeout(load,delay);}
    };
    timer=setTimeout(load,delay);
    return ()=>{live=false;clearTimeout(timer);};
  },[route.mode,route.id,workspace,config,draft]);
  const backToList = href({ ...route, mode: "list" }, {});
  const canEdit = permissions.has(`${config.permissionResource ?? config.resource}.${creating ? "create" : "update"}`);
  const workspaceEditor = pipelineWorkspace && config.resource === "pipelines" && (creating || route.mode === "edit");

  const renderField = (field: FormField<T>) => (
    <Field
      key={field.key}
      field={field}
      workspace={workspace}
      reportOptions={reportOptions}
      value={draft[field.key]}
      stored={!!draft[field.stored ?? `has_${field.key}`]}
      creating={creating}
      draft={draft}
      permissions={permissions}
      readOnly={readOnly || (!creating && !!field.immutable)}
      asText={readOnly}
      fillHeight={workspaceEditor && field.type === "actions-editor"}
      {...(workspaceEditor ? { editorMode, onEditorModeChange: setEditorMode } : {})}
      onChange={(value) => setDraft((d) => ({
        ...d,
        ...Object.fromEntries((field.resetOnChange ?? []).map((key) => [key, ""])),
        ...(["repository-source", "pipeline-repository", "marketplace-repository", "actions-editor"].includes(field.type) ? value as Record<string, unknown> : { [field.key]: value }),
      }) as Partial<T>)}
    />
  );

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (readOnly || !canEdit || loading || optionsBlocked) return;
    const runAfterSave = workspaceEditor && (event.nativeEvent as SubmitEvent).submitter?.getAttribute("value") === "save-run";
    setSaving(true);
    setError("");
    let savedId: string | undefined;
    try {
      if (creating) {
        const created = await createResource<{ id: string } | undefined>(workspace, config.resource, draft, config.endpoint);
        savedId = created?.id;
      } else {
        // `version` rides along so the server can refuse a stale write.
        await patchResource(workspace, config.resource, route.id!, draft, config.endpoint);
        savedId = route.id;
      }
      onSaved?.();
      if (createInPlace) return;
      if (runAfterSave && savedId) {
        const action = config.viewActions?.find(candidate => candidate.permission === "pipelines.run");
        if (action && permissions.has("pipelines.run")) {
          const saved = await getResource<T>(workspace, config.resource, savedId, config.endpoint);
          if (action.parameters?.(saved).length) {
            setPendingActionRow(saved);
            setPendingAction(action);
          } else {
            await action.action?.(saved);
          }
          return;
        }
      }
      navigate({ ...route, mode: "list" }, {});
    } catch (cause) {
      if (creating && savedId) {
        toast.error(`流水线已保存，但执行失败：${errorText(cause)}`);
        navigate({ ...route, id: savedId, mode: "edit" }, {});
      } else setError(errorText(cause));
    } finally {
      setSaving(false);
    }
  };

  if (loading)
    return (
      <p className="text-muted-foreground">
        <Loader2 className="mr-2 inline size-4 animate-spin" />
        载入中…
      </p>
    );

  const hostConnectionTabs = creating && config.resource === "hosts" && <div className="sticky top-0 z-10 flex items-center gap-2 border-b bg-background py-2" role="tablist" aria-label="接入方式">
    {([ ["runner", "代理 Runner"], ["ssh", "SSH"] ] as const).map(([connection, label]) => <Button key={connection} type="button" role="tab" aria-selected={draft.connection === connection} variant={draft.connection === connection ? "secondary" : "ghost"} onClick={() => setDraft(current => ({ ...current, connection } as Partial<T>))}>{label}</Button>)}
  </div>;

  if (creating && config.resource === "hosts" && draft.connection === "runner") return <div className="mx-auto w-full max-w-6xl space-y-4 pb-8">
    {hostConnectionTabs}
    {permissions.has("hosts.enroll") ? <RunnerConnection workspace={workspace} inline autoGenerate /> : <p role="alert" className="text-sm text-destructive">当前角色没有代理接入权限。</p>}
  </div>;

  if (workspaceEditor) {
    const fields = config.sections.flatMap(section => section.fields).filter(field => !field.when || field.when(draft));
    const editor = fields.find(field => field.key === "definition");
    const settings = fields.filter(field => field.key !== "name" && field !== editor && field.key !== "application_id" && field.key !== "description");
    const closeAction = () => {
      setPendingAction(undefined);
      if (creating && pendingActionRow && "id" in pendingActionRow)
        navigate({ ...route, id: String(pendingActionRow.id), mode: "edit" }, {});
    };
    return <form className="flex h-full min-h-0 min-w-0 flex-col" onSubmit={submit}>
      <header className="flex flex-wrap items-center gap-2 border-b bg-background px-3 py-2 sm:min-h-14 sm:px-4">
        <Button type="button" variant="ghost" size="icon" aria-label="返回流水线" title="返回流水线" onClick={() => navigate({ ...route, mode: "list" }, {})}><ArrowLeft /></Button>
        <Input aria-label="流水线名称" placeholder="流水线名称" required maxLength={60} className="h-9 w-48 max-w-[calc(100vw-6rem)] shrink-0 border-0 bg-transparent px-2 text-base font-semibold shadow-none focus-visible:ring-0 sm:w-56" value={String(draft.name ?? "")} onChange={event => setDraft(current => ({ ...current, name: event.target.value } as Partial<T>))} />
        <Button type="button" variant="ghost" size="sm" aria-expanded={settingsOpen} onClick={() => setSettingsOpen(open => !open)}><SlidersHorizontal className="size-4" />流水线设置</Button>
        {editor?.type === "actions-editor" && <div className="flex gap-1" role="tablist" aria-label="流水线编辑模式">
          <Button type="button" role="tab" aria-selected={editorMode === "visual"} variant={editorMode === "visual" ? "secondary" : "ghost"} size="sm" onClick={() => setEditorMode("visual")}>可视化</Button>
          <Button type="button" role="tab" aria-selected={editorMode === "yaml"} variant={editorMode === "yaml" ? "secondary" : "ghost"} size="sm" onClick={() => setEditorMode("yaml")}>YAML</Button>
        </div>}
        {config.generator && <Button type="button" variant="ghost" size="sm" onClick={() => setTemplateDialog("import")}><Download />从模板导入</Button>}
        {permissions.has("pipeline-templates.create") && <Button type="button" variant="ghost" size="sm" onClick={() => { setTemplateError(""); setTemplateDialog("export"); }}><Upload />保存为模板</Button>}
        <div className="ml-auto flex gap-2">
          <Button type="submit" variant="outline" disabled={saving || !canEdit || optionsBlocked}>{saving && <Loader2 className="animate-spin" />}<Save />保存</Button>
          {permissions.has("pipelines.run") && <Button type="submit" value="save-run" disabled={saving || !canEdit || optionsBlocked}><Play />保存并执行</Button>}
        </div>
      </header>
      <main className="flex min-h-0 flex-1 flex-col overflow-y-auto px-3 py-3 sm:px-4">
        {error && <p role="alert" className="mb-3 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">{error}</p>}
        {settingsOpen && <div className="mb-3 space-y-4 border-b pb-3">
            <div className="grid gap-4 sm:grid-cols-2">{settings.map(renderField)}</div>
        </div>}
        {editor && <div className="flex min-h-0 flex-1 flex-col">{renderField(editor)}</div>}
      </main>
      {pendingAction && <ResourceActionDialog action={pendingAction} row={pendingActionRow!} workspace={workspace} permissions={permissions} onClose={closeAction} onDone={() => setPendingAction(undefined)} />}
      <Dialog.Root open={templateDialog !== null} onOpenChange={open => { if (!open && !templateBusy) setTemplateDialog(null); }}><Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-50 bg-black/40" />
        <Dialog.Content className="fixed left-1/2 top-1/2 z-50 max-h-[90dvh] w-[min(36rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 overflow-y-auto rounded-lg border bg-background p-6 shadow-lg">
          <Dialog.Title className="text-lg font-semibold">{templateDialog === "import" ? "从模板导入" : "保存为模板"}</Dialog.Title>
          <Dialog.Description className="mt-2 text-sm text-muted-foreground">{templateDialog === "import" ? "选择模板并生成配置到当前流水线。" : "将当前流水线配置保存为可复用模板。"}</Dialog.Description>
          {templateDialog === "import" && config.generator && <div className="mt-4"><FormGenerator config={config.generator} compactWhenUnused={false} listMode report={reportOptions} onGenerated={value => { setDraft(d => ({ ...d, ...value })); setTemplateDialog(null); }} /></div>}
          {templateDialog === "export" && <div className="mt-4 space-y-4">
            <label className="block space-y-2 text-sm">模板名称<Input value={templateName} maxLength={60} onChange={event => setTemplateName(event.target.value)} /></label>
            <label className="block space-y-2 text-sm">说明<Input value={templateDescription} onChange={event => setTemplateDescription(event.target.value)} /></label>
            {templateError && <p role="alert" className="text-sm text-destructive">{templateError}</p>}
            <div className="flex justify-end gap-2"><Button type="button" variant="outline" onClick={() => setTemplateDialog(null)}>取消</Button><Button type="button" disabled={templateBusy || !templateName.trim()} onClick={async () => {
              setTemplateBusy(true); setTemplateError("");
              try {
                const content = draft.definition_format === "github-actions" ? JSON.stringify({ engine: "github-actions", workflow: draft.definition, executor_bindings: draft.executor_bindings ?? [], checkout_bindings: draft.checkout_bindings ?? [], graph_layout: draft.graph_layout ?? {} }) : String(draft.definition ?? "");
                await createResource(workspace, "pipeline-templates", { name: templateName.trim(), description: templateDescription, content, parameters: "[]" });
                setTemplateDialog(null); toast.success("模板已保存");
              }
              catch (cause) { setTemplateError(errorText(cause)); }
              finally { setTemplateBusy(false); }
            }}>{templateBusy ? "保存中…" : "保存模板"}</Button></div>
          </div>}
        </Dialog.Content>
      </Dialog.Portal></Dialog.Root>
    </form>;
  }

  return (
    <form className={cn("min-w-0 w-full pb-8", readOnly ? "resource-detail" : ["space-y-6", config.width !== "full" && "mx-auto max-w-6xl"])} onSubmit={submit}>
      {hostConnectionTabs}
      {error && (
        <p className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {error}
        </p>
      )}

      {readOnly && showViewNavigation && <div className="flex flex-wrap items-center gap-2 border-b pb-4">
        {!protectedRow && permissions.has(`${config.permissionResource ?? config.resource}.update`) && route.id && <Button asChild size="sm"><a href={href({ ...route, mode: "edit" }, {})}>编辑</a></Button>}
      </div>}

      {pendingAction && <ResourceActionDialog action={pendingAction} row={draft as T} workspace={workspace} permissions={permissions} onClose={() => setPendingAction(undefined)} onDone={() => { setPendingAction(undefined); if(route.id) void getResource<T>(workspace,config.resource,route.id,config.endpoint).then(setDraft).catch(cause=>setError(errorText(cause))); }} />}
      {pendingDelete && <DeleteConfirmDialog action={pendingDelete.label} target={String(draft.name ?? draft.title ?? draft.email ?? route.id ?? "当前记录")} onClose={() => setPendingDelete(undefined)} onConfirm={async () => {
        if (pendingDelete.parameters?.(draft as T).length) { setPendingAction(pendingDelete); return; }
        setSaving(true); setError("");
        try { await pendingDelete.action?.(draft as T); if (route.id) setDraft(await getResource<T>(workspace, config.resource, route.id, config.endpoint)); }
        finally { setSaving(false); }
      }} />}
      {creating && config.generator && <FormGenerator config={config.generator} compactWhenUnused={config.resource === "pipelines"} report={reportOptions} onGenerated={value => setDraft(d => ({ ...d, ...value }))} />}

      {readOnly && config.viewActions && <div className="flex flex-wrap gap-2">{config.viewActions.filter(action=>(!action.permission || permissions.has(action.permission)) && (!action.when || action.when(draft as T))).map(action=>action.to ? <Button key={action.label} variant="outline" asChild><a href={href({...route,...action.to(draft as T)},action.query?.(draft as T))}>{action.label}</a></Button> : <Button key={action.label} type="button" variant="outline" disabled={saving} onClick={async()=>{
        if(action.destructive){setPendingDelete(action);return;}
        if(action.parameters?.(draft as T).length){setPendingAction(action);return;}
        setSaving(true);setError("");
        try {await action.action?.(draft as T);if(route.id)setDraft(await getResource<T>(workspace,config.resource,route.id,config.endpoint));}
        catch(cause){setError(errorText(cause));}finally{setSaving(false);}
      }}>{action.label}</Button>)}</div>}
      {config.sections.map((section, index) => {
        const fields = section.fields.filter((field) => !(creating && config.resource === "hosts" && field.key === "connection") && (!field.when || field.when(draft)));
        if (!fields.length) return null;
        return (
          <section key={section.title ?? index} aria-label={section.title ?? (readOnly ? index === 0 ? "基本信息" : "其他信息" : undefined)} className={readOnly ? cn("detail-section", fields.some(field => ["code", "manifest", "actions-editor"].includes(field.type)) && "col-span-full") : "space-y-4"}>
            {(section.title || readOnly) && (
              <h2 className={readOnly ? "detail-section-title" : "mb-2 text-sm font-medium text-muted-foreground"}>
                {section.title ?? (index === 0 ? "基本信息" : "其他信息")}
              </h2>
            )}
            <div className={readOnly ? "detail-fields grid sm:grid-cols-2" : "grid gap-4 sm:grid-cols-2"}>
              {fields.map(renderField)}
            </div>
          </section>
        );
      })}

      {!readOnly && <div className="flex gap-2">
            <Button type="submit" disabled={saving || !canEdit || optionsBlocked}>
              {saving && <Loader2 className="animate-spin" />}
              {creating ? config.resource === "hosts" ? "添加主机" : "创建" : "保存"}
            </Button>
            {onCancel ? <Button type="button" variant="outline" onClick={onCancel} disabled={saving}>取消</Button> : <Button type="button" variant="outline" asChild>
              <a href={backToList}>取消</a>
            </Button>}
      </div>}
    </form>
  );
}

function FormGenerator<T>({ config, compactWhenUnused, listMode = false, report, onGenerated }: {
  config: NonNullable<FormConfig<T>["generator"]>;
  compactWhenUnused: boolean;
  listMode?: boolean;
  report: (key: string, blocked: boolean) => void;
  onGenerated: (value: Partial<T>) => void;
}) {
  const [id, setId] = useState(() => new URLSearchParams(location.search).get("template") ?? "");
  const [attempt, setAttempt] = useState(0);
  const [template, setTemplate] = useState<Awaited<ReturnType<typeof config.load>>>();
  const [values, setValues] = useState<Record<string, unknown>>({});
  const [busy, setBusy] = useState(false);
  const [generated, setGenerated] = useState(false);
  const [error, setError] = useState("");
  const { options, error: optionsError, loading: optionsLoading } = useOptions(config.options);
  const ignoreOptions = useCallback(() => {}, []);
  useEffect(() => {
    let live = true;
    setTemplate(undefined); setGenerated(false); setError("");
    if (!id) { setBusy(false); return; }
    setBusy(true);
    void config.load(id).then(value => { if (live) { setTemplate(value); setValues(value.values); } })
      .catch(cause => { if (live) setError(errorText(cause)); })
      .finally(() => { if (live) setBusy(false); });
    return () => { live = false; };
  }, [id, attempt, config]);
  useEffect(() => { report("$generator", !!id && (!generated || busy)); return () => report("$generator", false); }, [id, generated, busy, report]);
  const compact = compactWhenUnused && !id;
  return <section aria-label="从模板生成" className={cn(compact ? "flex flex-wrap items-center gap-3" : listMode ? "space-y-4" : "space-y-4 rounded-md border p-4")}>
    {!listMode && <h2 className={cn("font-medium", compact && "text-sm")}>从模板生成配置</h2>}
    {listMode ? <div className="max-h-64 space-y-1 overflow-y-auto" role="listbox" aria-label="模板列表">
      {optionsLoading && <p className="py-3 text-sm text-muted-foreground">正在加载模板…</p>}
      {!optionsLoading && !options.length && <p className="py-3 text-sm text-muted-foreground">暂无模板</p>}
      {options.map(option => <button key={option.value} type="button" role="option" aria-selected={id === option.value} className={cn("w-full rounded border px-3 py-2 text-left text-sm hover:bg-accent", id === option.value && "border-primary bg-accent")} onClick={() => { setGenerated(false); setId(option.value); }}>{option.label}</button>)}
    </div> : <label className="flex min-w-0 flex-col gap-2 text-sm"><span className={compact ? "sr-only" : undefined}>模板</span>
      <Select value={id || "__none"} disabled={busy || optionsLoading} onValueChange={value => { setGenerated(false); setId(value === "__none" ? "" : value); }}>
        <SelectTrigger className={compact ? "w-56 max-w-full" : undefined}><SelectValue placeholder="选择模板" /></SelectTrigger>
        <SelectContent><SelectItem value="__none">不使用模板</SelectItem>{options.map(o => <SelectItem key={o.value} value={o.value}>{o.label}</SelectItem>)}</SelectContent>
      </Select>
    </label>}
    {(error || optionsError) && <p role="alert" className="text-sm text-destructive">{error || optionsError}</p>}
    {id && <>
      {template && <><p className="text-sm text-muted-foreground">{template.name} · 版本 {template.version}</p>
        <div className="grid gap-4 sm:grid-cols-2">{template.fields.map(field => <Field key={`${id}/${template.version}/${field.key}`} field={field} value={values[field.key]} readOnly={busy} asText={false} stored={false} creating draft={values} permissions={new Set<string>()} reportOptions={ignoreOptions} onChange={value => { setGenerated(false); setValues(current => ({ ...current, [field.key]: value })); }} />)}</div>
      </>}
      <div className="flex gap-2"><Button type="button" disabled={busy || !template} onClick={async () => {
        if (!template) return; setBusy(true); setError("");
        try { const result = await config.generate(id, template.version, values); onGenerated(result); setGenerated(true); }
        catch (cause) { setError(errorText(cause)); setGenerated(false); }
        finally { setBusy(false); }
      }}>{busy ? "处理中…" : "生成配置"}</Button><Button type="button" variant="outline" disabled={busy} onClick={() => setAttempt(n => n + 1)}>重新载入模板</Button></div>
      <p role="status" className="text-sm text-muted-foreground">{generated ? "配置已填入下方编辑器，请检查并确认创建。后续修改模板不会影响已创建对象。" : "填写参数并生成配置后，再确认创建。"}</p>
    </>}
  </section>;
}

function SecretField<T>({ field, value, stored, creating, asText, readOnly, draft, permissions, onChange }: {
  field: FormField<T>; value: unknown; stored: boolean; creating: boolean; asText: boolean;
  readOnly: boolean; draft: Partial<T>; permissions: Set<string>; onChange: (value: unknown) => void;
}) {
  const [revealed, setRevealed] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const canReveal = !!field.reveal && permissions.has(field.reveal.permission);
  if (asText) return <Wrapper field={field}>
    <span className="text-sm font-medium">{field.label}</span>
    <p className="text-sm text-muted-foreground">{stored ? "已设置" : "未设置"}</p>
    {canReveal && stored && <Button type="button" variant="outline" disabled={busy} onClick={async () => {
      if (revealed !== undefined) { setRevealed(undefined); return; }
      setBusy(true); setError("");
      try { setRevealed(await field.reveal!.action(draft)); }
      catch (cause) { setError(errorText(cause)); }
      finally { setBusy(false); }
    }}>{busy ? "正在读取…" : revealed === undefined ? "查看明文（记录审计）" : "隐藏明文"}</Button>}
    {canReveal && revealed !== undefined && <pre aria-label="变量明文" className="whitespace-pre-wrap break-all rounded-md border p-3 font-mono text-sm">{revealed === "" ? "（空字符串）" : revealed}</pre>}
    {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
  </Wrapper>;
  const replacing = creating || value !== undefined;
  return <Wrapper field={field}>
    {!creating && <label className="flex items-center gap-2 text-sm">
      <Checkbox checked={replacing} disabled={readOnly} onCheckedChange={(checked) => onChange(checked ? "" : undefined)} />
      替换{field.label}（不勾选则保留原值）
    </label>}
    {!creating && <p className="text-xs text-muted-foreground">{stored ? "已设置密钥" : "尚未设置密钥"}</p>}
    {field.help && <p className="text-xs text-muted-foreground">{field.help}</p>}
    {replacing && <label className="flex flex-col gap-1.5 text-sm font-medium">
      {field.label}
      <textarea className="min-h-24 rounded-md border border-input bg-transparent px-3 py-2 font-mono text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
        value={String(value ?? "")} onChange={(event) => onChange(event.target.value)} readOnly={readOnly}
        autoComplete="off" spellCheck={false} placeholder={field.placeholder ?? "允许留空；空白和换行会原样保存"} />
    </label>}
  </Wrapper>;
}

function Field<T>({
  field,
  workspace = "",
  value,
  readOnly,
  asText,
  stored,
  creating,
  draft,
  permissions,
  onChange,
  reportOptions,
  fillHeight = false,
  editorMode,
  onEditorModeChange,
}: {
  field: FormField<T>;
  workspace?: string;
  value: unknown;
  readOnly: boolean;
  asText: boolean;
  stored: boolean;
  creating: boolean;
  draft: Partial<T>;
  permissions: Set<string>;
  onChange: (value: unknown) => void;
  reportOptions: (key: string, blocked: boolean) => void;
  fillHeight?: boolean;
  editorMode?: "visual" | "yaml";
  onEditorModeChange?: (mode: "visual" | "yaml") => void;
}) {
  const { options, error: optionsError, loading: optionsLoading, reload: reloadOptions } = useOptions(field.options);
  const [creatingOption, setCreatingOption] = useState(false);
  const [optionSearch, setOptionSearch] = useState("");
  useEffect(() => {
    reportOptions(field.key, optionsLoading || !!optionsError);
    return () => reportOptions(field.key, false);
  }, [field.key, optionsLoading, optionsError, reportOptions]);
  const reportManifest = useCallback((blocked: boolean) => reportOptions(`$manifest:${field.key}`, blocked), [field.key, reportOptions]);
  const reportCheckout = useCallback((blocked: boolean) => reportOptions(`$checkout:${field.key}`, blocked), [field.key, reportOptions]);
  const text = String(value ?? "");
  if (optionsError) return <Wrapper field={field}><p role="alert" className="text-destructive">{field.label}：{optionsError}，请刷新页面重试。</p></Wrapper>;

  const label = (
    <span className="text-sm font-medium">
      {field.label}
      {field.required && !asText && <span className="ml-0.5 text-destructive">*</span>}
    </span>
  );

  if (field.type === "secret" && (asText || field.replaceOnEdit))
    return <SecretField field={field} value={value} stored={stored} creating={creating}
      asText={asText} readOnly={readOnly} draft={draft} permissions={permissions} onChange={onChange} />;

  if (optionsLoading) return <Wrapper field={field}><p role="status" className="text-sm text-muted-foreground">正在载入{field.label}选项…</p></Wrapper>;

  if (field.type === "actions-editor") return <div className={cn("min-w-0 sm:col-span-2", fillHeight && "flex min-h-0 flex-1 flex-col")}><ActionsEditor draft={draft} options={options} readOnly={readOnly} fillHeight={fillHeight} {...(editorMode && onEditorModeChange ? { mode: editorMode, onModeChange: onEditorModeChange } : {})} onChange={onChange} onValidityChange={reportManifest} renderCheckout={(jobId,stepIndex)=><PipelineRepository key={`${jobId}:${stepIndex}`} workspace={workspace!} draft={draft} jobId={jobId} stepIndex={stepIndex} readOnly={readOnly} onChange={onChange} onValidityChange={reportCheckout}/>}/></div>;
  if (field.type === "marketplace-repository") return <div className="min-w-0 sm:col-span-2"><MarketplaceRepository workspace={workspace} draft={draft} options={options} readOnly={readOnly} onChange={onChange} onValidityChange={reportManifest} /></div>;
  if (field.type === "pipeline-repository") return <div className="min-w-0 sm:col-span-2"><PipelineRepository workspace={workspace} draft={draft} options={options} readOnly={readOnly} onChange={onChange} onValidityChange={reportManifest} /></div>;

  if (field.type === "repository-source") return <div className="min-w-0 sm:col-span-2"><RepositorySource workspace={workspace} draft={draft} readOnly={readOnly} onChange={onChange} onValidityChange={reportManifest} /></div>;

  if (field.type === "manifest") return <Wrapper field={field}><ManifestEditor value={text} mode={String((draft as Record<string, unknown>).mode ?? "compose")} readOnly={readOnly} onChange={onChange} onValidityChange={reportManifest} /></Wrapper>;
  if (field.type === "cron") return <Wrapper field={field}><CronEditor value={text} readOnly={readOnly} onChange={onChange} onValidityChange={reportManifest} />{field.help && <p className="text-xs text-muted-foreground">{field.help}</p>}</Wrapper>;

  if (field.type === "multiselect") {
    const selected = new Set(Array.isArray(value) ? value as string[] : []);
    if (asText) return <Wrapper field={field}>{label}<p className="text-sm">{options.filter(option => selected.has(option.value)).map(option => option.label).join("、") || "—"}</p></Wrapper>;
    const toggle = (key: string, checked: boolean) => {
      const next = new Set(selected);
      if (checked) {
        const add = (key: string) => { if (next.has(key)) return; next.add(key); options.find(o => o.value === key)?.requires?.forEach(add); };
        add(key);
      } else {
        next.delete(key);
        let changed = true;
        while (changed) { changed = false; for (const option of options) if (next.has(option.value) && option.requires?.some(key => !next.has(key))) { next.delete(option.value); changed = true; } }
      }
      onChange([...next]);
    };
    return <Wrapper field={field}><fieldset className="space-y-3"><legend>{label}</legend>
      {field.createOption && !readOnly && <div className="flex flex-wrap items-center gap-2"><Input aria-label={`搜索${field.label}`} placeholder={`搜索${field.label}`} value={optionSearch} onChange={event => setOptionSearch(event.target.value)} className="min-w-0 flex-1"/>{permissions.has(field.createOption.permission) && <Button type="button" variant="outline" onClick={() => setCreatingOption(true)}>{field.createOption.label}</Button>}</div>}
      {!options.length && <p className="text-sm text-muted-foreground">暂无可选{field.label}{field.createOption && permissions.has(field.createOption.permission) ? "，可在此新建。" : "，请联系有权限的成员创建。"}</p>}
      {creatingOption && field.createOption && workspace && <ResourceActionDialog workspace={workspace} permissions={permissions} row={{}} onClose={() => setCreatingOption(false)} onDone={() => { setCreatingOption(false); reloadOptions(); }} action={{label:field.createOption.label, parameters:() => field.createOption!.fields, action:async (_row, values) => { const added = await field.createOption!.create(values ?? {}); onChange([...new Set([...selected, added.value])]); }}}/>}
      {[...new Set(options.filter(o => o.label.toLowerCase().includes(optionSearch.toLowerCase())).map(o => o.group ?? "选项"))].map(group => <div key={group}><p className="mb-2 text-sm text-muted-foreground">{group}</p><div className="flex flex-wrap gap-4">
        {options.filter(o => (o.group ?? "选项") === group && o.label.toLowerCase().includes(optionSearch.toLowerCase())).map(option => <label key={option.value} className="flex items-center gap-2 text-sm"><Checkbox checked={selected.has(option.value)} disabled={readOnly || option.disabled} onCheckedChange={checked => toggle(option.value, !!checked)} />{option.label}</label>)}
      </div></div>)}
      {field.help && <p className="text-xs text-muted-foreground">{field.help}</p>}
    </fieldset></Wrapper>;
  }

  if (asText)
    return (
      <Wrapper field={field}>
        {label}
        <p className={cn("min-h-5 text-sm break-words", (field.type === "code" || field.type === "textarea") && "whitespace-pre-wrap", field.type === "code" && "font-mono")}>
          {field.type === "switch" ? (
            value ? "是" : "否"
          ) : field.type === "select" ? (
            (options.find((option) => option.value === text)?.label ?? (text || "—"))
          ) : (
            text || "—"
          )}
        </p>
      </Wrapper>
    );

  return (
    <Wrapper field={field}>
      <label className="flex flex-col gap-1.5">
        {label}
        {field.type === "select" ? (
          <>
            <div className="flex items-center gap-2"><Select value={text} onValueChange={onChange} disabled={readOnly}>
              <SelectTrigger className="min-w-0 flex-1">
                <SelectValue placeholder={field.placeholder ?? "请选择"} />
              </SelectTrigger>
              <SelectContent>
                {options.map((option) => (
                  <SelectItem key={option.value} value={option.value}>
                    {option.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            {field.createOption && !readOnly && permissions.has(field.createOption.permission) && <Button type="button" variant="outline" onClick={() => setCreatingOption(true)}>{field.createOption.label}</Button>}</div>
            {!options.length && field.createOption && !readOnly && <p className="text-xs text-muted-foreground">暂无可选{field.label}，可在此添加。</p>}
            {creatingOption && field.createOption && workspace && <ResourceActionDialog workspace={workspace} permissions={permissions} row={{}} onClose={() => setCreatingOption(false)} onDone={() => { setCreatingOption(false); reloadOptions(); }} action={{ label: field.createOption.label, parameters: () => field.createOption!.fields, action: async (_row, values) => { const added = await field.createOption!.create(values ?? {}); onChange(added.value); } }} />}
          </>
        ) : field.type === "textarea" || field.type === "secret" || field.type === "code" ? (
          // A `secret` has to be a textarea, not <input type=password>: an
          // HTML input cannot hold newlines, so pasting a PEM private key
          // into one silently strips them and stores an unusable key. The
          // cost is that the value is visible while typing; for a key being
          // pasted that is a fair trade, and nothing is ever rendered back.
          <textarea
            className={cn(
              "min-h-20 rounded-md border border-input bg-transparent px-3 py-2 shadow-xs",
              "outline-none focus-visible:border-ring focus-visible:ring-[3px]",
              "focus-visible:ring-ring/50 disabled:opacity-50",
              field.type === "code" ? "min-h-64 font-mono text-sm" : field.type === "secret" ? "font-mono text-xs" : "text-sm",
            )}
            value={text}
            placeholder={field.placeholder}
            readOnly={readOnly}
            spellCheck={false}
            autoComplete="off"
            onChange={(event) => onChange(event.target.value)}
          />
        ) : field.type === "switch" ? (
          <Checkbox
            checked={!!value}
            disabled={readOnly}
            onCheckedChange={(checked) => onChange(!!checked)}
          />
        ) : (
          <Input
            type={
              field.type === "number" ? "number" : field.type === "text" ? "text" : "password"
            }
            value={text}
            placeholder={field.placeholder}
            readOnly={readOnly}
            required={field.required}
            min={field.min}
            autoComplete={field.type === "password" ? "new-password" : undefined}
            onChange={(event) =>
              onChange(field.type === "number" && event.target.value !== "" ? Number(event.target.value) : event.target.value)
            }
          />
        )}
      </label>
      {field.help && <p className="mt-1 text-xs text-muted-foreground">{field.help}</p>}
    </Wrapper>
  );
}

const Wrapper = <T,>({
  field,
  children,
}: {
  field: FormField<T>;
  children: React.ReactNode;
}) => (
  <div className={cn("resource-field flex min-w-0 flex-col gap-1.5", field.span === 2 && "sm:col-span-2")}>{children}</div>
);

/** Shared field rendering and modal lifecycle for parameterized row actions. */
export function ResourceActionDialog<T>({ action, row, workspace, permissions, onClose, onDone }: {
 action: RowAction<T>; row: T; workspace: string; permissions: Set<string>; onClose: () => void; onDone: () => void;
}) {
 const [values,setValues]=useState<Record<string,unknown>>(() => action.initialValues?.(row) ?? {});
 const [blocked,setBlocked]=useState<Record<string,boolean>>({});
 const [busy,setBusy]=useState(false),[error,setError]=useState("");
 const report=useCallback((key:string,value:boolean)=>setBlocked(current=>current[key]===value?current:{...current,[key]:value}),[]);
 const fields=action.parameters!(row).filter(field=>!field.when||field.when(values));
 const incomplete=fields.some(field=>field.required&&(values[field.key]===undefined||values[field.key]===null||values[field.key]===""||(Array.isArray(values[field.key])&&(values[field.key] as unknown[]).length===0)) || (field.type==="number"&&field.min!==undefined&&Number(values[field.key])<field.min));
 return <Dialog.Root open onOpenChange={open=>{if(!open&&!busy)onClose();}}><Dialog.Portal>
  <Dialog.Overlay className="fixed inset-0 z-50 bg-black/40" />
  <Dialog.Content className="fixed left-1/2 top-1/2 z-50 max-h-[90dvh] w-[min(32rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 overflow-y-auto rounded-lg border bg-background p-6 shadow-lg">
   <Dialog.Title className="text-lg font-semibold">{action.label}</Dialog.Title>
   <Dialog.Description className="mt-2 text-sm text-muted-foreground">{action.description ?? "确认以下执行参数后开始操作。"}</Dialog.Description>
   <form className="mt-4 space-y-4" onSubmit={async event=>{
    event.preventDefault();event.stopPropagation();if(busy||incomplete||Object.values(blocked).some(Boolean))return;
    setBusy(true);setError("");try{await action.action?.(row,values);onDone();}catch(cause){setError(errorText(cause));}finally{setBusy(false);}
   }}>
    {fields.map(field=><Field key={field.key} field={field} workspace={workspace} value={values[field.key]} readOnly={busy} asText={false} stored={false} creating draft={values} permissions={permissions} reportOptions={report} onChange={value=>setValues(current=>({...current,...Object.fromEntries((field.resetOnChange??[]).map(key=>[key,undefined])),[field.key]:value}))} />)}
    {error&&<p role="alert" className="text-sm text-destructive">{error}</p>}
    <div className="flex justify-end gap-2"><Button type="button" variant="outline" disabled={busy} onClick={onClose}>取消</Button><Button type="submit" disabled={busy||incomplete||Object.values(blocked).some(Boolean)}>{busy?"处理中…":"确认"+action.label}</Button></div>
   </form>
  </Dialog.Content>
 </Dialog.Portal></Dialog.Root>;
}
