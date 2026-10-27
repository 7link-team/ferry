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
import { ManifestEditor } from "./ManifestEditor";
import { Loader2 } from "lucide-react";
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
import type { FormConfig, FormField, Option, Options } from "./dsl";

function useOptions(source: Options | undefined) {
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
  }, [source]);
  return { options, error, loading };
}

export function ResourceForm<T extends Record<string, unknown>>({
  config,
  workspace,
  permissions,
  onSaved,
}: {
  config: FormConfig<T>;
  workspace: string;
  permissions: Set<string>;
  onSaved?: () => void;
}) {
  const route = useRoute();
  const creating = route.mode === "new";
  const copyId = creating ? new URLSearchParams(location.search).get("copy") : null;

  const [draft, setDraft] = useState<Partial<T>>(config.blank);
  const protectedRow = !creating && !!config.readOnly?.(draft);
  const readOnly = route.mode === "view" || protectedRow;
  const [loading, setLoading] = useState(!creating || !!copyId);
  const [saving, setSaving] = useState(false);
  const [unavailableOptions, setUnavailableOptions] = useState<Record<string, boolean>>({});
  const reportOptions = useCallback((key: string, blocked: boolean) => {
    setUnavailableOptions(current => current[key] === blocked ? current : { ...current, [key]: blocked });
  }, []);
  const optionsBlocked = Object.values(unavailableOptions).some(Boolean);
  const [error, setError] = useState("");

  useEffect(() => {
    const id = creating ? (config.copy ? copyId : null) : route.id;
    if (!id) { setDraft(config.blank); setLoading(false); return; }
    let live = true;
    setLoading(true);
    getResource<T>(workspace, config.resource, id, config.endpoint)
      .then((row) => live && (setDraft(creating && config.copy ? config.copy(row) : row), setError("")))
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

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (readOnly || !canEdit || loading || optionsBlocked) return;
    setSaving(true);
    setError("");
    try {
      if (creating) {
        await createResource(workspace, config.resource, draft, config.endpoint);
      } else {
        // `version` rides along so the server can refuse a stale write.
        await patchResource(workspace, config.resource, route.id!, draft, config.endpoint);
      }
      navigate({ ...route, mode: "list" }, {});
      onSaved?.();
    } catch (cause) {
      setError(errorText(cause));
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

  return (
    <form className="max-w-3xl space-y-6 pb-8" onSubmit={submit}>
      {error && (
        <p className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {error}
        </p>
      )}

      {creating && config.generator && <FormGenerator config={config.generator} report={reportOptions} onGenerated={value => setDraft(d => ({ ...d, ...value }))} />}

      {readOnly && config.viewActions && <div className="flex flex-wrap gap-2">{config.viewActions.filter(action=>(!action.permission || permissions.has(action.permission)) && (!action.when || action.when(draft as T))).map(action=>action.to ? <Button key={action.label} variant="outline" asChild><a href={href({...route,...action.to(draft as T)},action.query?.(draft as T))}>{action.label}</a></Button> : <Button key={action.label} type="button" variant="outline" disabled={saving} onClick={async()=>{
        if(action.destructive && !confirm(`${action.label}？`))return;
        setSaving(true);setError("");
        try {await action.action?.(draft as T);if(route.id)setDraft(await getResource<T>(workspace,config.resource,route.id,config.endpoint));}
        catch(cause){setError(errorText(cause));}finally{setSaving(false);}
      }}>{action.label}</Button>)}</div>}
      {config.sections.map((section, index) => {
        const fields = section.fields.filter((field) => !field.when || field.when(draft));
        if (!fields.length) return null;
        return (
          <fieldset key={section.title ?? index} className="space-y-4">
            {section.title && (
              <legend className="mb-2 text-sm font-medium text-muted-foreground">
                {section.title}
              </legend>
            )}
            <div className="grid gap-4 sm:grid-cols-2">
              {fields.map((field) => (
                <Field
                  key={field.key}
                  field={field}
                  reportOptions={reportOptions}
                  value={draft[field.key]}
                  stored={!!draft[field.stored ?? `has_${field.key}`]}
                  creating={creating}
                  draft={draft}
                  permissions={permissions}
                  readOnly={readOnly || (!creating && !!field.immutable)}
                  asText={readOnly}
                  onChange={(value) => setDraft((d) => ({
                    ...d,
                    ...Object.fromEntries((field.resetOnChange ?? []).map((key) => [key, ""])),
                    [field.key]: value,
                  }) as Partial<T>)}
                />
              ))}
            </div>
          </fieldset>
        );
      })}

      <div className="flex gap-2">
        {readOnly ? (
          <>
            <Button asChild variant="outline">
              <a href={backToList}>返回列表</a>
            </Button>
            {!protectedRow && permissions.has(`${config.permissionResource ?? config.resource}.update`) && route.id && (
              <Button asChild>
                <a href={href({ ...route, mode: "edit" }, {})}>编辑</a>
              </Button>
            )}
          </>
        ) : (
          <>
            <Button type="submit" disabled={saving || !canEdit || optionsBlocked}>
              {saving && <Loader2 className="animate-spin" />}
              {creating ? "创建" : "保存"}
            </Button>
            <Button type="button" variant="outline" asChild>
              <a href={backToList}>取消</a>
            </Button>
          </>
        )}
      </div>
    </form>
  );
}

function FormGenerator<T>({ config, report, onGenerated }: {
  config: NonNullable<FormConfig<T>["generator"]>;
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
  return <section aria-label="从模板生成" className="space-y-4 rounded-md border p-4">
    <h2 className="font-medium">从模板生成配置</h2>
    <label className="flex flex-col gap-2 text-sm">模板
      <Select value={id || "__none"} disabled={busy || optionsLoading} onValueChange={value => { setGenerated(false); setId(value === "__none" ? "" : value); }}>
        <SelectTrigger><SelectValue placeholder="选择模板" /></SelectTrigger>
        <SelectContent><SelectItem value="__none">不使用模板</SelectItem>{options.map(o => <SelectItem key={o.value} value={o.value}>{o.label}</SelectItem>)}</SelectContent>
      </Select>
    </label>
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
    {replacing && <label className="flex flex-col gap-1.5 text-sm font-medium">
      {field.label}
      <textarea className="min-h-24 rounded-md border border-input bg-transparent px-3 py-2 font-mono text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
        value={String(value ?? "")} onChange={(event) => onChange(event.target.value)} readOnly={readOnly}
        autoComplete="off" spellCheck={false} placeholder="允许留空；空白和换行会原样保存" />
    </label>}
  </Wrapper>;
}

function Field<T>({
  field,
  value,
  readOnly,
  asText,
  stored,
  creating,
  draft,
  permissions,
  onChange,
  reportOptions,
}: {
  field: FormField<T>;
  value: unknown;
  readOnly: boolean;
  asText: boolean;
  stored: boolean;
  creating: boolean;
  draft: Partial<T>;
  permissions: Set<string>;
  onChange: (value: unknown) => void;
  reportOptions: (key: string, blocked: boolean) => void;
}) {
  const { options, error: optionsError, loading: optionsLoading } = useOptions(field.options);
  useEffect(() => {
    reportOptions(field.key, optionsLoading || !!optionsError);
    return () => reportOptions(field.key, false);
  }, [field.key, optionsLoading, optionsError, reportOptions]);
  const reportManifest = useCallback((blocked: boolean) => reportOptions(`$manifest:${field.key}`, blocked), [field.key, reportOptions]);
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

  if (field.type === "manifest") return <Wrapper field={field}><ManifestEditor value={text} mode={String((draft as Record<string, unknown>).mode ?? "compose")} readOnly={readOnly} onChange={onChange} onValidityChange={reportManifest} /></Wrapper>;

  if (field.type === "multiselect") {
    const selected = new Set(Array.isArray(value) ? value as string[] : []);
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
      {[...new Set(options.map(o => o.group ?? "选项"))].map(group => <div key={group}><p className="mb-2 text-sm text-muted-foreground">{group}</p><div className="flex flex-wrap gap-4">
        {options.filter(o => (o.group ?? "选项") === group).map(option => <label key={option.value} className="flex items-center gap-2 text-sm"><Checkbox checked={selected.has(option.value)} disabled={readOnly || option.disabled} onCheckedChange={checked => toggle(option.value, !!checked)} />{option.label}</label>)}
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
          <Select value={text} onValueChange={onChange} disabled={readOnly}>
            <SelectTrigger>
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
  <div className={cn("flex flex-col gap-1.5", field.span === 2 && "sm:col-span-2")}>{children}</div>
);
