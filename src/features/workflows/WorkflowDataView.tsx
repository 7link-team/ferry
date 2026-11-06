import { FieldCaption } from "../../shared/forms/FieldHelp";
import { useState } from "react";
import { Copy, ChevronLeft, ChevronRight } from "lucide-react";
import { workflowText as t } from "../../shared/i18n/i18n";
import { Button } from "../../shared/ui/button";
import { AutoTable } from "../../shared/tables/auto-table";
import { WorkflowAssetPreview } from "./WorkflowAssets";
import { fieldExpression, type WorkflowItem } from "./workflow-data";

const display = (value: unknown) =>
  typeof value === "string" ? value : JSON.stringify(value);
function Fields({
  value,
  path = [],
  reference,
}: {
  value: unknown;
  path?: (string | number)[];
  reference?: (expression: string) => void;
}) {
  if (!value || typeof value !== "object")
    return (
      <span className="break-words text-xs text-muted-foreground">
        {display(value)}
      </span>
    );
  const entries = Object.entries(value).slice(0, 100);
  if (!entries.length) return <p className="text-xs text-muted-foreground"><code>{Array.isArray(value) ? "[]" : "{}"}</code> · {t("emptyObject")}</p>;
  return (
    <div className="space-y-2">
      {entries.map(([key, item]) => {
        const next = [...path, Array.isArray(value) ? Number(key) : key];
        const expression = fieldExpression(next);
        const nested = item !== null && typeof item === "object";
        return (
          <div key={key} className="min-w-0 border-l pl-3">
            <div className="flex min-w-0 items-center gap-2">
              <button
                type="button"
                disabled={!reference}
                draggable={!!reference}
                className="min-w-0 rounded px-1 text-left text-xs font-medium text-primary underline-offset-4 enabled:hover:underline focus-visible:outline-2 focus-visible:outline-ring"
                aria-label={`${t("inspector.reference")} ${JSON.stringify(next)}`}
                title={reference ? expression : undefined}
                onMouseDown={(event) => event.preventDefault()}
                onClick={() => reference?.(expression)}
                onDragStart={(event) => {
                  event.dataTransfer.setData(
                    "application/x-workflow-field",
                    expression,
                  );
                  event.dataTransfer.setData("text/plain", expression);
                  event.dataTransfer.effectAllowed = "copy";
                }}
              >
                <span className="break-all">{key}</span>
              </button>
              <span className="text-[11px] text-muted-foreground">
                {item === null
                  ? "null"
                  : Array.isArray(item)
                    ? `array · ${item.length}`
                    : typeof item}
              </span>
            </div>
            {nested ? (
              <details className="mt-1" open={path.length < 1}>
                <summary className="cursor-pointer text-xs text-muted-foreground">
                  {t("inspector.expand")}
                </summary>
                <div className="mt-2">
                  <Fields
                    value={item}
                    path={next}
                    {...(reference ? { reference } : {})}
                  />
                </div>
              </details>
            ) : (
              <div className="mt-1 line-clamp-3 whitespace-pre-wrap break-all text-xs text-muted-foreground">
                {display(item)}
              </div>
            )}
          </div>
        );
      })}
      {Object.keys(value).length > 100 && (
        <p className="text-xs text-muted-foreground">
          {t("inspector.fieldLimit")}
        </p>
      )}
    </div>
  );
}

export function WorkflowDataView({
  items,
  reference,
  empty,
}: {
  items: WorkflowItem[];
  reference?: (expression: string) => void;
  empty?: string;
}) {
  const [mode, setMode] = useState("schema");
  const [page, setPage] = useState(0);
  const [copy, setCopy] = useState("");
  const last = Math.max(0, Math.ceil(items.length / 20) - 1),
    current = Math.min(page, last);
  const rows = items.slice(current * 20, current * 20 + 20);
  const [itemIndex, setItemIndex] = useState(0);
  const selected = items[Math.min(itemIndex, Math.max(0, items.length - 1))];
  const columns = [
    ...new Set(rows.flatMap((item) => Object.keys(item.json))),
  ].slice(0, 30);
  return (
    <div className="min-w-0 space-y-4">
      <div
        className="flex flex-wrap items-center gap-1 border-b pb-3"
        role="group"
        aria-label={t("inspector.view")}
      >
        {(["schema", "table", "json", "binary"] as const).map((key) => (
          <Button
            key={key}
            type="button"
            size="sm"
            variant={mode === key ? "secondary" : "ghost"}
            aria-pressed={mode === key}
            onClick={() => {
              setMode(key);
              setCopy("");
            }}
          >
            {t(`inspector.${key}`)}
          </Button>
        ))}
        <span className="ml-auto text-xs tabular-nums text-muted-foreground">
          {items.length} {t("inspector.items")}
        </span>
      </div>
      {!items.length ? (
        <p className="py-10 text-center text-sm leading-6 text-muted-foreground">
          {empty ?? t("inspector.noData")}
        </p>
      ) : (
        <>
          {(mode === "schema" || mode === "binary") && (
            <label className="flex items-center justify-between gap-3 text-xs text-muted-foreground">
              <FieldCaption label={t("inspector.item")} />
              <input
                aria-label={t("inspector.item")}
                type="number"
                min={1}
                max={items.length}
                value={Math.min(itemIndex + 1, items.length)}
                className="h-8 w-20 rounded border bg-background px-2"
                onChange={(event) =>
                  setItemIndex(
                    Math.max(
                      0,
                      Math.min(
                        items.length - 1,
                        Number(event.target.value) - 1,
                      ),
                    ),
                  )
                }
              />
            </label>
          )}
          {mode === "schema" && (
            <Fields
              value={selected?.json}
              {...(reference ? { reference } : {})}
            />
          )}
          {mode === "binary" && (
            <div className="space-y-4">
              {Object.entries(selected?.binary ?? {}).map(([name, file]) => (
                <section key={name} className="min-w-0 space-y-2">
                  <h4 className="break-all text-sm font-medium">{name}</h4>
                  <WorkflowAssetPreview file={file} />
                </section>
              ))}
              {!Object.keys(selected?.binary ?? {}).length && (
                <p className="py-8 text-center text-sm text-muted-foreground">
                  {t("inspector.noBinary")}
                </p>
              )}
            </div>
          )}
          {mode === "table" && (
            <div className="max-w-full overflow-x-auto rounded-md border">
              <AutoTable<WorkflowItem & {id:string}> state={{rows:rows.map((item,index)=>({...item,id:String(current*20+index)})),total:items.length,query:{page:current+1,pageSize:20}}} rowKey="id" showSequence pagination={false}
                columns={columns.map(key=>({field:key,name:key,render:(_value,row)=><span className="line-clamp-4 max-w-56 whitespace-pre-wrap break-all text-xs">{display(row.json[key])}</span>}))} />
            </div>
          )}
          {mode === "json" && (
            <>
              <div className="flex justify-end">
                <Button
                  type="button"
                  size="sm"
                  variant="ghost"
                  onClick={async () => {
                    try {
                      await navigator.clipboard.writeText(
                        JSON.stringify(rows, null, 2),
                      );
                      setCopy(t("inspector.copied"));
                    } catch {
                      setCopy(t("inspector.copyFailed"));
                    }
                  }}
                >
                  <Copy />
                  {t("inspector.copyPage")}
                </Button>
              </div>
              <pre className="max-w-full overflow-x-auto rounded-md bg-muted/40 p-3 text-xs leading-5">
                {JSON.stringify(rows, null, 2)}
              </pre>
              {copy && (
                <p role="status" className="text-xs text-muted-foreground">
                  {copy}
                </p>
              )}
            </>
          )}
          {(mode === "table" || mode === "json") && (
            <div className="flex items-center justify-between gap-2">
              <Button
                type="button"
                size="icon"
                variant="outline"
                aria-label={t("inspector.previousPage")}
                disabled={!current}
                onClick={() => setPage(current - 1)}
              >
                <ChevronLeft />
              </Button>
              <span className="text-xs tabular-nums text-muted-foreground">
                {current + 1} / {last + 1}
              </span>
              <Button
                type="button"
                size="icon"
                variant="outline"
                aria-label={t("inspector.nextPage")}
                disabled={current >= last}
                onClick={() => setPage(current + 1)}
              >
                <ChevronRight />
              </Button>
            </div>
          )}
        </>
      )}
    </div>
  );
}
