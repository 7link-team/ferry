import { useQueryValue, useScrollSync } from "../router";
import { ResourceHeading } from "../../shared/ui/resource-heading";
import { ScrollFormSections } from "./ScrollFormSections";
import { TemplateChooserDialog } from "./TemplateChooserDialog";
import { LoadingSkeleton } from "../../shared/ui/loading-skeleton";
import { AutoField,AutoFields,AutoForm,FieldControl,selectDisplay } from "../../shared/forms/auto-form";
import { EditorFormActionsContext, FormActions } from "../../shared/forms/auto-form/form-actions";
import { workflowText, commonText,configText,optionText,useCommonTranslation } from "../../shared/i18n/i18n";
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
import { cn } from "cn";
import { ArrowLeft,Copy,Download,Loader2,Play,Save,SlidersHorizontal,Upload } from "lucide-react";
import { Dialog } from "radix-ui";
import { useCallback,useEffect,useId,useRef,useState,type ReactNode } from "react";
import { toast } from "sonner";
import { ManifestEditor } from "../../features/deployments/ManifestEditor";
import { ActionNotification } from "../../features/notifications/ActionNotification";
import { ActionsEditor } from "../../features/pipelines/ActionsEditor";
import { findItem } from "../nav";
import { WorkflowKindChoice } from "../../features/workflows/WorkflowKindChoice";
import { createWorkflowDraftAssets, uploadWorkflowDraftAssets, WorkflowDraftAssetsContext } from "../../features/workflows/WorkflowAssets";
import { patchTemplateWorkflow, templateWorkflow } from "../../features/templates/workflow";
import { CronEditor } from "../../features/scheduling/CronEditor";
import { errorText } from "../../shared/api/api";
import { createResource,getResource,patchResource } from "../../shared/api/resource";
import type { FormConfig,FormField,Option,Options,RowAction } from "../../shared/resources/types";
import { Button } from "../../shared/ui/button";
import { Checkbox } from "../../shared/ui/checkbox";
import { DeleteConfirmDialog } from "../../shared/ui/DeleteConfirmDialog";
import { Input } from "../../shared/ui/input";
import { Tabs,TabsContent,TabsList,TabsTrigger } from "../../shared/ui/tabs";
import { CodeInsightAliases,CodeInsightRepositories } from "../pages/CodeInsightFields";
import { MarketplaceRepository,PipelineRepository,PipelineRepositories } from "../pages/PipelineRepository";
import { RepositorySource } from "../pages/RepositorySource";
import { RunnerConnection } from "../pages/RunnerConnection";
import { href,navigate,parse,replacePath,safeLoginRedirect,useRoute } from "../router";

/** The side-menu group last opened in each grouped form during this session. */

function incompleteFields<T>(fields: FormField<T>[], values: Partial<T>) {
  return fields.some(field => {
    const value = values[field.key as keyof T];
    return (!field.when || field.when(values)) && (
    ((typeof field.required === "function" ? field.required(values) : field.required) && (value === undefined || value === null || value === "" || (Array.isArray(value) && value.length === 0))) ||
    (field.type === "number" && value !== undefined && value !== "" && (
      !Number.isFinite(Number(value)) ||
      (field.min !== undefined && Number(value) < field.min) ||
      (field.max !== undefined && Number(value) > field.max)
    ))
  ); });
}


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
  editInPlace = false,
  onCancel,
  pipelineWorkspace = false,
  showViewNavigation = true,
  showHeading = true,
  editHref,
  headerActions,
  groupLinks,
  groupContent,
  onGroupChange,
}: {
  config: FormConfig<T>;
  workspace: string;
  permissions: Set<string>;
  onSaved?: (id?: string) => void;
  createInPlace?: boolean;
  editInPlace?: boolean;
  onCancel?: () => void;
  pipelineWorkspace?: boolean;
  showViewNavigation?: boolean;
  showHeading?: boolean;
  editHref?: string;
  headerActions?: ReactNode;
  groupLinks?: ReactNode;
  groupContent?: ReactNode;
  onGroupChange?: () => void;
}) {
  useCommonTranslation();
  const pageRoute = useRoute();
  const route = createInPlace ? { ...pageRoute, mode: "new" as const } : editInPlace ? { ...pageRoute, mode: "edit" as const } : pageRoute;
  const creating = route.mode === "new";
  const copyId = creating && !createInPlace ? new URLSearchParams(location.search).get("copy") : null;
  const baseBlankDraft = creating && !createInPlace && config.resource === "alert-rules" && new URLSearchParams(location.search).get("target_kind") === "host"
    ? { ...config.blank, target_kind: "host", target_ref: new URLSearchParams(location.search).get("target_ref") ?? "" } as Partial<T>
    : creating && !createInPlace && config.resource === "host-groups"
      ? { ...config.blank, parent_id: new URLSearchParams(location.search).get("parent_id") ?? "" } as Partial<T>
      : config.blank;
  const [queryPresets] = useState(() => creating && !createInPlace
    ? Object.fromEntries((config.queryDefaults ?? []).map(key => [key, new URLSearchParams(location.search).get(key) ?? ""]).filter(([, value]) => value))
    : {});
  const [defaultWorkflowName] = useState(() => {
    if (!["workflows", "pipelines"].includes(config.resource)) return "";
    const now = new Date();
    const pad = (value: number) => String(value).padStart(2, "0");
    const suffix = Array.from(crypto.getRandomValues(new Uint8Array(3)), value => value.toString(16).padStart(2,"0")).join("");
    return `${workflowText("newWorkflow")}-${now.getFullYear()}${pad(now.getMonth()+1)}${pad(now.getDate())}-${pad(now.getHours())}${pad(now.getMinutes())}${pad(now.getSeconds())}-${suffix}`;
  });
  const presetBlankDraft = { ...baseBlankDraft, ...queryPresets } as Partial<T>;
  const blankDraft = creating && ["workflows", "pipelines"].includes(config.resource)
    ? { ...presetBlankDraft, name: defaultWorkflowName } as Partial<T> : presetBlankDraft;
  const [draft, setDraft] = useState<Partial<T>>(blankDraft);
  const [draftAssets] = useState(createWorkflowDraftAssets);
  const [pendingWorkflow, setPendingWorkflow] = useState<T | null>(null);
  useEffect(() => () => draftAssets.clear(), [draftAssets]);
  const protectedRow = !creating && !!config.readOnly?.(draft);
  const readOnly = route.mode === "view" || protectedRow;
  const [loading, setLoading] = useState(!creating || !!copyId);
  const scrollRef = useScrollSync<HTMLFormElement>(!loading);
  const [saving, setSaving] = useState(false);
  const editorForm = useRef<HTMLFormElement>(null);
  const [pendingAction, setPendingAction] = useState<RowAction<T>>();
  const [pendingDelete, setPendingDelete] = useState<RowAction<T>>();
  const [pendingActionRow, setPendingActionRow] = useState<T>();
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [group, setGroup] = useQueryValue("section", "");
  const [formTab, setFormTab] = useQueryValue("form_tab", "");
  const [templateDialog, setTemplateDialog] = useState<"start" | "import" | "export" | null>(() => config.generator && creating && !copyId ? "start" : null);
  const [templateName, setTemplateName] = useState("");
  const [templateDescription, setTemplateDescription] = useState("");
  const [templateBusy, setTemplateBusy] = useState(false);
  const [templateError, setTemplateError] = useState("");
  const [editorView, setEditorMode] = useQueryValue("editor", "visual");
  const editorMode = editorView === "yaml" ? "yaml" : "visual";
  const [unavailableOptions, setUnavailableOptions] = useState<Record<string, boolean>>({});
  const reportOptions = useCallback((key: string, blocked: boolean) => {
    setUnavailableOptions(current => current[key] === blocked ? current : { ...current, [key]: blocked });
  }, []);
  const optionsBlocked = !!templateDialog || Object.values(unavailableOptions).some(Boolean) || (!readOnly && incompleteFields(config.sections.flatMap(section => section.fields), draft));
  const [error, setError] = useState("");

  useEffect(() => {
    const id = creating ? (config.copy ? copyId : null) : route.id;
    if (!id) { setDraft(blankDraft); setLoading(false); return; }
    let live = true;
    setLoading(true);
    getResource<T>(workspace, config.resource, id, config.endpoint)
      .then((row) => {
        if (config.resource === "credentials" && findItem(route.group, route.item)?.params?.scope && row.scope !== config.blank.scope) throw new Error(commonText("form.wrongScope"));
        if (live) { setDraft(creating && config.copy ? config.copy(row) : route.mode === "edit" && config.editValues ? { ...row, ...config.editValues(row) } : row); setError(""); }
      })
      .catch((cause: unknown) => live && setError(errorText(cause)))
      .finally(() => live && setLoading(false));
    return () => {
      live = false;
    };
  }, [creating, copyId, route.id, route.mode, workspace, config]);

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
  const returnPath = ["host-groups", "pipeline-groups", "alert-rules", "code-connection-monitors", "code-insights", "holes"].includes(config.resource)
    ? safeLoginRedirect(new URLSearchParams(location.search).get("return_to")) : undefined;
  const returnUrl = returnPath ? new URL(returnPath, location.origin) : undefined;
  const returnRoute = returnUrl ? parse(returnUrl.pathname, returnUrl.search) : undefined;
  const returnTarget = returnRoute?.workspace === workspace && ["list", "view"].includes(returnRoute.mode) ? returnPath : undefined;
  const backToList = returnTarget ?? href({ ...route, mode: "list" }, {});
  const canEdit = permissions.has(`${config.permissionResource ?? config.resource}.${creating ? "create" : "update"}`);
  const templateWorkspace = config.resource === "pipeline-templates";
  const workspaceEditor = !!config.editor || (pipelineWorkspace && (templateWorkspace || (config.resource === "pipelines" && (creating || route.mode === "edit"))));

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
      readOnly={readOnly || (!creating && !!field.immutable) || (creating && field.key in queryPresets) || !!field.disabled?.(draft)}
      asText={readOnly}
      fillHeight={workspaceEditor && (["actions-editor", "actions-template-editor"].includes(field.type) || config.editor?.field === field.key)}
      {...(workspaceEditor ? { editorMode, onEditorModeChange: setEditorMode } : {})}
      onChange={(value) => setDraft((d) => ({
        ...d,
        ...Object.fromEntries((field.resetOnChange ?? []).map((key) => [key, Array.isArray(d[key]) || Array.isArray(config.blank[key]) ? [] : ""])),
        ...(["repository-source", "marketplace-repository", "actions-editor", "actions-template-editor", "custom"].includes(field.type) ? value as Record<string, unknown> : { [field.key]: value }),
      }) as Partial<T>)}
    />
  );

  const saveForm = async (runAfterSave = false, throwOnError = false) => {
    if (groupContent || readOnly || !canEdit || loading || optionsBlocked || saving || templateDialog) return;
    setSaving(true);
    setError("");
    let savedId: string | undefined;
    try {
      let savedWorkflow: T | undefined;
      if (creating && !pendingWorkflow) {
        const created = await createResource<(T & { id: string }) | undefined>(workspace, config.resource, draft, config.endpoint);
        savedId = created?.id;
        if (config.resource === "workflows" && created) {
          savedWorkflow = created;
          setPendingWorkflow(created);
        }
      } else {
        // `version` rides along so the server can refuse a stale write.
        savedId = pendingWorkflow ? String(pendingWorkflow.id) : route.id;
        const saved = await patchResource<T>(workspace, config.resource, savedId!, pendingWorkflow ? { ...draft, version: pendingWorkflow.version } : draft, config.endpoint);
        if (config.resource === "workflows") {
          savedWorkflow = saved;
          if (pendingWorkflow) setPendingWorkflow(saved);
        }
      }
      if (config.resource === "workflows" && savedWorkflow && savedId && draftAssets.files.size) {
        const definition = await uploadWorkflowDraftAssets(workspace, savedId, String(draft.definition), draftAssets);
        if (definition !== savedWorkflow.definition) {
          savedWorkflow = await patchResource<T>(workspace, config.resource, savedId, { version: savedWorkflow.version, definition });
          setPendingWorkflow(savedWorkflow);
        }
        draftAssets.clear();
      }
      onSaved?.(savedId);
      if (config.editor && savedId) {
        const row = await getResource<T>(workspace, config.resource, savedId, config.endpoint);
        setDraft(row);
        if (creating) navigate({ ...route, id: savedId, mode: "edit" }, {});
        toast.success(commonText("common.save"));
        return row;
      }
      if (createInPlace) return;
      if (editInPlace) {
        setDraft(await getResource<T>(workspace, config.resource, route.id!, config.endpoint));
        toast.success(commonText("common.save"));
        return;
      }
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
      if (returnTarget) replacePath(returnTarget);
      else navigate({ ...route, mode: "list" }, {});
    } catch (cause) {
      if (creating && savedId && config.resource === "workflows" && draftAssets.files.size) {
        // Keep the local files and the created row/version so retry never creates a second workflow.
        setError(errorText(cause));
      } else if (creating && savedId) {
        toast.error(commonText("form.savedRunFailed", { error: errorText(cause) }));
        navigate({ ...route, id: savedId, mode: "edit" }, {});
      } else setError(errorText(cause));
      if (throwOnError) throw cause;
    } finally {
      setSaving(false);
    }
  };
  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    await saveForm(workspaceEditor && (event.nativeEvent as SubmitEvent).submitter?.getAttribute("value") === "save-run");
  };

  if (loading) return <div className="min-w-0">{pipelineWorkspace && <div className="flex justify-end border-b px-4 py-2">{headerActions}</div>}<LoadingSkeleton variant={config.editor ? "editor" : "form"} /></div>;

  const visibleViewActions = (config.viewActions ?? []).filter(action => (!action.permission || permissions.has(action.permission)) && (!action.when || action.when(draft as T)));
  const renderViewAction = (action: NonNullable<FormConfig<T>["viewActions"]>[number]) => action.to ? <Button key={action.label} variant={action.iconOnly ? "ghost" : "outline"} size={action.iconOnly ? "icon" : "default"} aria-label={configText(action.label, action.labelKey)} title={action.iconOnly ? configText(action.label, action.labelKey) : undefined} asChild><a href={href({...route,...action.to(draft as T)},action.query?.(draft as T))}>{action.iconOnly ? action.icon : configText(action.label, action.labelKey)}</a></Button> : <Button key={action.label} type="button" variant={action.iconOnly ? "ghost" : "outline"} size={action.iconOnly ? "icon" : "default"} aria-label={configText(action.label, action.labelKey)} title={action.iconOnly ? configText(action.label, action.labelKey) : undefined} disabled={saving} onClick={async()=>{
        if(action.destructive){setPendingDelete(action);return;}
        if(action.parameters?.(draft as T).length){setPendingAction(action);return;}
        setSaving(true);setError("");
        try {await action.action?.(draft as T);if(route.id)setDraft(await getResource<T>(workspace,config.resource,route.id,config.endpoint));}
        catch(cause){setError(errorText(cause));}finally{setSaving(false);}
      }}>{action.iconOnly ? action.icon : configText(action.label, action.labelKey)}</Button>;

  const hostConnectionTabs = creating && config.resource === "hosts" && <div className="sticky top-0 z-10 grid grid-cols-2 items-center gap-2 border-b bg-background py-2" role="tablist" aria-label={commonText("form.connection")}>
    {([ ["runner", commonText("form.runner")], ["ssh", "SSH"] ] as const).map(([connection, label]) => <Button key={connection} className="w-full min-w-0" type="button" role="tab" aria-selected={draft.connection === connection} variant={draft.connection === connection ? "secondary" : "ghost"} onClick={() => setDraft(current => ({ ...current, connection } as Partial<T>))}>{label}</Button>)}
  </div>;

  if (creating && config.resource === "hosts" && draft.connection === "runner") return <div className="w-full space-y-4 pb-8">
    {hostConnectionTabs}
    {permissions.has("hosts.enroll") ? <RunnerConnection workspace={workspace} inline autoGenerate /> : <p role="alert" className="text-sm text-destructive">{commonText("form.noRunnerPermission")}</p>}
  </div>;

  const templateChooser = config.generator && (templateDialog === "start" || templateDialog === "import") && <TemplateChooserDialog
    config={config.generator} allowBlank={templateDialog === "start"}
    onBlank={() => setTemplateDialog(null)}
    onCancel={() => { const initial = templateDialog === "start"; setTemplateDialog(null); if (initial) { if (onCancel) onCancel(); else replacePath(backToList); } }}
    onGenerated={value => { setDraft(d => ({ ...d, ...value })); setTemplateDialog(null); }}
    validate={(fields, values) => !incompleteFields(fields, values)}
    renderParameter={(field, value, onChange, busy) => <Field field={field} value={value} readOnly={busy} asText={false} stored={false} creating draft={{ [field.key]: value }} permissions={new Set<string>()} reportOptions={() => {}} onChange={onChange} />}
  />;
  if (workspaceEditor) {
    const fields = config.sections.flatMap(section => section.fields).filter(field => !field.when || field.when(draft));
    const editor = fields.find(field => config.editor ? field.key === config.editor.field : ["actions-editor", "actions-template-editor"].includes(field.type));
    const settings = fields.filter(field => field.key !== "name" && field !== editor && field.key !== "application_id" && (templateWorkspace || field.key !== "description"));
    const closeAction = () => {
      setPendingAction(undefined);
      if (creating && pendingActionRow && "id" in pendingActionRow)
        navigate({ ...route, id: String(pendingActionRow.id), mode: "edit" }, {});
    };
    return <EditorFormActionsContext.Provider value={{save:()=>editorForm.current?.requestSubmit(),saveAndWait:async()=>editorForm.current?.reportValidity() ? saveForm(false,true) : undefined,saving,blocked:readOnly || !canEdit || optionsBlocked}}><WorkflowDraftAssetsContext.Provider value={draftAssets}><AutoForm ref={editorForm} className="flex h-full min-h-0 min-w-0 flex-col" onSubmit={submit}>
      <header className="flex flex-wrap items-center gap-2 border-b bg-background px-3 py-2 sm:min-h-14 sm:px-4">
        <Button type="button" variant="ghost" size="icon" aria-label={commonText(templateWorkspace || config.editor ? "common.backList" : "form.backPipeline")} title={commonText(templateWorkspace || config.editor ? "common.backList" : "form.backPipeline")} onClick={() => navigate({ ...route, mode: "list" }, {})}><ArrowLeft /></Button>
        <AutoField label={config.editor?.nameLabel ?? commonText(templateWorkspace ? "form.templateName" : "form.pipelineName")} name="name" hideLabel className="shrink-0"><Input aria-label={config.editor?.nameLabel ?? commonText(templateWorkspace ? "form.templateName" : "form.pipelineName")} placeholder={config.editor?.nameLabel ?? commonText(templateWorkspace ? "form.templateName" : "form.pipelineName")} readOnly={readOnly} required maxLength={60} className="h-9 w-48 max-w-[calc(100vw-6rem)] shrink-0 border-0 bg-transparent px-2 text-base font-semibold shadow-none focus-visible:ring-0 sm:w-56" value={String(draft.name ?? "")} onChange={event => setDraft(current => ({ ...current, name: event.target.value } as Partial<T>))} /></AutoField>
        <Button type="button" variant="ghost" size="sm" aria-expanded={settingsOpen} onClick={() => setSettingsOpen(open => !open)}><SlidersHorizontal className="size-4" />{commonText(templateWorkspace || config.editor ? "shell.configuration" : "form.pipelineSettings")}</Button>
        {editor && !config.editor && <div className="flex gap-1" role="tablist" aria-label={commonText("form.editorMode")}>
          <Button type="button" role="tab" aria-selected={editorMode === "visual"} variant={editorMode === "visual" ? "secondary" : "ghost"} size="sm" onClick={() => setEditorMode("visual")}>{commonText("form.visual")}</Button>
          <Button type="button" role="tab" aria-selected={editorMode === "yaml"} variant={editorMode === "yaml" ? "secondary" : "ghost"} size="sm" onClick={() => setEditorMode("yaml")}>{workflowText("advancedYaml")}</Button>
        </div>}
        {config.generator && <Button type="button" variant="ghost" size="sm" onClick={() => setTemplateDialog("import")}><Download />{commonText("form.importTemplate")}</Button>}
        {!templateWorkspace && permissions.has("pipeline-templates.create") && <Button type="button" variant="ghost" size="sm" onClick={() => { setTemplateError(""); setTemplateDialog("export"); }}><Upload />{commonText("form.saveAsTemplate")}</Button>}
        <div className="ml-auto flex gap-2">
      <FormActions placement="toolbar">
        {readOnly ? canEdit && <Button asChild className="w-full"><a href={editHref ?? href({ ...route, mode: "edit" }, {})}>{commonText("common.edit")}</a></Button> : config.resource !== "workflows" && <Button type="submit" variant="outline" disabled={saving || !canEdit || optionsBlocked}>{saving && <Loader2 className="animate-spin" />}<Save />{commonText(templateWorkspace && creating ? "common.create" : "common.save")}</Button>}
        {!config.editor && !templateWorkspace && permissions.has("pipelines.run") && <Button type="submit" value="save-run" disabled={saving || !canEdit || optionsBlocked}><Play />{commonText("form.saveAndRun")}</Button>}
      </FormActions>
          {headerActions}
        </div>
      </header>
      {["pipelines", "workflows", "pipeline-templates"].includes(config.resource) && <WorkflowKindChoice value={config.resource === "workflows" || (templateWorkspace && !!config.editor) ? "nodes" : "actions"} creating={creating && !templateWorkspace} dirty={JSON.stringify(draft) !== JSON.stringify(blankDraft)} allowed={kind => permissions.has(`${kind === "actions" ? "pipelines" : "workflows"}.create`)} choose={kind => navigate({ workspace, group: "workflow", item: kind === "actions" ? "pipelines" : "workflows", mode: "new" }, {})} />}
      <main className="flex min-h-0 flex-1 flex-col overflow-y-auto px-3 py-3 sm:px-4">
        {error && <p role="alert" className="mb-3 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">{error}</p>}
        {settingsOpen && <div className="mb-3 space-y-4 border-b pb-3">
            <div className="grid gap-4 sm:grid-cols-2">{settings.map(renderField)}</div>
        </div>}
        {editor && <div className="flex min-h-0 flex-1 flex-col">{renderField(editor)}</div>}
      </main>

      {pendingAction && <ResourceActionDialog action={pendingAction} row={pendingActionRow!} workspace={workspace} permissions={permissions} onClose={closeAction} onDone={() => setPendingAction(undefined)} />}
      {templateChooser}
      <Dialog.Root open={templateDialog === "export"} onOpenChange={open => { if (!open && !templateBusy) setTemplateDialog(null); }}><Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-50 bg-black/40" />
        <Dialog.Content className="fixed left-1/2 top-1/2 z-50 max-h-[90dvh] w-[min(36rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 dialog-shell rounded-lg border bg-background p-6 shadow-lg">
          <Dialog.Title className="text-lg font-semibold">{templateDialog === "import" ? commonText("form.importTemplate") : commonText("form.saveAsTemplate")}</Dialog.Title>
          <Dialog.Description className="mt-2 text-sm text-muted-foreground">{templateDialog === "import" ? commonText("form.importTemplateHelp") : commonText("form.saveTemplateHelp")}</Dialog.Description>
          {templateDialog === "export" && <div className="dialog-form mt-4">
            <div className="dialog-body space-y-4">
            <AutoField label={commonText("form.templateName")} className="block space-y-2 text-sm"><Input value={templateName} maxLength={60} onChange={event => setTemplateName(event.target.value)} /></AutoField>
            <AutoField label={commonText("common.description")} className="block space-y-2 text-sm"><Input value={templateDescription} onChange={event => setTemplateDescription(event.target.value)} /></AutoField>
            {templateError && <p role="alert" className="text-sm text-destructive">{templateError}</p>}
            </div>
            <div className="dialog-footer mt-4"><Button type="button" variant="outline" onClick={() => setTemplateDialog(null)}>{commonText("common.cancel")}</Button><Button type="button" disabled={templateBusy || !templateName.trim()} onClick={async () => {
              setTemplateBusy(true); setTemplateError("");
              try {
                const content = config.resource === "workflows" ? String(draft.definition ?? "") : JSON.stringify({ engine: "github-actions", workflow: draft.definition, executor_bindings: draft.executor_bindings ?? [], checkout_bindings: draft.checkout_bindings ?? [], graph_layout: draft.graph_layout ?? {} });
                await createResource(workspace, "pipeline-templates", { name: templateName.trim(), description: templateDescription, content, parameters: "[]" });
                setTemplateDialog(null); toast.success(commonText("form.templateSaved"));
              }
              catch (cause) { setTemplateError(errorText(cause)); }
              finally { setTemplateBusy(false); }
            }}>{templateBusy ? commonText("common.saving") : commonText("form.saveTemplate")}</Button></div>
          </div>}
        </Dialog.Content>
      </Dialog.Portal></Dialog.Root>
    </AutoForm></WorkflowDraftAssetsContext.Provider></EditorFormActionsContext.Provider>;
  }

  const hasSectionGroups = config.sections.some(section => section.group);
  const recordName = String(config.heading(draft as T) || (creating ? commonText("common.create") : route.id || commonText("common.record")));
  const formActions = <>
      {!groupContent && readOnly && showViewNavigation && !protectedRow && permissions.has(`${config.permissionResource ?? config.resource}.update`) && route.id && <FormActions>
        <Button asChild><a href={editHref ?? href({ ...route, mode: "edit" }, {})}>{commonText("common.edit")}</a></Button>
      </FormActions>}
      {!groupContent && !readOnly && <FormActions>
            {optionsBlocked && config.incompleteMessage && <p role="status" className="basis-full text-sm text-muted-foreground">{configText(config.incompleteMessage, config.incompleteMessageKey)}</p>}
            {onCancel ? <Button type="button" variant="outline" onClick={onCancel} disabled={saving}>{commonText("common.cancel")}</Button> : <Button type="button" variant="outline" asChild>
              <a href={backToList}>{commonText("common.cancel")}</a>
            </Button>}
            <Button type="submit" disabled={saving || !canEdit || optionsBlocked}>
              {saving && <Loader2 className="animate-spin" />}
              {creating ? config.resource === "hosts" ? commonText("form.addHost") : commonText("common.create") : commonText("common.save")}
            </Button>
      </FormActions>}
  </>;
  return (
    <AutoForm ref={scrollRef} className={cn("min-w-0 w-full pb-8", readOnly ? "resource-detail" : "space-y-4")} onSubmit={submit}>
      {showHeading && config.sectionNavigation !== "scroll" && !createInPlace && <ResourceHeading name={recordName} />}
      {hostConnectionTabs}
      {!readOnly && config.description && <p className="max-w-3xl text-sm leading-6 text-muted-foreground">{configText(config.description, config.descriptionKey)}</p>}
      {error && (
        <p className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {error}
        </p>
      )}

      {pendingAction && <ResourceActionDialog action={pendingAction} row={draft as T} workspace={workspace} permissions={permissions} onClose={() => setPendingAction(undefined)} onDone={() => { setPendingAction(undefined); if(route.id) void getResource<T>(workspace,config.resource,route.id,config.endpoint).then(setDraft).catch(cause=>setError(errorText(cause))); }} />}
      {pendingDelete && <DeleteConfirmDialog action={configText(pendingDelete.label, pendingDelete.labelKey)} target={String(draft.name ?? draft.title ?? draft.email ?? route.id ?? commonText("common.record"))} onClose={() => setPendingDelete(undefined)} onConfirm={async () => {
        if (pendingDelete.parameters?.(draft as T).length) { setPendingAction(pendingDelete); return; }
        setSaving(true); setError("");
        try { await pendingDelete.action?.(draft as T); if (route.id) setDraft(await getResource<T>(workspace, config.resource, route.id, config.endpoint)); }
        finally { setSaving(false); }
      }} />}
      {templateChooser}
      {creating && config.generator && canEdit && <div className="flex justify-end"><Button type="button" variant="outline" onClick={() => setTemplateDialog("import")}><Download />{commonText("form.selectTemplate")}</Button></div>}

      {(() => {
      const primarySection = config.sections.findIndex(section => section.fields.some(field => !field.when || field.when(draft)));
      const renderSection = (section: FormConfig<T>["sections"][number], index: number) => {
        const fields = section.fields.filter((field) => !(creating && config.resource === "hosts" && field.key === "connection") && (!field.when || field.when(draft)));
        if (!fields.length) return null;
        return (
          <section key={section.title ?? index} aria-label={configText(section.title, section.titleKey) || (readOnly ? index === 0 ? commonText("common.basic") : commonText("common.other") : undefined)} className={readOnly ? "detail-section" : "min-w-0 w-full space-y-4 rounded-xl border bg-card p-4 text-card-foreground"}>
            {(section.title || readOnly) && (
              <div className={readOnly ? "detail-section-title flex-wrap" : "flex items-start gap-3 text-sm font-medium"}>
                {!readOnly && config.presentation === "sections" && <span aria-hidden="true" className="flex size-7 shrink-0 items-center justify-center rounded-md bg-secondary text-xs font-semibold tabular-nums text-secondary-foreground">{String(index + 1).padStart(2, "0")}</span>}
                <div className="min-w-0 flex-1"><h2 className={config.presentation === "sections" && !readOnly ? "leading-7" : undefined}>
                {configText(section.title, section.titleKey) || (index === 0 ? commonText("common.basic") : commonText("common.other"))}
              </h2>{!readOnly && section.description && <p className="mt-1 text-xs font-normal leading-5 text-muted-foreground">{configText(section.description, section.descriptionKey)}</p>}</div>{readOnly && <div className="ml-auto flex flex-wrap items-center justify-end gap-2">{visibleViewActions.filter(action => action.section ? action.section === section.title : index === primarySection).map(renderViewAction)}</div>}</div>
            )}
            <AutoFields className={readOnly ? "detail-fields" : undefined} fields={Object.fromEntries(fields.map(field => [field.key, { name: configText(field.label, field.labelKey), type: "custom" as const, col: field.span ?? 1, width: field.width ?? (field.type === "select" && Array.isArray(field.options) ? "auto" : "full"), framed: false, render: () => renderField(field) }]))} values={draft} onChange={values => setDraft(values as Partial<T>)} readonly={readOnly} />
          </section>
        );
      };
      const groups = [...new Set(config.sections.map(section => section.group).filter((group): group is string => !!group))];
      if (!groups.length) return config.sections.map(renderSection);
      if (config.sectionNavigation === "scroll") return <ScrollFormSections name={recordName} sections={groups.map(name => ({ key: name, label: configText(name, config.sections.find(section => section.group === name)?.groupKey), content: config.sections.map((section, index) => ({ section, index })).filter(({ section }) => (section.group ?? groups[0]) === name).map(({ section, index }) => renderSection(section, index)) }))} footer={formActions} />;
      const active = groups.includes(group) ? group : groups[0]!;
      const sections = config.sections.map((section, index) => ({ section, index })).filter(({ section }) => (section.group ?? groups[0]) === active);
      const tabSections = sections.filter(({ section }) => section.tab);
      const tabs = tabSections.filter(({ section }, index) => tabSections.findIndex(item => item.section.tab === section.tab) === index);
      return <div className="flex flex-col gap-4 md:flex-row">
        <nav aria-label={commonText("form.categories")} className="sticky top-[var(--resource-heading-height,0px)] z-10 flex shrink-0 flex-wrap gap-1 bg-background md:max-h-[65dvh] md:w-40 md:flex-col md:self-start md:overflow-y-auto">
          {groups.map(name => <button key={name} type="button" aria-current={!groupContent && name === active ? "page" : undefined} onClick={() => { setGroup(name); onGroupChange?.(); }}
            className={cn("min-h-11 rounded px-3 py-2 text-left text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring", !groupContent && name === active ? "bg-accent font-medium text-accent-foreground" : "text-foreground hover:bg-accent")}>{configText(name, config.sections.find(section => section.group === name)?.groupKey)}</button>)}
          {groupLinks}
        </nav>
        <div className="min-w-0 flex-1 space-y-4" data-form-content>
          {groupContent ?? <>
          {sections.filter(({ section }) => !section.tab).map(({ section, index }) => renderSection(section, index))}
          {tabs.length > 0 && <Tabs key={active} value={tabs.some(({section})=>section.tab===formTab)?formTab:tabs[0]!.section.tab} onValueChange={setFormTab}>
            <div className="min-w-0 overflow-x-auto"><TabsList className="h-auto min-h-11">{tabs.map(({ section }) => <TabsTrigger className="min-h-11" key={section.tab} value={section.tab!}>{configText(section.tab, section.tabKey)}</TabsTrigger>)}</TabsList></div>
            {tabs.map(({ section }) => <TabsContent key={section.tab} value={section.tab!} className="space-y-4 pt-2">{tabSections.filter(item => item.section.tab === section.tab).map(({section, index}) => renderSection(section, index))}</TabsContent>)}
          </Tabs>}
          </>}
          {formActions}
        </div>
      </div>;
      })()}

      {!hasSectionGroups && formActions}
    </AutoForm>
  );
}

function SecretField<T>({ field, value, stored, creating, asText, readOnly, draft, permissions, onChange }: {
  field: FormField<T>; value: unknown; stored: boolean; creating: boolean; asText: boolean;
  readOnly: boolean; draft: Partial<T>; permissions: Set<string>; onChange: (value: unknown) => void;
}) {
  useCommonTranslation();
  field = { ...field, label: configText(field.label, field.labelKey), ...(field.helpKey ? { help: commonText(field.helpKey) } : {}), ...(field.placeholderKey ? { placeholder: commonText(field.placeholderKey) } : {}) };
  const [revealed, setRevealed] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const canReveal = !!field.reveal && permissions.has(field.reveal.permission);
  if (asText) return <Wrapper field={field}>
    <span className="text-sm font-medium">{field.label}</span>
    <p className="text-sm text-muted-foreground">{stored ? commonText("common.set") : commonText("common.notSet")}</p>
    {canReveal && stored && <Button type="button" variant="outline" disabled={busy} onClick={async () => {
      if (revealed !== undefined) { setRevealed(undefined); return; }
      setBusy(true); setError("");
      try { setRevealed(await field.reveal!.action(draft)); }
      catch (cause) { setError(errorText(cause)); }
      finally { setBusy(false); }
    }}>{busy ? commonText("form.reading") : revealed === undefined ? commonText("form.reveal") : commonText("form.hidePlaintext")}</Button>}
    {canReveal && revealed !== undefined && <pre aria-label={commonText("form.plaintext")} className="whitespace-pre-wrap break-all rounded-md border p-3 font-mono text-sm">{revealed === "" ? commonText("form.emptyString") : revealed}</pre>}
    {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
  </Wrapper>;
  const replacing = creating || !stored || value !== undefined;
  return <Wrapper field={field}>
    {!creating && stored && <label className="flex items-center gap-2 text-sm">
      <Checkbox checked={replacing} disabled={readOnly} onCheckedChange={(checked) => onChange(checked ? "" : undefined)} />
      {commonText("form.replace")}{field.label}{commonText("form.retainValue")}</label>}
    {replacing && <AutoField label={<>{field.label}</>} required={typeof field.required === "function" ? field.required(draft) : field.required} className="flex flex-col gap-1.5 text-sm font-medium">

      <textarea className="min-h-24 rounded-md border border-input bg-transparent px-3 py-2 font-mono text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
        value={String(value ?? "")} onChange={(event) => onChange(event.target.value)} readOnly={readOnly}
        required={typeof field.required === "function" ? field.required(draft) : field.required} autoComplete="off" spellCheck={false} placeholder={field.placeholder ?? commonText("form.allowEmpty")} />
    </AutoField>}
    {!creating && !field.hideSecretStatus && <p className="text-xs text-muted-foreground">{stored ? commonText("form.secretSet") : commonText("form.secretMissing")}</p>}
    {field.help && <p className="text-xs text-muted-foreground">{field.help}</p>}
  </Wrapper>;
}

function Field<T>({
  field: configuredField,
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
  useCommonTranslation();
  let field = { ...configuredField, required: !asText && !!(typeof configuredField.required === "function" ? configuredField.required(draft) : configuredField.required) };
  const { options: rawOptions, error: optionsError, loading: optionsLoading, reload: reloadOptions } = useOptions(field.options);
  const suggestionsId = useId();
  const options = rawOptions.filter(option => !field.optionFilter || field.optionFilter(option, draft)).map(option => ({ ...option, label: optionText(option), ...(option.group ? { group: configText(option.group, option.groupKey) } : {}) }));
  if (field.type === "multiselect" && Array.isArray(value)) {
    for (const selected of value.map(String)) {
      if (!options.some(option => option.value === selected))
        options.push({ value: selected, label: selectDisplay(selected, { options: [] }) });
    }
  }
  field = { ...field, label: configText(field.label, field.labelKey), ...(field.helpKey ? { help: commonText(field.helpKey) } : {}), ...(field.placeholderKey ? { placeholder: commonText(field.placeholderKey) } : {}), ...(field.createOption ? { createOption: { ...field.createOption, label: configText(field.createOption.label, field.createOption.labelKey) } } : {}) };
  const [creatingOption, setCreatingOption] = useState(false);
  const [optionSearch, setOptionSearch] = useState("");
  useEffect(() => {
    reportOptions(field.key, field.type !== "text" && (optionsLoading || !!optionsError));
    return () => reportOptions(field.key, false);
  }, [field.key, field.type, optionsLoading, optionsError, reportOptions]);
  const reportManifest = useCallback((blocked: boolean) => reportOptions(`$manifest:${field.key}`, blocked), [field.key, reportOptions]);
  const reportCheckout = useCallback((blocked: boolean) => reportOptions(`$checkout:${field.key}`, blocked), [field.key, reportOptions]);
  const text = String(value ?? "");
  let envelope: Record<string, unknown> | undefined;
  let templateError = "";
  if (field.type === "actions-template-editor") {
    try { envelope = templateWorkflow(text); }
    catch (cause) { templateError = errorText(cause); }
  }
  useEffect(() => {
    reportOptions(`$template:${field.key}`, !!templateError);
    return () => reportOptions(`$template:${field.key}`, false);
  }, [field.key, templateError, reportOptions]);
  const copyControl = field.copyable && field.type !== "secret" && field.type !== "password" && <Button type="button" variant="ghost" size="icon" className="shrink-0" disabled={!text} aria-label={`${commonText("common.copy")} ${field.label}`} title={`${commonText("common.copy")} ${field.label}`} onClick={async () => {
    try { await navigator.clipboard.writeText(text); toast.success(commonText("common.copied")); }
    catch { toast.error(commonText("common.copyFailed")); }
  }}><Copy className="size-4" aria-hidden="true" /></Button>;
  if (optionsError && field.type !== "text") return <Wrapper field={field}><p role="alert" className="text-destructive">{field.label}：{optionsError}{commonText("form.refreshRetry")}</p></Wrapper>;

  const label = (
    <span className="text-sm font-medium">
      {field.label}
      {field.required && !asText && <span className="ml-0.5 text-destructive">*</span>}
    </span>
  );

  if (field.type === "secret" && (asText || field.replaceOnEdit))
    return <SecretField field={field} value={value} stored={stored} creating={creating}
      asText={asText} readOnly={readOnly} draft={draft} permissions={permissions} onChange={onChange} />;

  if (optionsLoading && field.type !== "text") return <Wrapper field={field}><LoadingSkeleton variant="inline" label={commonText("form.loadingOptions", { name: field.label })} /></Wrapper>;

  if (field.type === "custom" && field.render) {
    const content = field.render({ workspace, draft, readOnly, onPatch: onChange, onValidityChange: reportManifest });
    return fillHeight ? <div className="flex min-h-0 min-w-0 flex-1 flex-col">{content}</div> : <Wrapper field={field}>{content}</Wrapper>;
  }

  if (field.type === "actions-template-editor" && !envelope) return <Wrapper field={field}>
    <p role="alert" className="text-sm text-destructive">{templateError}</p>
    <FieldControl name={field.key} field={{ name: field.label, type: "code" }} value={text} readonly={readOnly} onChange={value => onChange({ content: value })} />
  </Wrapper>;
  if (field.type === "actions-editor" || field.type === "actions-template-editor") {
    const editorDraft = envelope ? { ...envelope, definition: envelope.workflow } : draft;
    const change = (patch: Record<string, unknown>) => onChange(envelope ? patchTemplateWorkflow(envelope, patch) : patch);
    return <div className={cn("min-w-0 sm:col-span-2", fillHeight && "flex min-h-0 flex-1 flex-col")}><ActionsEditor renderNotification={(inputs, onChange) => <ActionNotification workspace={workspace!} inputs={inputs} readOnly={readOnly} onChange={onChange} />} draft={editorDraft} options={options} readOnly={readOnly} fillHeight={fillHeight} requireExecutorBinding={!envelope} {...(editorMode && onEditorModeChange ? { mode: editorMode, onModeChange: onEditorModeChange } : {})} onChange={change} onValidityChange={reportManifest} renderCheckoutBatch={jobId=><PipelineRepositories canCreateConnection={permissions.has("repositories.create")} key={jobId} workspace={workspace!} draft={editorDraft} jobId={jobId} readOnly={readOnly} onChange={change}/>} renderCheckout={(jobId,stepIndex)=><PipelineRepository canCreateConnection={permissions.has("repositories.create")} key={`${jobId}:${stepIndex}`} workspace={workspace!} draft={editorDraft} jobId={jobId} stepIndex={stepIndex} readOnly={readOnly} onChange={change} onValidityChange={reportCheckout}/>}/></div>;
  }
  if (field.type === "marketplace-repository") return <div className="min-w-0 sm:col-span-2"><MarketplaceRepository canCreateConnection={permissions.has("repositories.create")} workspace={workspace} draft={draft} options={options} readOnly={readOnly} onChange={onChange} onValidityChange={reportManifest} /></div>;

  if (field.type === "connection-repositories") return <CodeInsightRepositories workspace={workspace} value={value} enabled={(draft as Record<string, unknown>).enabled !== false} readOnly={readOnly} permissions={permissions} onChange={onChange} onValidityChange={reportManifest} {...(field.repositoryAddLabelKey ? {addLabel:commonText(field.repositoryAddLabelKey)} : {})} />;
  if (field.type === "author-aliases") return <CodeInsightAliases value={value} readOnly={readOnly} onChange={onChange} onValidityChange={reportManifest} />;
  if (field.type === "repository-source") return <div className="min-w-0 sm:col-span-2"><RepositorySource workspace={workspace} draft={draft} readOnly={readOnly} onChange={onChange} onValidityChange={reportManifest} /></div>;

  if (field.type === "manifest") return <Wrapper field={field}><ManifestEditor value={text} mode={String((draft as Record<string, unknown>).mode ?? "compose")} readOnly={readOnly} onChange={onChange} onValidityChange={reportManifest} workspace={workspace} /></Wrapper>;
  if (field.type === "cron" && asText) return <Wrapper field={field}>{label}<p className="break-words font-mono text-sm">{text || "—"}</p></Wrapper>;
  if (field.type === "cron") return <Wrapper field={field}><CronEditor value={text} readOnly={readOnly} onChange={onChange} onValidityChange={reportManifest} />{field.help && <p className="text-xs text-muted-foreground">{field.help}</p>}</Wrapper>;

  if (field.type === "multiselect") {
    const selected = new Set(Array.isArray(value) ? value.map(String) : []);
    if (asText) {
      const chosen = options.filter(option => selected.has(option.value));
      const groups = [...new Set(chosen.map(option => option.group ?? ""))];
      return <Wrapper field={field}>{label}{chosen.length ? <div className={groups.length > 1 ? "grid min-w-0 gap-3 lg:grid-cols-2" : "min-w-0 space-y-3"} data-multiselect-summary>
        {groups.map(group => <section key={group} aria-label={group || field.label} className="min-w-0 rounded-lg border bg-background p-3">
          {group && <h3 className="mb-2 flex items-start justify-between gap-2 text-sm font-semibold"><span className="min-w-0 break-words">{group}</span><span className="shrink-0 rounded bg-muted px-1.5 py-0.5 text-xs font-normal tabular-nums text-muted-foreground">{chosen.filter(option => (option.group ?? "") === group).length}</span></h3>}
          <ul className="flex min-w-0 flex-wrap gap-2 text-sm">{chosen.filter(option => (option.group ?? "") === group).map(option => <li key={option.value} className="max-w-full break-words rounded-md border bg-muted/50 px-2.5 py-1" title={option.value}>{option.content ?? optionText(option)}</li>)}</ul>
        </section>)}
      </div> : <p className="text-sm">—</p>}</Wrapper>;
    }
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
      {(field.searchable || field.createOption) && !readOnly && <div className="flex flex-wrap items-center gap-2"><Input aria-label={commonText("form.searchField", { name: field.label })} placeholder={commonText("form.searchField", { name: field.label })} value={optionSearch} onChange={event => setOptionSearch(event.target.value)} className="min-w-0 flex-1"/>{field.createOption && permissions.has(field.createOption.permission) && <Button type="button" variant="outline" onClick={() => setCreatingOption(true)}>{field.createOption.label}</Button>}</div>}
      {field.searchable && <div role="status" className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground"><span>{commonText("table.selected", {count:selected.size})}</span>{options.filter(option => selected.has(option.value)).map(option => <span key={option.value} className="rounded-md bg-muted px-2 py-1 text-foreground">{optionText(option)}</span>)}</div>}
      {field.searchable && options.length > 0 && !options.some(option => optionText(option).toLowerCase().includes(optionSearch.toLowerCase())) && <p className="text-sm text-muted-foreground">{commonText("table.empty")}</p>}
      {!options.length && <p className="text-sm text-muted-foreground">{commonText("form.noOptions")}{field.label}{field.createOption && permissions.has(field.createOption.permission) ? commonText("form.canCreate") : commonText("form.askCreator")}</p>}
      {creatingOption && field.createOption && workspace && <ResourceActionDialog workspace={workspace} permissions={permissions} row={{}} onClose={() => setCreatingOption(false)} onDone={() => { setCreatingOption(false); reloadOptions(); }} action={{label:field.createOption.label, labelKey:field.createOption.labelKey, parameters:() => field.createOption!.fields, action:async (_row, values) => { const added = await field.createOption!.create(values ?? {}); onChange([...new Set([...selected, added.value])]); }}}/>}
      {[...new Set(options.filter(o => optionText(o).toLowerCase().includes(optionSearch.toLowerCase())).map(o => o.group ?? commonText("common.option")))].map(group => <div key={group}>{(!field.searchable || group !== commonText("common.option")) && <p className="mb-2 text-sm text-muted-foreground">{group}</p>}<div className="flex flex-wrap gap-4">
        {options.filter(o => (o.group ?? commonText("common.option")) === group && optionText(o).toLowerCase().includes(optionSearch.toLowerCase())).map(option => <label key={option.value} className="flex items-center gap-2 text-sm"><Checkbox checked={selected.has(option.value)} disabled={readOnly || option.disabled} onCheckedChange={checked => toggle(option.value, !!checked)} />{option.content ?? optionText(option)}</label>)}
      </div></div>)}
      {field.help && <p className="text-xs text-muted-foreground">{field.help}</p>}
    </fieldset></Wrapper>;
  }

  if (field.type === "records") {
    const items = Array.isArray(value) ? value.filter((item): item is Record<string, unknown> => !!item && typeof item === "object") : [];
    const record = field.record;
    const safe = (href: string | undefined) => (href && /^https?:\/\//i.test(href) ? href : undefined);
    return <Wrapper field={field}>{label}
      {items.length && record ? <ol className="space-y-2" aria-label={field.label}>{items.map((item, index) => {
        const href = safe(record.href?.(item));
        const body = record.body?.(item);
        return <li key={index} className="rounded-md border bg-muted/20 p-3 text-sm">
          <div className="flex min-w-0 flex-wrap items-baseline gap-x-3 gap-y-1">
            <span className="font-mono font-medium">{record.title(item) || "—"}</span>
            {record.meta && <span className="text-xs text-muted-foreground">{record.meta(item)}</span>}
            {href && <a className="text-xs text-primary underline underline-offset-4" href={href} target="_blank" rel="noopener noreferrer">{commonText("common.open")}</a>}
          </div>
          {body && <p className="mt-2 whitespace-pre-wrap break-words">{body}</p>}
        </li>;
      })}</ol> : <p className="text-sm">—</p>}
    </Wrapper>;
  }

  if (asText) {
    const shown = field.display ? field.display(draft as Partial<T>) : text;
    const href = field.link && /^https?:\/\//i.test(shown) ? shown : undefined;
    return (
      <Wrapper field={field}>
        {label}
        <div className="flex min-w-0 items-start gap-2"><p className={cn("min-h-5 min-w-0 flex-1 text-sm break-words", (field.type === "code" || field.type === "textarea") && "whitespace-pre-wrap", field.type === "code" && "font-mono")}>
          {href ? (
            <a className="text-primary underline underline-offset-4" href={href} target="_blank" rel="noopener noreferrer">{href}</a>
          ) : field.type === "switch" ? (
            value ? commonText("common.yes") : commonText("common.no")
          ) : field.type === "select" ? (
            field.display ? shown || "—" : selectDisplay(value, { options, ...(field.emptyValues ? { emptyValues: field.emptyValues } : {}), ...(field.emptyLabel || field.emptyLabelKey ? { emptyLabel: configText(field.emptyLabel, field.emptyLabelKey) } : {}) })
          ) : (
            shown || "—"
          )}
        </p>{copyControl}</div>
      </Wrapper>
    );
  }

  if (field.type === "text" && field.options) return <AutoField name={field.key} label={field.label} required={field.required} help={field.help}>
    <Input name={field.key} aria-label={field.label} list={suggestionsId} value={text} disabled={readOnly} required={field.required} placeholder={field.placeholder} onChange={event => onChange(event.target.value)} />
    <datalist id={suggestionsId}>{options.map(option => <option key={option.value} value={option.value}>{optionText(option)}</option>)}</datalist>
    {optionsError && <p role="alert" className="text-sm text-destructive">{optionsError}{commonText("form.refreshRetry")}</p>}
  </AutoField>;

  return <AutoField name={field.key} label={field.label} required={field.required} help={field.help} className={field.span === 2 ? "sm:col-span-2" : undefined}>
    <div className="flex min-w-0 items-start gap-2">
      <div className="min-w-0 flex-1"><FieldControl name={field.key} field={{ name: field.label, type: field.type === "secret" ? "textarea" : field.type === "custom" ? "text" : field.type, config: { options, width: field.width ?? (Array.isArray(field.options) ? "auto" : "full"), ...(field.emptyValues ? { emptyValues: field.emptyValues } : {}), ...(field.emptyLabel || field.emptyLabelKey ? { emptyLabel: configText(field.emptyLabel, field.emptyLabelKey) } : {}), ...(field.min === undefined ? {} : { min: field.min }), ...(field.max === undefined ? {} : { max: field.max }), ...(field.placeholder ? { placeholder: field.placeholder } : {}) } }} value={value} required={field.required} disabled={readOnly} onChange={onChange} /></div>
      {copyControl}
      {field.createOption && !readOnly && permissions.has(field.createOption.permission) && <Button type="button" variant="outline" onClick={() => setCreatingOption(true)}>{field.createOption.label}</Button>}
    </div>
    {!options.length && field.createOption && !readOnly && <p className="text-xs text-muted-foreground">{commonText("form.noOptions")}{field.label}{commonText("form.canAdd")}</p>}
    {creatingOption && field.createOption && workspace && <ResourceActionDialog workspace={workspace} permissions={permissions} row={{}} onClose={() => setCreatingOption(false)} onDone={() => { setCreatingOption(false); reloadOptions(); }} action={{ label: field.createOption.label, labelKey: field.createOption.labelKey, parameters: () => field.createOption!.fields, action: async (_row, values) => { const added = await field.createOption!.create(values ?? {}); onChange(added.value); } }} />}
  </AutoField>;

}

const Wrapper = <T,>({
  field,
  children,
}: {
  field: FormField<T>;
  children: React.ReactNode;
}) => (
  <AutoField name={field.key} className={cn(field.span === 2 && "sm:col-span-2")}>{children}</AutoField>
);

/** Shared field rendering and modal lifecycle for parameterized row actions. */
export function ResourceActionDialog<T>({ action, row, workspace, permissions, onClose, onDone }: {
 action: RowAction<T>; row: T; workspace: string; permissions: Set<string>; onClose: () => void; onDone: () => void;
}) {
  useCommonTranslation();
 const [values,setValues]=useState<Record<string,unknown>>(() => action.initialValues?.(row) ?? {});
 const [blocked,setBlocked]=useState<Record<string,boolean>>({});
 const [busy,setBusy]=useState(false),[error,setError]=useState("");
 const report=useCallback((key:string,value:boolean)=>setBlocked(current=>current[key]===value?current:{...current,[key]:value}),[]);
 const fields=action.parameters!(row).filter(field=>!field.when||field.when(values));
 const incomplete=incompleteFields(fields,values);
 return <Dialog.Root open onOpenChange={open=>{if(!open&&!busy)onClose();}}><Dialog.Portal>
  <Dialog.Overlay className="fixed inset-0 z-50 bg-black/40" />
  <Dialog.Content className={`fixed left-1/2 top-1/2 z-50 max-h-[90dvh] ${action.dialogWidth === "wide" ? "w-[min(48rem,calc(100vw-2rem))]" : "w-[min(32rem,calc(100vw-2rem))]"} -translate-x-1/2 -translate-y-1/2 dialog-shell rounded-lg border bg-background p-6 shadow-lg`}>
   <Dialog.Title className="text-lg font-semibold">{configText(action.label, action.labelKey)}</Dialog.Title>
   <Dialog.Description className="mt-2 text-sm text-muted-foreground">{configText(action.description, action.descriptionKey) || commonText("form.actionHelp")}</Dialog.Description>
   <AutoForm className="dialog-form mt-4" onSubmit={async event=>{
    event.preventDefault();event.stopPropagation();if(busy||incomplete||Object.values(blocked).some(Boolean))return;
    setBusy(true);setError("");try{await action.action?.(row,values);onDone();}catch(cause){setError(errorText(cause));}finally{setBusy(false);}
   }}>
    <div className="dialog-body space-y-4">
    {fields.map(field=><Field key={field.key} field={field} workspace={workspace} value={values[field.key]} readOnly={busy} asText={false} stored={false} creating draft={values} permissions={permissions} reportOptions={report} onChange={value=>setValues(current=>({...current,...Object.fromEntries((field.resetOnChange??[]).map(key=>[key,undefined])),...(field.type === "custom" && field.render ? value as Record<string, unknown> : { [field.key]: value })}))} />)}
    {error&&<p role="alert" className="text-sm text-destructive">{error}</p>}
    </div>
    <div className="dialog-footer mt-4"><Button type="button" variant="outline" disabled={busy} onClick={onClose}>{commonText("common.cancel")}</Button><Button type="submit" disabled={busy||incomplete||Object.values(blocked).some(Boolean)}>{busy?commonText("common.processing"):commonText("form.confirmAction", { action: configText(action.label, action.labelKey) })}</Button></div>
   </AutoForm>
  </Dialog.Content>
 </Dialog.Portal></Dialog.Root>;
}
