import { useCallback, useEffect, useMemo, useState } from "react";
import { parseDocument } from "yaml";
import { AutoField } from "../../shared/forms/auto-form";
import {
  hostRepositoryText,
  useCommonTranslation,
} from "../../shared/i18n/i18n";
import { Button } from "../../shared/ui/button";
import { DeleteConfirmDialog } from "../../shared/ui/DeleteConfirmDialog";
import { area, grid, manifestFields } from "./manifest/fields";
import { JsonField } from "./manifest/JsonField";
import {
  appendManifest,
  changeManifest,
  readManifest,
  removeManifest,
  type ManifestChange,
  type Path,
  type Row,
} from "./manifest/model";
import { pipelineChoices } from "./manifest/pickers";
import { ServiceEditor } from "./manifest/ServiceEditor";
export { readManifest } from "./manifest/model";

export function ManifestEditor({
  value,
  mode,
  readOnly,
  onChange,
  onValidityChange,
  workspace,
}: {
  value: string;
  mode: string;
  readOnly: boolean;
  onChange: (value: string) => void;
  onValidityChange: (blocked: boolean) => void;
  /** When known, artifact sources choose the pipeline and build run by name. */
  workspace?: string;
}) {
  const { i18n } = useCommonTranslation();
  const loadPipelines = useCallback(
    () => (workspace ? pipelineChoices(workspace) : Promise.resolve([])),
    [workspace],
  );
  const [sourceMode, setSourceMode] = useState(false);
  const [notice, setNotice] = useState("");
  const [pendingRemoval, setPendingRemoval] = useState<{
    path: Path;
    index: number;
    action: string;
    target: string;
  }>();
  const [invalidFields, setInvalidFields] = useState<Record<string, boolean>>(
    {},
  );
  const reportJson = useCallback(
    (key: string, invalid: boolean) =>
      setInvalidFields((current) =>
        current[key] === invalid ? current : { ...current, [key]: invalid },
      ),
    [],
  );
  const invalidJson = Object.values(invalidFields).some(Boolean);
  const parsed = useMemo(() => {
    try {
      return { data: readManifest(value), error: "" };
    } catch (error) {
      return {
        data: undefined,
        error: error instanceof Error ? error.message : String(error),
      };
    }
  }, [value, i18n.resolvedLanguage]);
  // Syntax errors block saving. Valid YAML that needs source-only editing remains saveable;
  // the existing server validator remains authoritative for capability/business validation.
  const syntaxError = useMemo(
    () => parseDocument(value).errors.length > 0,
    [value],
  );
  useEffect(() => {
    onValidityChange(syntaxError || invalidJson);
    return () => onValidityChange(false);
  }, [syntaxError, invalidJson, onValidityChange]);
  const doc = parsed.data?.doc;
  const services = (parsed.data?.value.services ?? []) as Row[];
  const get = (path: Path): unknown => doc?.getIn(path, false);
  const plain = (path: Path): unknown => {
    let item: unknown = parsed.data?.value;
    for (const key of path) item = (item as Row | undefined)?.[key];
    return item;
  };
  const change = (changes: ManifestChange[]) => {
    if (!doc || readOnly) return;
    setNotice("");
    onChange(changeManifest(doc, changes));
  };
  const set = (path: Path, value: unknown) => change([{ path, value }]);
  const remove = (path: Path, index: number) => {
    if (!doc || readOnly) return;
    setNotice("");
    onChange(removeManifest(doc, path, index));
  };
  const append = (path: Path, value: unknown) => {
    if (!doc || readOnly) return;
    setNotice("");
    onChange(appendManifest(doc, path, value));
  };
  const { text, select, rows } = manifestFields({
    get,
    plain,
    set,
    append,
    readOnly,
    invalidJson,
    setPendingRemoval,
  });
  return (
    <section
      aria-label={hostRepositoryText("manifest.014")}
      className="min-w-0 space-y-4"
    >
      {pendingRemoval && (
        <DeleteConfirmDialog
          action={pendingRemoval.action}
          target={pendingRemoval.target}
          description={hostRepositoryText("manifest.015")}
          onClose={() => setPendingRemoval(undefined)}
          onConfirm={() => remove(pendingRemoval.path, pendingRemoval.index)}
        />
      )}
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h2 className="font-medium">{hostRepositoryText("manifest.016")}</h2>
        <div
          className="flex gap-2"
          role="group"
          aria-label={hostRepositoryText("manifest.017")}
        >
          <Button
            type="button"
            variant={!sourceMode ? "secondary" : "outline"}
            aria-pressed={!sourceMode}
            onClick={() => setSourceMode(false)}
          >
            {hostRepositoryText("manifest.018")}
          </Button>
          <Button
            type="button"
            variant={sourceMode ? "secondary" : "outline"}
            aria-pressed={sourceMode}
            disabled={invalidJson}
            onClick={() => setSourceMode(true)}
          >
            {hostRepositoryText("manifest.019")}
          </Button>
        </div>
      </div>
      {sourceMode || !parsed.data ? (
        <>
          <AutoField
            label={hostRepositoryText("manifest.020")}
            className="flex flex-col gap-2 text-sm"
          >
            <textarea
              className={`${area} min-h-80`}
              value={value}
              readOnly={readOnly}
              spellCheck={false}
              onChange={(e) => onChange(e.target.value)}
              aria-invalid={syntaxError}
            />
          </AutoField>
          {parsed.error && (
            <p
              role="alert"
              className="whitespace-pre-wrap text-sm text-destructive"
            >
              {parsed.error}
            </p>
          )}
          <p className="text-sm text-muted-foreground">
            {hostRepositoryText("manifest.021")}
          </p>
        </>
      ) : (
        <>
          <div className={grid}>
            {text(hostRepositoryText("manifest.022"), ["metadata", "name"], {
              required: true,
              hint: hostRepositoryText("manifest.023"),
            })}
          </div>
          <p className="text-sm text-muted-foreground">
            {hostRepositoryText("manifest.024")}
          </p>
          {services.map((service, index) => (
            <ServiceEditor
              key={index}
              service={service}
              index={index}
              services={services}
              mode={mode}
              workspace={workspace}
              readOnly={readOnly}
              invalidJson={invalidJson}
              get={get}
              change={change}
              set={set}
              text={text}
              select={select}
              rows={rows}
              loadPipelines={loadPipelines}
              setPendingRemoval={setPendingRemoval}
              reportJson={reportJson}
              setNotice={setNotice}
              append={append}
              plain={plain}
            />
          ))}
          {!readOnly && (
            <Button
              type="button"
              variant="outline"
              disabled={invalidJson || services.length >= 50}
              onClick={() => {
                let number = services.length + 1;
                while (services.some((s) => s.name === `service-${number}`))
                  number++;
                append(["services"], {
                  name: `service-${number}`,
                  ...(mode === "native"
                    ? {
                        artifact: {
                          source: "pipeline",
                          pipeline: "",
                          path: "",
                        },
                      }
                    : { image: "" }),
                });
              }}
            >
              {hostRepositoryText("manifest.090")}
            </Button>
          )}
          <details className="space-y-3">
            <summary className="cursor-pointer text-sm font-medium">
              {hostRepositoryText("manifest.091")}
            </summary>
            <div className={grid}>
              {select(
                hostRepositoryText("manifest.092"),
                ["strategy", "type"],
                mode === "stack" ? ["recreate", "rolling"] : ["recreate"],
                "recreate",
              )}
              {text(
                hostRepositoryText("manifest.093"),
                ["strategy", "healthTimeout"],
                {
                  number: true,
                  min: 1,
                  max: 600,
                  fallback: 120,
                },
              )}
              {mode === "stack" &&
                text(
                  hostRepositoryText("manifest.094"),
                  ["strategy", "maxUnavailable"],
                  {
                    number: true,
                    min: 0,
                    max: 100,
                  },
                )}
            </div>
            <JsonField
              fieldId="labels"
              report={reportJson}
              label={hostRepositoryText("manifest.095")}
              value={plain(["metadata", "labels"]) ?? {}}
              readOnly={readOnly}
              onChange={(value) => set(["metadata", "labels"], value)}
            />
            <p className="text-xs text-muted-foreground">
              {hostRepositoryText("manifest.096")}
            </p>
            <JsonField
              fieldId="overrides"
              report={reportJson}
              label={hostRepositoryText("manifest.097")}
              value={plain(["overrides"]) ?? {}}
              readOnly={readOnly}
              onChange={(value) => set(["overrides"], value)}
            />
          </details>
        </>
      )}
      {notice && (
        <p role="status" className="text-sm">
          {notice}
        </p>
      )}
    </section>
  );
}
