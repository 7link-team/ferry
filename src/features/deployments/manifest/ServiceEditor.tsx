import { Trash2 } from "lucide-react";
import { AutoField } from "../../../shared/forms/auto-form";
import { commonText, hostRepositoryText } from "../../../shared/i18n/i18n";
import { Button } from "../../../shared/ui/button";
import { Checkbox } from "../../../shared/ui/checkbox";
import { Switch } from "../../../shared/ui/switch";
import { Input } from "../../../shared/ui/input";
import { grid, type ManifestFields, type Removal } from "./fields";
import { JsonField } from "./JsonField";
import { type ManifestChange, object, type Path, type Row } from "./model";
import { type Choice, ReferencePicker, RunPicker } from "./pickers";

export function ServiceEditor({
  service,
  index,
  services,
  mode,
  portable,
  workspace,
  readOnly,
  invalidJson,
  get,
  change,
  set,
  text,
  select,
  rows,
  loadPipelines,
  setPendingRemoval,
  reportJson,
  setNotice,
  append,
  plain,
}: {
  append: (path: Path, value: unknown) => void;
  plain: (path: Path) => unknown;
  service: Row;
  index: number;
  services: Row[];
  mode: string;
  portable: boolean;
  workspace: string | undefined;
  readOnly: boolean;
  invalidJson: boolean;
  get: (path: Path) => unknown;
  change: (changes: ManifestChange[]) => void;
  set: (path: Path, value: unknown) => void;
  text: ManifestFields["text"];
  select: ManifestFields["select"];
  rows: ManifestFields["rows"];
  loadPipelines: () => Promise<Choice[]>;
  setPendingRemoval: (removal: Removal) => void;
  reportJson: (key: string, invalid: boolean) => void;
  setNotice: (notice: string) => void;
}) {
  const path: Path = ["services", index];
  const p = (...keys: Path) => [...path, ...keys];
  const dependencies = (service.dependsOn ?? []) as string[];
  const artifact = object(service.artifact);
  const dualSource = portable && artifact && typeof service.image === "string";
  const source = dualSource ? "both" : artifact ? "artifact" : "image";
  const sourceOptions = [
    ...(portable ? ["both"] : []),
    ...(mode !== "native" ? ["image"] : []),
    ...(!["docker", "compose", "stack"].includes(mode) ? ["artifact"] : []),
  ];
  const sourceCompatible = sourceOptions.includes(source);
  const sourceLabel = (value: string) => value === "both"
    ? commonText("resource-domain:deployment.dualSource")
    : hostRepositoryText(value === "image" ? "manifest.034" : "manifest.035");
  const placement = object(service.placement) ? service.placement : undefined;
  return (
    <section
      key={index}
      aria-label={hostRepositoryText("manifest.025", { value1: index + 1 })}
      className="space-y-5 rounded-md border p-4"
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h3 className="font-medium">
          {index + 1}.{" "}
          {String(service.name || hostRepositoryText("manifest.026"))}
        </h3>
        {!readOnly && (
          <div className="flex flex-wrap gap-1">
            <Button
              type="button"
              variant="ghost"
              size="icon"
              aria-label={hostRepositoryText("manifest.027", {
                value1: String(service.name || index + 1),
              })}
              title={hostRepositoryText("manifest.027", {
                value1: String(service.name || index + 1),
              })}
              disabled={invalidJson || services.length <= 1}
              onClick={() => {
                if (
                  services.some(
                    (other, otherIndex) =>
                      otherIndex !== index &&
                      ((other.dependsOn ?? []) as string[]).includes(
                        String(service.name),
                      ),
                  )
                ) {
                  setNotice(hostRepositoryText("manifest.028"));
                  return;
                }
                setPendingRemoval({
                  path: ["services"],
                  index,
                  action: hostRepositoryText("manifest.029"),
                  target: String(service.name || index + 1),
                });
              }}
            >
              <Trash2 className="size-4" />
            </Button>
          </div>
        )}
      </div>
      <div className={grid}>
        <AutoField
          label={hostRepositoryText("manifest.030")}
          className="flex flex-col gap-1.5 text-sm"
        >
          <Input
            value={String(service.name ?? "")}
            required
            disabled={readOnly}
            onChange={(e) => {
              if (
                services.some(
                  (other, otherIndex) =>
                    otherIndex !== index && other.name === e.target.value,
                )
              ) {
                setNotice(hostRepositoryText("manifest.031"));
                return;
              }
              const changes = [
                { path: p("name"), value: e.target.value as unknown },
              ];
              services.forEach((other, i) => {
                if (Array.isArray(other.dependsOn))
                  other.dependsOn.forEach((name, j) => {
                    if (name === service.name)
                      changes.push({
                        path: ["services", i, "dependsOn", j],
                        value: e.target.value,
                      });
                  });
              });
              change(changes);
            }}
          />
        </AutoField>
        {text(hostRepositoryText("manifest.032"), p("replicas"), {
          number: true,
          min: 1,
          max: 100,
          fallback: 1,
        })}
        <AutoField
          label={hostRepositoryText("manifest.033")}
          className="flex flex-col gap-1.5 text-sm"
        >
          <select
            aria-label={hostRepositoryText("manifest.033")}
            className="h-9 rounded-md border border-input bg-background px-2"
            value={sourceCompatible || readOnly ? source : ""}
            required
            aria-invalid={!sourceCompatible || undefined}
            disabled={readOnly}
            onChange={(e) => {
              if (!sourceOptions.includes(e.target.value)) return;
              // Switching is an ordinary draft edit: it takes effect
              // only when saved and can be undone by switching back.
              change([
                {
                  path: p("image"),
                  value: e.target.value !== "artifact" ? (service.image ?? "") : undefined,
                },
                {
                  path: p("artifact"),
                  value:
                    e.target.value !== "image"
                      ? (service.artifact ?? { source: "pipeline", pipeline: "", path: "" })
                      : undefined,
                },
              ]);
            }}
          >
            <option value="" disabled>{commonText("common.select")}</option>
            {readOnly && !sourceCompatible && <option value={source} disabled>{sourceLabel(source)}</option>}
            {sourceOptions.map(value => <option key={value} value={value}>{sourceLabel(value)}</option>)}
          </select>
        </AutoField>
        {(!artifact || dualSource) &&
          text(hostRepositoryText("manifest.036"), p("image"), {
            required: true,
            hint: hostRepositoryText("manifest.037"),
          })}
      </div>
      {(mode === "native" ? !artifact : !service.image || (!portable && artifact)) && (
        <p role="status" className="text-sm text-destructive">
          {mode === "native"
            ? hostRepositoryText("manifest.038")
            : hostRepositoryText("manifest.039")}
        </p>
      )}
      {artifact && (
        <div className={grid}>
          {workspace ? (
            <ReferencePicker
              label={hostRepositoryText("manifest.040")}
              required
              readOnly={readOnly}
              empty={hostRepositoryText("manifest.041")}
              value={String(get(p("artifact", "pipeline")) ?? "")}
              load={loadPipelines}
              // A pinned run belongs to the previous pipeline.
              onChange={(next) =>
                change([
                  { path: p("artifact", "pipeline"), value: next || undefined },
                  { path: p("artifact", "run"), value: undefined },
                ])
              }
            />
          ) : (
            text(
              hostRepositoryText("manifest.042"),
              p("artifact", "pipeline"),
              {
                required: true,
              },
            )
          )}
          {text(hostRepositoryText("manifest.043"), p("artifact", "path"), {
            required: true,
          })}
          {workspace ? (
            <RunPicker
              workspace={workspace}
              pipeline={String(get(p("artifact", "pipeline")) ?? "")}
              readOnly={readOnly}
              value={String(get(p("artifact", "run")) ?? "")}
              onChange={(next) => set(p("artifact", "run"), next || undefined)}
            />
          ) : (
            text(hostRepositoryText("manifest.044"), p("artifact", "run"), {
              hint: hostRepositoryText("manifest.045"),
            })
          )}
        </div>
      )}
      {(portable && (mode === "stack" || placement)) && <details className="space-y-3" open={!!placement}>
        <summary className="cursor-pointer text-sm font-medium">{commonText("resource-domain:deployment.placement")}</summary>
        <p className="text-sm text-muted-foreground">{commonText("resource-domain:deployment.placementHelp")}</p>
        {mode !== "stack" && <p role="status" className="text-sm text-destructive">{commonText("resource-domain:deployment.placementUnsupported")}</p>}
        <AutoField label={commonText("resource-domain:deployment.placementConstraints")} help={commonText("resource-domain:deployment.placementConstraintsHelp")}>
          <textarea className="min-h-24 w-full rounded-md border bg-background p-3 font-mono text-sm" disabled={readOnly}
            value={Array.isArray(placement?.constraints) ? placement.constraints.join("\n") : ""} onChange={event => {
              const constraints = event.target.value.split("\n");
              const max = placement?.maxReplicasPerNode;
              set(p("placement"), !event.target.value && max == null ? undefined : { ...placement, constraints: event.target.value ? constraints : [] });
            }} />
        </AutoField>
        {text(commonText("resource-domain:deployment.maxReplicasPerNode"), p("placement", "maxReplicasPerNode"), { number: true, min: 1, max: 100, hint: commonText("resource-domain:deployment.maxReplicasHelp") })}
        {!!placement && !readOnly && <Button type="button" variant="outline" onClick={() => set(p("placement"), undefined)}>{commonText("resource-domain:deployment.clearPlacement")}</Button>}
      </details>}
      {rows(
        hostRepositoryText("manifest.046"),
        p("ports"),
        { container: 80, protocol: "tcp" },
        (row) => (
          <>
            {text(hostRepositoryText("manifest.047"), [...row, "container"], {
              number: true,
              min: 1,
              max: 65535,
              required: true,
            })}
            {text(hostRepositoryText("manifest.048"), [...row, "published"], {
              number: true,
              min: 1,
              max: 65535,
            })}
            {text(hostRepositoryText("manifest.049"), [...row, "name"])}
            {select(
              hostRepositoryText("manifest.050"),
              [...row, "protocol"],
              ["tcp", "udp"],
              "tcp",
            )}
          </>
        ),
      )}
      {rows(
        hostRepositoryText("manifest.051"),
        p("env"),
        { name: "", value: "" },
        (row, environment) => (
          <>
            {text(hostRepositoryText("manifest.052"), [...row, "name"], {
              required: true,
            })}
            <AutoField
              label={hostRepositoryText("manifest.053")}
              className="flex flex-col gap-1.5 text-sm"
            >
              <select
                aria-label={hostRepositoryText("manifest.053")}
                className="h-9 rounded-md border border-input bg-background px-2"
                disabled={readOnly}
                value={
                  environment.valueFrom !== undefined ? "reference" : "literal"
                }
                onChange={(e) =>
                  change([
                    {
                      path: [...row, "value"],
                      value: e.target.value === "literal" ? "" : undefined,
                    },
                    {
                      path: [...row, "valueFrom"],
                      value:
                        e.target.value === "reference"
                          ? "variable/"
                          : undefined,
                    },
                  ])
                }
              >
                <option value="literal">
                  {hostRepositoryText("manifest.054")}
                </option>
                <option value="reference">
                  {hostRepositoryText("manifest.055")}
                </option>
              </select>
            </AutoField>
            {environment.valueFrom !== undefined
              ? text(
                  hostRepositoryText("manifest.056"),
                  [...row, "valueFrom"],
                  {
                    required: true,
                    hint: hostRepositoryText("manifest.057"),
                  },
                )
              : text(hostRepositoryText("manifest.058"), [...row, "value"], {
                  required: true,
                  multiline: true,
                })}
          </>
        ),
      )}
      {rows(
        hostRepositoryText("manifest.059"),
        p("volumes"),
        { name: "", path: "" },
        (row) => (
          <>
            {text(hostRepositoryText("manifest.060"), [...row, "name"], {
              required: true,
            })}
            {text(hostRepositoryText("manifest.061"), [...row, "path"], {
              required: true,
            })}
          </>
        ),
      )}
      {rows(
        hostRepositoryText("manifest.062"),
        p("files"),
        { path: "", fromTemplate: "" },
        (row, file) => (
          <>
            {text(hostRepositoryText("manifest.063"), [...row, "path"], {
              required: true,
            })}
            {text(
              hostRepositoryText("manifest.064"),
              [...row, "fromTemplate"],
              { required: true },
            )}
            <JsonField
              fieldId={JSON.stringify(row)}
              report={reportJson}
              label={hostRepositoryText("manifest.065")}
              value={file.values ?? {}}
              readOnly={readOnly}
              onChange={(value) => set([...row, "values"], value)}
            />
          </>
        ),
      )}
      <details className="space-y-4">
        <summary className="cursor-pointer text-sm font-medium">
          {hostRepositoryText("manifest.066")}
        </summary>
        <section
          aria-label={hostRepositoryText("manifest.067")}
          className="space-y-2"
        >
          <h4 className="text-sm font-medium">
            {hostRepositoryText("manifest.068")}
          </h4>
          <p className="text-xs text-muted-foreground">
            {hostRepositoryText("manifest.069")}
          </p>
          {((service.command ?? []) as string[]).map((_, i) => (
            <div key={i} className="flex items-end gap-2">
              <div className="min-w-0 flex-1">
                {text(
                  hostRepositoryText("manifest.070", { value1: i + 1 }),
                  p("command", i),
                  {
                    required: true,
                  },
                )}
              </div>
              {!readOnly && (
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  aria-label={hostRepositoryText("manifest.071", {
                    value1: i + 1,
                  })}
                  title={hostRepositoryText("manifest.071", { value1: i + 1 })}
                  onClick={() =>
                    setPendingRemoval({
                      path: p("command"),
                      index: i,
                      action: hostRepositoryText("manifest.072"),
                      target: hostRepositoryText("manifest.073", {
                        value1: String(
                          service.name || hostRepositoryText("manifest.026"),
                        ),
                        value2: i + 1,
                      }),
                    })
                  }
                >
                  <Trash2 className="size-4" />
                </Button>
              )}
            </div>
          ))}
          {!readOnly && (
            <Button
              type="button"
              variant="outline"
              onClick={() => {
                if (service.command == null) set(p("command"), [""]);
                else append(p("command"), "");
              }}
            >
              {hostRepositoryText("manifest.074")}
            </Button>
          )}
        </section>
        <fieldset className="space-y-2">
          <legend className="text-sm font-medium">
            {hostRepositoryText("manifest.075")}
          </legend>
          {mode === "stack" && (
            <p className="text-xs text-muted-foreground">
              {hostRepositoryText("manifest.076")}
            </p>
          )}
          {[
            ...new Set([
              ...services
                .filter((_, i) => i !== index)
                .map((s) => String(s.name)),
              ...dependencies,
            ]),
          ].map((name) => (
            <label
              key={name}
              className="mr-4 inline-flex items-center gap-2 text-sm"
            >
              <Checkbox
                disabled={readOnly}
                checked={dependencies.includes(name)}
                onCheckedChange={(checked) =>
                  set(
                    p("dependsOn"),
                    checked
                      ? [...dependencies, name]
                      : dependencies.filter((n) => n !== name),
                  )
                }
              />
              {name}
            </label>
          ))}
        </fieldset>
        {(
          [
            ["readiness", hostRepositoryText("manifest.077")],
            ["liveness", hostRepositoryText("manifest.078")],
          ] as const
        ).map(([key, label]) => {
          const healthPath = p("health", key);
          const enabled = plain(healthPath) != null;
          return (
            <section key={key} className="space-y-3">
              <label className="flex items-center gap-2 text-sm">
                <Switch
                  disabled={readOnly}
                  checked={enabled}
                  onCheckedChange={(checked) =>
                    set(
                      healthPath,
                      checked
                        ? {
                            httpGet: { path: "/", port: 80 },
                            periodSeconds: 10,
                            timeoutSeconds: 1,
                            failureThreshold: 3,
                          }
                        : undefined,
                    )
                  }
                />
                {label}（HTTP）
              </label>
              {enabled && (
                <div className={grid}>
                  {text(
                    hostRepositoryText("manifest.079", { value1: label }),
                    [...healthPath, "httpGet", "path"],
                    { required: true },
                  )}
                  <AutoField
                    label={hostRepositoryText("manifest-extra.004", {
                      value1: label,
                    })}
                    className="flex flex-col gap-1.5 text-sm"
                  >
                    <Input
                      disabled={readOnly}
                      value={String(
                        get([...healthPath, "httpGet", "port"]) ?? "",
                      )}
                      onChange={(e) =>
                        set(
                          [...healthPath, "httpGet", "port"],
                          /^\d+$/.test(e.target.value)
                            ? Number(e.target.value)
                            : e.target.value,
                        )
                      }
                    />
                  </AutoField>
                  {text(
                    hostRepositoryText("manifest.081", { value1: label }),
                    [...healthPath, "periodSeconds"],
                    { number: true, min: 1, max: 3600, fallback: 10 },
                  )}
                  {text(
                    hostRepositoryText("manifest.082", { value1: label }),
                    [...healthPath, "timeoutSeconds"],
                    { number: true, min: 1, max: 600, fallback: 1 },
                  )}
                  {text(
                    hostRepositoryText("manifest.083", { value1: label }),
                    [...healthPath, "failureThreshold"],
                    { number: true, min: 1, max: 100, fallback: 3 },
                  )}
                </div>
              )}
            </section>
          );
        })}
        <section className="space-y-3">
          <h4 className="text-sm font-medium">
            {hostRepositoryText("manifest.084")}
          </h4>
          <div className={grid}>
            {(
              [
                ["requests", hostRepositoryText("manifest.085")],
                ["limits", hostRepositoryText("manifest.086")],
              ] as const
            ).map(([key, label]) => (
              <div key={key} className="space-y-3">
                {text(`CPU ${label}`, p("resources", key, "cpu"), {
                  hint: hostRepositoryText("manifest.087"),
                })}
                {text(
                  hostRepositoryText("manifest.088", { value1: label }),
                  p("resources", key, "memory"),
                  {
                    hint: hostRepositoryText("manifest.089"),
                  },
                )}
              </div>
            ))}
          </div>
        </section>
      </details>
    </section>
  );
}
