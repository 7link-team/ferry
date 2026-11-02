import { Trash2 } from "lucide-react";
import {
  AutoField,
  FieldControl,
  selectDisplay,
} from "../../../shared/forms/auto-form";
import { hostRepositoryText } from "../../../shared/i18n/i18n";
import { Button } from "../../../shared/ui/button";
import type { Path, Row } from "./model";

export const area =
  "min-h-24 w-full rounded-md border border-input bg-transparent p-3 font-mono text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";
export const grid = "grid gap-3 sm:grid-cols-2";
export type Removal = {
  path: Path;
  index: number;
  action: string;
  target: string;
};
export function manifestFields({
  get,
  plain,
  set,
  append,
  readOnly,
  invalidJson,
  setPendingRemoval,
}: {
  get: (path: Path) => unknown;
  plain: (path: Path) => unknown;
  set: (path: Path, value: unknown) => void;
  append: (path: Path, value: unknown) => void;
  readOnly: boolean;
  invalidJson: boolean;
  setPendingRemoval: (removal: Removal) => void;
}) {
  const text = (
    label: string,
    path: Path,
    options: {
      number?: boolean;
      min?: number;
      max?: number;
      hint?: string;
      fallback?: string | number;
      required?: boolean;
      multiline?: boolean;
    } = {},
  ) => {
    const current = get(path);
    return (
      <AutoField label={label} required={options.required} help={options.hint}>
        <FieldControl
          name={label}
          field={{
            name: label,
            type: options.multiline
              ? "textarea"
              : options.number
                ? "number"
                : "text",
            config: {
              ...(options.min === undefined ? {} : { min: options.min }),
              ...(options.max === undefined ? {} : { max: options.max }),
            },
          }}
          value={String(current ?? options.fallback ?? "")}
          required={options.required}
          disabled={readOnly}
          onChange={(value) =>
            set(path, value === "" && !options.required ? undefined : value)
          }
        />
      </AutoField>
    );
  };
  const select = (
    label: string,
    path: Path,
    choices: string[],
    fallback: string,
  ) => (
    <AutoField label={label}>
      <FieldControl
        name={label}
        field={{
          name: label,
          type: "select",
          config: {
            options: [
              ...new Set([...choices, String(get(path) ?? fallback)]),
            ].map((value) => ({
              value,
              label: !choices.includes(value)
                ? selectDisplay(value, { options: [] })
                : value === "recreate"
                  ? hostRepositoryText("manifest-extra.001")
                  : value === "rolling"
                    ? hostRepositoryText("manifest-extra.002")
                    : value,
            })),
          },
        }}
        value={String(get(path) ?? fallback)}
        disabled={readOnly}
        onChange={(value) => set(path, value)}
      />
    </AutoField>
  );

  const rows = (
    title: string,
    path: Path,
    initial: Row,
    render: (path: Path, row: Row) => React.ReactNode,
  ) => {
    const items = (plain(path) ?? []) as Row[];
    return (
      <section aria-label={title} className="space-y-3">
        <div className="flex items-center justify-between gap-3">
          <h4 className="text-sm font-medium">{title}</h4>
          {!readOnly && (
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={invalidJson}
              onClick={() => append(path, initial)}
            >
              {hostRepositoryText("manifest-extra.003", { value1: title })}
            </Button>
          )}
        </div>
        {items.length === 0 && (
          <p className="text-sm text-muted-foreground">
            {hostRepositoryText("manifest.011")}
          </p>
        )}
        {items.map((row, index) => (
          <fieldset key={index} className="space-y-3 border-l-2 pl-3">
            <legend className="sr-only">
              {title} {index + 1}
            </legend>
            <div className={grid}>{render([...path, index], row)}</div>
            {!readOnly && (
              <Button
                type="button"
                variant="ghost"
                size="icon"
                aria-label={hostRepositoryText("manifest.012", {
                  value1: title,
                  value2: index + 1,
                })}
                title={hostRepositoryText("manifest.012", {
                  value1: title,
                  value2: index + 1,
                })}
                disabled={invalidJson}
                onClick={() =>
                  setPendingRemoval({
                    path,
                    index,
                    action: hostRepositoryText("manifest.013", {
                      value1: title,
                    }),
                    target: String(row.name ?? row.key ?? index + 1),
                  })
                }
              >
                <Trash2 className="size-4" />
              </Button>
            )}
          </fieldset>
        ))}
      </section>
    );
  };

  return { text, select, rows };
}
export type ManifestFields = ReturnType<typeof manifestFields>;
