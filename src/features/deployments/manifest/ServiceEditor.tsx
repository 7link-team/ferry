import { Trash2 } from "lucide-react";
import { AutoField } from "../../../shared/forms/auto-form";
import { hostRepositoryText } from "../../../shared/i18n/i18n";
import { Button } from "../../../shared/ui/button";
import { Checkbox } from "../../../shared/ui/checkbox";
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
            value={artifact ? "artifact" : "image"}
            disabled={readOnly}
            onChange={(e) => {
              // Switching is an ordinary draft edit: it takes effect
              // only when saved and can be undone by switching back.
              change([
                {
                  path: p("image"),
                  value: e.target.value === "image" ? "" : undefined,
                },
                {
                  path: p("artifact"),
                  value:
                    e.target.value === "artifact"
                      ? { source: "pipeline", pipeline: "", path: "" }
                      : undefined,
                },
              ]);
            }}
          >
            <option value="image">{hostRepositoryText("manifest.034")}</option>
            <option value="artifact">
              {hostRepositoryText("manifest.035")}
            </option>
          </select>
        </AutoField>
        {!artifact &&
          text(hostRepositoryText("manifest.036"), p("image"), {
            required: true,
            hint: hostRepositoryText("manifest.037"),
          })}
      </div>
      {(mode === "native") !== artifact && (
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
                <Checkbox
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
