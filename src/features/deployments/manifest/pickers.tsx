import { useCallback, useEffect, useState } from "react";
import { listResource, named } from "../../../shared/api/resource";
import { AutoField, FieldControl } from "../../../shared/forms/auto-form";
import {
  hostRepositoryText,
  useCommonTranslation,
} from "../../../shared/i18n/i18n";
import { currentLocale } from "../../../shared/i18n/locale";

export type Choice = { value: string; label: string };

/** A pipeline or build run chosen by name, keeping a stored ID that no longer resolves. */
export function ReferencePicker({
  label,
  value,
  empty,
  required,
  hint,
  readOnly,
  load,
  onChange,
}: {
  label: string;
  value: string;
  empty: string;
  required?: boolean;
  hint?: string;
  readOnly: boolean;
  load: () => Promise<Choice[]>;
  onChange: (value: string) => void;
}) {
  useCommonTranslation();
  const [choices, setChoices] = useState<Choice[]>();
  useEffect(() => {
    let live = true;
    setChoices(undefined);
    load().then(
      (rows) => {
        if (live) setChoices(rows);
      },
      () => {
        if (live) setChoices([]);
      },
    );
    return () => {
      live = false;
    };
  }, [load]);
  const known = choices ?? [];
  const selected = value === "0" ? "" : value;
  const options = [
    { value: "", label: choices ? empty : hostRepositoryText("manifest.006") },
    ...known,
    ...(selected && !known.some((choice) => choice.value === selected)
      ? [{ value: selected, label: named(undefined, selected) }]
      : []),
  ];
  return (
    <AutoField label={label} required={required} help={hint}>
      <FieldControl
        name={label}
        field={{ name: label, type: "select", config: { options } }}
        value={selected}
        required={required}
        disabled={readOnly || !choices}
        onChange={(next) => onChange(String(next))}
      />
    </AutoField>
  );
}

export function RunPicker({
  workspace,
  pipeline,
  value,
  readOnly,
  onChange,
}: {
  workspace: string;
  pipeline: string;
  value: string;
  readOnly: boolean;
  onChange: (value: string) => void;
}) {
  const { i18n } = useCommonTranslation();
  const load = useCallback(
    () => runChoices(workspace, pipeline),
    [workspace, pipeline, i18n.resolvedLanguage],
  );
  return (
    <ReferencePicker
      label={hostRepositoryText("manifest.007")}
      value={value}
      empty={hostRepositoryText("manifest.008")}
      readOnly={readOnly}
      load={load}
      onChange={onChange}
      hint={hostRepositoryText("manifest.009")}
    />
  );
}

export async function pipelineChoices(workspace: string): Promise<Choice[]> {
  const choices: Choice[] = [];
  for (let page = 1; ; page += 1) {
    const result = await listResource<{ id: string; name: string }>(
      workspace,
      "pipelines",
      { page, page_size: 200 },
    );
    choices.push(
      ...result.items.map((row) => ({
        value: row.id,
        label: named(row.name, row.id),
      })),
    );
    if (!result.items.length || page * result.page_size >= result.total)
      return choices;
  }
}

export async function runChoices(
  workspace: string,
  pipeline: string,
): Promise<Choice[]> {
  if (!pipeline || pipeline === "0") return [];
  const result = await listResource<{
    id: string;
    number: number;
    finished_at: number;
  }>(workspace, "pipeline-runs", {
    pipeline_id: pipeline,
    state: "succeeded",
    sort: "-number",
    page_size: 50,
  });
  return result.items.map((run) => ({
    value: run.id,
    label: `#${run.number} · ${run.finished_at > 0 && Number.isFinite(run.finished_at) ? new Date(run.finished_at).toLocaleString(currentLocale()) : "—"}`,
  }));
}
