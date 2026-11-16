import { workflowText as t } from "../../shared/i18n/i18n";
import { Button } from "../../shared/ui/button";
import { Input } from "../../shared/ui/input";
import type { ReactNode } from "react";

type Params = Record<string, unknown>;
/** The editor's structured value control, passed in to keep modules acyclic. */
type Structured = (props: { label: string; value: unknown; readOnly: boolean; change: (value: unknown) => void }) => ReactNode;
const control = "h-9 w-full rounded-md border border-input bg-background px-3 text-sm";

/** Formats `report.render` can produce; extend together with the server's FORMATS. */
export const reportFormats = [{ value: "pdf", label: "PDF" }] as const;

/** A list param given either as an expression (e.g. ={{ $json.sections }}) or edited by hand. */
function ListOrExpression({ label, value, expression, readOnly, change, Structured }: {
  label: string; value: unknown; expression: string; readOnly: boolean; change: (value: unknown) => void; Structured: Structured;
}) {
  const isExpression = typeof value === "string";
  return <div className="space-y-2 rounded-md border p-3">
    <div className="flex items-center justify-between gap-2">
      <p className="text-sm font-medium">{label}</p>
      {!readOnly && <Button type="button" variant="outline" size="sm" onClick={() => change(isExpression ? [] : expression)}>
        {t(isExpression ? "report.editManually" : "report.useExpression")}
      </Button>}
    </div>
    {isExpression
      ? <Input aria-label={label} readOnly={readOnly} value={value} placeholder={expression} onChange={event => change(event.target.value)} />
      : <Structured label={label} value={Array.isArray(value) ? value : []} readOnly={readOnly} change={change} />}
  </div>;
}

/** Parameters of the `report.render` node (structured content → document file). */
export function WorkflowReportParameters({ value, readOnly, patch, Structured }: {
  value: Params; readOnly: boolean; patch: (next: Params) => void; Structured: Structured;
}) {
  const text = (key: string, label: Parameters<typeof t>[0], placeholder?: string) => (
    <label className="block space-y-1.5 text-sm">
      <span className="font-medium">{t(label)}</span>
      <Input aria-label={t(label)} readOnly={readOnly} value={String(value[key] ?? "")} placeholder={placeholder}
        onChange={event => patch({ [key]: event.target.value })} />
    </label>
  );
  return <div className="space-y-3">
    <p className="text-sm leading-6 text-muted-foreground">{t("report.help")}</p>
    <label className="block space-y-1.5 text-sm">
      <span className="font-medium">{t("report.format")}</span>
      <select aria-label={t("report.format")} className={control} disabled={readOnly} value={String(value.format ?? "pdf")}
        onChange={event => patch({ format: event.target.value })}>
        {reportFormats.map(format => <option key={format.value} value={format.value}>{format.label}</option>)}
      </select>
    </label>
    {text("title", "report.title", "={{ $json.title }}")}
    {text("subtitle", "report.subtitle")}
    {text("period", "report.period")}
    <label className="block space-y-1.5 text-sm">
      <span className="font-medium">{t("report.summary")}</span>
      <textarea aria-label={t("report.summary")} className="min-h-24 w-full rounded-md border border-input bg-background p-3 text-sm"
        readOnly={readOnly} value={String(value.summary ?? "")} placeholder="={{ $json.summary }}"
        onChange={event => patch({ summary: event.target.value })} />
    </label>
    <ListOrExpression label={t("report.metrics")} value={value.metrics} expression="={{ $json.metrics }}" readOnly={readOnly} change={metrics => patch({ metrics })} Structured={Structured} />
    <ListOrExpression label={t("report.sections")} value={value.sections} expression="={{ $json.sections }}" readOnly={readOnly} change={sections => patch({ sections })} Structured={Structured} />
    <label className="block space-y-1.5 text-sm">
      <span className="font-medium">{t("report.language")}</span>
      <select aria-label={t("report.language")} className={control} disabled={readOnly} value={String(value.language ?? "zh")}
        onChange={event => patch({ language: event.target.value })}>
        <option value="zh">{t("report.languageZh")}</option>
        <option value="en">{t("report.languageEn")}</option>
      </select>
    </label>
    {text("file_name", "report.fileName", t("report.fileNamePlaceholder"))}
    {text("output_field", "report.outputField", "file")}
    <label className="block space-y-1.5 text-sm">
      <span className="font-medium">{t("report.linkTtl")}</span>
      <Input aria-label={t("report.linkTtl")} type="number" min={1} max={30} step={1} readOnly={readOnly}
        value={String(value.link_ttl_days ?? 7)} onChange={event => patch({ link_ttl_days: Number(event.target.value) })} />
    </label>
    <p className="text-xs leading-5 text-muted-foreground">{t("report.outputHelp")}</p>
  </div>;
}
