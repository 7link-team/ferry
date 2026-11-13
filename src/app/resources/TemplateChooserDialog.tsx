import { FilePlus2, LayoutTemplate } from "lucide-react";
import { Dialog } from "radix-ui";
import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { errorText } from "../../shared/api/api";
import { AutoForm } from "../../shared/forms/auto-form";
import { commonText } from "../../shared/i18n/i18n";
import type { FormConfig, FormField } from "../../shared/resources/types";
import { Button } from "../../shared/ui/button";
import { Input } from "../../shared/ui/input";
import { LoadingSkeleton } from "../../shared/ui/loading-skeleton";

const PAGE_SIZE = 12;
type TemplateRow = { id: string; name: string; description?: string; version: number };

/** One creation/import chooser for applications, Actions and node workflows. */
export function TemplateChooserDialog<T>({ config, allowBlank, onBlank, onCancel, onGenerated, renderParameter, validate }: {
  config: NonNullable<FormConfig<T>["generator"]>;
  allowBlank: boolean;
  onBlank: () => void;
  onCancel: () => void;
  onGenerated: (value: Partial<T>) => void;
  renderParameter: (field: FormField<Record<string, unknown>>, value: unknown, onChange: (value: unknown) => void, busy: boolean) => ReactNode;
  validate: (fields: FormField<Record<string, unknown>>[], values: Record<string, unknown>) => boolean;
}) {
  const radioName = useId();
  const [selected, setSelected] = useState(() => new URLSearchParams(location.search).get("template") ?? "");
  const [step, setStep] = useState<"catalog" | "parameters">(() => new URLSearchParams(location.search).get("template") ? "parameters" : "catalog");
  const [page, setPage] = useState(1);
  const [search, setSearch] = useState("");
  const [query, setQuery] = useState("");
  const [attempt, setAttempt] = useState(0);
  const [catalog, setCatalog] = useState<{ rows: TemplateRow[]; total: number }>({ rows: [], total: 0 });
  const [loading, setLoading] = useState(true);
  const [listError, setListError] = useState("");
  const [template, setTemplate] = useState<{ id: string; data: Awaited<ReturnType<typeof config.load>> }>();
  const [values, setValues] = useState<Record<string, unknown>>({});
  const [loadingTemplate, setLoadingTemplate] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const alive = useRef(true);
  const body = useRef<HTMLDivElement>(null);
  const title = useRef<HTMLHeadingElement>(null);
  const previousStep = useRef(step);
  useEffect(() => {
    if (previousStep.current !== step) title.current?.focus();
    previousStep.current = step;
  }, [step]);
  useEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);
  useEffect(() => {
    const timer = setTimeout(() => setQuery(search.trim()), 250);
    return () => clearTimeout(timer);
  }, [search]);
  useEffect(() => {
    if (step !== "catalog") return;
    const controller = new AbortController();
    setLoading(true); setListError("");
    void config.list({ page, pageSize: PAGE_SIZE, search: query, signal: controller.signal })
      .then(result => {
        if (controller.signal.aborted) return;
        const last = Math.max(1, Math.ceil(result.total / PAGE_SIZE));
        if (page > last) { setPage(last); return; }
        setCatalog({ rows: result.items, total: result.total });
        if (body.current) body.current.scrollTop = 0;
      })
      .catch(cause => { if (!controller.signal.aborted) setListError(errorText(cause)); })
      .finally(() => { if (!controller.signal.aborted) setLoading(false); });
    return () => controller.abort();
  }, [config, page, query, attempt, step]);
  useEffect(() => {
    let live = true;
    setTemplate(undefined); setError(""); setValues({});
    if (!selected) { setLoadingTemplate(false); return; }
    setLoadingTemplate(true);
    void config.load(selected)
      .then(data => { if (live) { setTemplate({ id: selected, data }); setValues(data.values); } })
      .catch(cause => { if (live) setError(errorText(cause)); })
      .finally(() => { if (live) setLoadingTemplate(false); });
    return () => { live = false; };
  }, [config, selected, attempt]);
  const current = template?.id === selected ? template.data : undefined;
  const pages = Math.max(1, Math.ceil(catalog.total / PAGE_SIZE));
  const choose = (id: string) => { setSelected(id); setError(""); };
  const cancel = () => { if (!busy) onCancel(); };
  return <Dialog.Root open onOpenChange={open => { if (!open) cancel(); }}><Dialog.Portal>
    <Dialog.Overlay className="fixed inset-0 z-50 bg-black/40" />
    <Dialog.Content className="dialog-shell fixed left-1/2 top-1/2 z-50 h-[min(46rem,90dvh)] w-[min(56rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 rounded-lg border bg-background p-4 shadow-lg sm:p-6" onEscapeKeyDown={event => { if (busy) event.preventDefault(); }} onInteractOutside={event => event.preventDefault()}>
      <div className="space-y-2 pb-4">
        <Dialog.Title ref={title} tabIndex={-1} className="text-lg font-semibold">{commonText(step === "parameters" ? "form.templateParametersStep" : allowBlank ? "form.startTitle" : "form.importTemplate")}</Dialog.Title>
        <Dialog.Description className="text-sm text-muted-foreground">{commonText(step === "parameters" ? "form.templateParametersHelp" : "form.templateSelectionHelp")}</Dialog.Description>
        <ol className="flex flex-wrap gap-x-4 gap-y-1 text-sm text-muted-foreground" aria-label={commonText("form.templateSteps")}>
          <li aria-current={step === "catalog" ? "step" : undefined}>1. {commonText("form.templateList")}</li>
          <li aria-current={step === "parameters" ? "step" : undefined}>2. {commonText("resource-domain:template.parameters")}</li>
        </ol>
        {step === "catalog" && <Input aria-label={commonText("common.search")} placeholder={commonText("common.search")} value={search} disabled={busy} onChange={event => { setSearch(event.target.value); setPage(1); }} />}
      </div>
      <AutoForm className="dialog-form" onSubmit={event => {
        event.preventDefault(); event.stopPropagation();
        if (busy) return;
        if (!selected) { if (allowBlank) onBlank(); return; }
        if (step === "catalog") { setStep("parameters"); if (body.current) body.current.scrollTop = 0; return; }
        if (!current || !validate(current.fields, values)) return;
        setBusy(true); setError("");
        void config.generate(selected, current.version, values)
          .then(result => { if (alive.current) onGenerated(result); })
          .catch(cause => { if (alive.current) setError(errorText(cause)); })
          .finally(() => { if (alive.current) setBusy(false); });
      }}>
        <div ref={body} className="dialog-body space-y-4 pr-1" data-template-catalog>
          {step === "catalog" && <fieldset className="min-w-0" disabled={busy}>
            <legend className="sr-only">{commonText("form.templateList")}</legend>
            <div className="grid min-w-0 gap-3 sm:grid-cols-2 lg:grid-cols-3">
              {allowBlank && <label className={cardStyle(!selected)}><span className="flex items-center gap-2"><input type="radio" name={radioName} checked={!selected} onChange={() => choose("")} /><FilePlus2 className="size-5 shrink-0" /><span className="font-medium">{commonText("form.startBlank")}</span></span><span className="text-sm text-muted-foreground">{commonText("form.blankHelp")}</span></label>}
              {!loading && !listError && catalog.rows.map(row => <label key={row.id} className={cardStyle(selected === row.id)}>
                <span className="flex items-start gap-2"><input type="radio" name={radioName} checked={selected === row.id} onChange={() => choose(row.id)} className="mt-1 shrink-0" /><LayoutTemplate className="size-5 shrink-0" /><span title={row.name} className="min-w-0 line-clamp-2 break-words font-medium [overflow-wrap:anywhere]">{row.name}</span></span>
                <span className="line-clamp-3 break-words text-sm text-muted-foreground [overflow-wrap:anywhere]">{row.description}</span>
                <span className="text-xs text-muted-foreground">{commonText("common.version")} {row.version}</span>
              </label>)}
            </div>
          </fieldset>}
          {step === "catalog" && loading && <LoadingSkeleton variant="form" label={commonText("form.loadingTemplates")} />}
          {step === "catalog" && listError && <div role="alert" className="space-y-2 text-sm text-destructive"><p>{listError}</p><Button type="button" variant="outline" onClick={() => setAttempt(n => n + 1)}>{commonText("common.retry")}</Button></div>}
          {step === "catalog" && !loading && !listError && !catalog.rows.length && <p className="py-4 text-sm text-muted-foreground">{commonText("form.noTemplates")}</p>}
          {step === "parameters" && loadingTemplate && <LoadingSkeleton variant="inline" />}
          {step === "parameters" && current && <section className="space-y-3" aria-label={commonText("resource-domain:template.parameters")}>
            <h3 className="break-words font-medium [overflow-wrap:anywhere]">{current.name} · {commonText("common.version")} {current.version}</h3>
            <div className="grid min-w-0 gap-4 sm:grid-cols-2">{current.fields.map(field => <div key={field.key} className="min-w-0">{renderParameter(field, values[field.key], value => setValues(previous => ({ ...previous, [field.key]: value })), busy)}</div>)}</div>
          </section>}
          {step === "parameters" && error && <div role="alert" className="space-y-2 text-sm text-destructive"><p>{error}</p><Button type="button" variant="outline" disabled={busy} onClick={() => setAttempt(n => n + 1)}>{commonText("form.reloadTemplate")}</Button></div>}
        </div>
        {step === "catalog" && <nav aria-label={commonText("table.pagination")} className="flex shrink-0 flex-wrap items-center justify-between gap-2 border-t py-3 text-sm">
          <span>{commonText("table.totalSummary", { count: catalog.total, start: catalog.total ? (page - 1) * PAGE_SIZE + 1 : 0, end: Math.min(page * PAGE_SIZE, catalog.total) })}</span>
          <div className="flex items-center gap-2"><Button type="button" variant="outline" className="min-h-11" disabled={loading || busy || page <= 1} onClick={() => setPage(n => n - 1)}>{commonText("table.previous")}</Button><span className="tabular-nums">{page} / {pages}</span><Button type="button" variant="outline" className="min-h-11" disabled={loading || busy || !!listError || page >= pages} onClick={() => setPage(n => n + 1)}>{commonText("table.next")}</Button></div>
        </nav>}
        <div className="dialog-footer grid grid-cols-2 pb-[env(safe-area-inset-bottom)] [&>button]:h-auto [&>button]:min-w-0 [&>button]:whitespace-normal [&>button]:py-2 sm:flex sm:flex-wrap sm:[&>button]:flex-1">
          <Button type="button" variant="outline" className="min-h-11" disabled={busy} onClick={cancel}>{commonText("common.cancel")}</Button>
          {step === "parameters" && <Button type="button" variant="outline" className="min-h-11" disabled={busy} onClick={() => { setStep("catalog"); if (body.current) body.current.scrollTop = 0; }}>{commonText("form.changeTemplate")}</Button>}
          <Button type="submit" className={`min-h-11 ${step === "parameters" ? "col-span-2 sm:col-auto" : ""}`} disabled={busy || (selected ? step === "parameters" && (!current || loadingTemplate || !validate(current.fields, values)) : !allowBlank)}>{busy ? commonText("common.processing") : commonText(selected ? step === "catalog" ? "form.configureTemplate" : "form.useTemplate" : "form.startBlank")}</Button>
        </div>
      </AutoForm>
    </Dialog.Content>
  </Dialog.Portal></Dialog.Root>;
}

const cardStyle = (selected: boolean) => `flex min-h-28 min-w-0 cursor-pointer flex-col gap-2 rounded-lg border-2 p-3 focus-within:ring-2 focus-within:ring-ring ${selected ? "border-primary bg-accent text-accent-foreground" : "border-border bg-card hover:border-primary/50"}`;
