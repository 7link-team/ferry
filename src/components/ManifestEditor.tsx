import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { parseDocument, visit } from "yaml";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { Checkbox } from "./ui/checkbox";

type Path = (string | number)[];
type Row = Record<string, unknown>;
const object = (value: unknown): value is Row =>
  !!value && typeof value === "object" && !Array.isArray(value);
const area =
  "min-h-24 w-full rounded-md border border-input bg-transparent p-3 font-mono text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";
const grid = "grid gap-3 sm:grid-cols-2";

/** Keep the YAML document, rather than rebuilding a partial manifest from form fields.
 * Unedited fields, comments, scalar types and mode-specific overrides survive edits.
 */
export function readManifest(source: string) {
  const doc = parseDocument(source, { uniqueKeys: true });
  if (doc.errors.length) throw new Error(doc.errors[0]!.message);
  let alias = false;
  visit(doc, {
    Alias: () => {
      alias = true;
    },
  });
  if (alias)
    throw new Error(
      "此配置使用 YAML 别名，请在源码模式编辑，或展开别名后使用表单。",
    );
  const value: unknown = doc.toJS({ maxAliasCount: 50 });
  if (
    !object(value) ||
    !object(value.metadata) ||
    !Array.isArray(value.services) ||
    !value.services.every(object)
  )
    throw new Error(
      "配置需要 metadata 对象和 services 服务数组，请先在源码中修正结构。",
    );
  const optionalObject = (parent: Row, key: string) => {
    if (parent[key] != null && !object(parent[key]))
      throw new Error(`${key} 必须是对象，请在源码中修正。`);
  };
  optionalObject(value, "strategy");
  optionalObject(value, "overrides");
  optionalObject(value.metadata, "labels");
  // Do not reinterpret malformed collection shapes as empty lists and lose data.
  for (const service of value.services as Row[]) {
    for (const name of ["ports", "env", "volumes", "files"]) {
      const rows = service[name];
      if (rows !== undefined && (!Array.isArray(rows) || !rows.every(object)))
        throw new Error(`${name} 必须是对象数组，请在源码中修正。`);
    }
    for (const name of ["command", "dependsOn"]) {
      const rows = service[name];
      if (
        rows != null &&
        (!Array.isArray(rows) || !rows.every((v) => typeof v === "string"))
      )
        throw new Error(`${name} 必须是字符串数组，请在源码中修正。`);
    }
    for (const name of ["artifact", "resources", "health"])
      optionalObject(service, name);
    if (object(service.resources))
      for (const key of ["requests", "limits"])
        optionalObject(service.resources, key);
    if (object(service.health))
      for (const key of ["readiness", "liveness"]) {
        optionalObject(service.health, key);
        const probe = service.health[key];
        if (object(probe)) optionalObject(probe, "httpGet");
      }
    for (const file of (service.files ?? []) as Row[])
      optionalObject(file, "values");
  }
  return { doc, value };
}

export function ManifestEditor({
  value,
  mode,
  readOnly,
  onChange,
  onValidityChange,
}: {
  value: string;
  mode: string;
  readOnly: boolean;
  onChange: (value: string) => void;
  onValidityChange: (blocked: boolean) => void;
}) {
  const [sourceMode, setSourceMode] = useState(false);
  const [notice, setNotice] = useState("");
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
  }, [value]);
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
  const change = (changes: { path: Path; value: unknown }[]) => {
    if (!doc || readOnly) return;
    const next = doc.clone();
    for (const change of changes) {
      // Optional objects may be explicitly null in existing documents.
      if (change.value !== undefined)
        for (let length = 1; length < change.path.length; length++) {
          const parent = change.path.slice(0, length);
          if (next.getIn(parent) == null)
            next.setIn(
              parent,
              next.createNode(
                typeof change.path[length] === "number" ? [] : {},
              ),
            );
        }
      if (
        change.path[0] === "strategy" &&
        change.value !== undefined &&
        !next.getIn(["strategy", "type"])
      )
        next.setIn(["strategy", "type"], "recreate");
      if (change.value === undefined) {
        if (next.hasIn(change.path)) next.deleteIn(change.path);
      } else
        next.setIn(
          change.path,
          typeof change.value === "object" && change.value !== null
            ? next.createNode(change.value)
            : change.value,
        );
    }
    setNotice("");
    onChange(next.toString());
  };
  const set = (path: Path, value: unknown) => change([{ path, value }]);
  const remove = (path: Path, index: number) => {
    if (!doc || readOnly) return;
    const next = doc.clone();
    next.deleteIn([...path, index]);
    setNotice("");
    onChange(next.toString());
  };
  const append = (path: Path, value: unknown) => {
    if (!doc || readOnly) return;
    const next = doc.clone();
    if (!next.hasIn(path)) next.setIn(path, next.createNode([]));
    next.addIn(path, next.createNode(value));
    setNotice("");
    onChange(next.toString());
  };
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
    const props = {
      "aria-label": label,
      value: String(current ?? options.fallback ?? ""),
      disabled: readOnly,
      onChange: (
        event: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement>,
      ) =>
        set(
          path,
          event.target.value === "" && !options.required
            ? undefined
            : options.number && event.target.value !== ""
              ? Number(event.target.value)
              : event.target.value,
        ),
    };
    return (
      <label className="flex min-w-0 flex-col gap-1.5 text-sm">
        {label}
        {options.multiline ? (
          <textarea {...props} className={area} />
        ) : (
          <Input
            {...props}
            required={options.required}
            type={options.number ? "number" : "text"}
            min={options.min}
            max={options.max}
            step={options.number ? 1 : undefined}
          />
        )}
        {options.hint && (
          <span className="text-xs text-muted-foreground">{options.hint}</span>
        )}
      </label>
    );
  };
  const select = (
    label: string,
    path: Path,
    choices: string[],
    fallback: string,
  ) => (
    <label className="flex flex-col gap-1.5 text-sm">
      {label}
      <select
        aria-label={label}
        className="h-9 rounded-md border border-input bg-background px-2"
        value={String(get(path) ?? fallback)}
        disabled={readOnly}
        onChange={(e) => set(path, e.target.value)}
      >
        {[...new Set([...choices, String(get(path) ?? fallback)])].map(
          (choice) => (
            <option key={choice}>{choice}</option>
          ),
        )}
      </select>
    </label>
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
              添加{title}
            </Button>
          )}
        </div>
        {items.length === 0 && (
          <p className="text-sm text-muted-foreground">未设置</p>
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
                size="sm"
                disabled={invalidJson}
                onClick={() => remove(path, index)}
              >
                删除{title} {index + 1}
              </Button>
            )}
          </fieldset>
        ))}
      </section>
    );
  };
  return (
    <section aria-label="应用配置编辑器" className="min-w-0 space-y-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h2 className="font-medium">应用配置</h2>
        <div className="flex gap-2" role="group" aria-label="编辑方式">
          <Button
            type="button"
            variant={!sourceMode ? "secondary" : "outline"}
            aria-pressed={!sourceMode}
            onClick={() => setSourceMode(false)}
          >
            表单编辑
          </Button>
          <Button
            type="button"
            variant={sourceMode ? "secondary" : "outline"}
            aria-pressed={sourceMode}
            disabled={invalidJson}
            onClick={() => setSourceMode(true)}
          >
            源码编辑
          </Button>
        </div>
      </div>
      {sourceMode || !parsed.data ? (
        <>
          <label className="flex flex-col gap-2 text-sm">
            应用配置源码（YAML / JSON）
            <textarea
              className={`${area} min-h-80`}
              value={value}
              readOnly={readOnly}
              spellCheck={false}
              onChange={(e) => onChange(e.target.value)}
              aria-invalid={syntaxError}
            />
          </label>
          {parsed.error && (
            <p
              role="alert"
              className="whitespace-pre-wrap text-sm text-destructive"
            >
              {parsed.error}
            </p>
          )}
          <p className="text-sm text-muted-foreground">
            切换不会覆盖源码。表单修改会输出 YAML
            并保留未修改字段；保存时校验当前部署模式支持的配置，并统一为
            JSON（不保留注释）。
          </p>
        </>
      ) : (
        <>
          <div className={grid}>
            {text("应用标识", ["metadata", "name"], {
              required: true,
              hint: "小写字母开头，可使用数字、短横线和下划线。",
            })}
          </div>
          <p className="text-sm text-muted-foreground">
            逐项配置服务。敏感值请引用变量；启动先后通过“依赖服务”设置。
          </p>
          {services.map((service, index) => {
            const path: Path = ["services", index];
            const p = (...keys: Path) => [...path, ...keys];
            const dependencies = (service.dependsOn ?? []) as string[];
            const artifact = object(service.artifact);
            return (
              <section
                key={index}
                aria-label={`服务 ${index + 1}`}
                className="space-y-5 rounded-md border p-4"
              >
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <h3 className="font-medium">
                    {index + 1}. {String(service.name || "未命名服务")}
                  </h3>
                  {!readOnly && (
                    <div className="flex flex-wrap gap-1">
                      <Button
                        type="button"
                        variant="ghost"
                        size="sm"
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
                            setNotice(
                              "其他服务仍依赖此服务，请先取消依赖再删除。",
                            );
                            return;
                          }
                          if (
                            confirm(
                              `删除服务“${String(service.name || index + 1)}”及其配置？保存前不会影响已部署服务。`,
                            )
                          )
                            remove(["services"], index);
                        }}
                      >
                        删除服务
                      </Button>
                    </div>
                  )}
                </div>
                <div className={grid}>
                  <label className="flex flex-col gap-1.5 text-sm">
                    服务名称
                    <Input
                      value={String(service.name ?? "")}
                      required
                      disabled={readOnly}
                      onChange={(e) => {
                        if (
                          services.some(
                            (other, otherIndex) =>
                              otherIndex !== index &&
                              other.name === e.target.value,
                          )
                        ) {
                          setNotice("服务名称不能重复，请使用不同名称。");
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
                  </label>
                  {text("副本数", p("replicas"), {
                    number: true,
                    min: 1,
                    max: 100,
                    fallback: 1,
                  })}
                  <label className="flex flex-col gap-1.5 text-sm">
                    运行来源
                    <select
                      aria-label="运行来源"
                      className="h-9 rounded-md border border-input bg-background px-2"
                      value={artifact ? "artifact" : "image"}
                      disabled={readOnly}
                      onChange={(e) => {
                        if (
                          !confirm(
                            "切换运行来源将替换当前镜像或构建产物配置，是否继续？",
                          )
                        )
                          return;
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
                      <option value="image">容器镜像</option>
                      <option value="artifact">流水线构建产物（Native）</option>
                    </select>
                  </label>
                  {!artifact &&
                    text("镜像", p("image"), {
                      required: true,
                      hint: "例如 nginx:alpine 或带 sha256 摘要的镜像地址。",
                    })}
                </div>
                {(mode === "native") !== artifact && (
                  <p role="status" className="text-sm text-destructive">
                    {mode === "native"
                      ? "Native 模式需要选择流水线构建产物。"
                      : "当前容器模式需要选择容器镜像。"}
                  </p>
                )}
                {artifact && (
                  <div className={grid}>
                    {text("流水线 ID", p("artifact", "pipeline"), {
                      required: true,
                    })}
                    {text("产物相对路径", p("artifact", "path"), {
                      required: true,
                    })}
                    {text("固定构建执行 ID", p("artifact", "run"), {
                      hint: "留空使用可用的最新成功构建。",
                    })}
                  </div>
                )}
                {rows(
                  "端口",
                  p("ports"),
                  { container: 80, protocol: "tcp" },
                  (row) => (
                    <>
                      {text("容器／服务端口", [...row, "container"], {
                        number: true,
                        min: 1,
                        max: 65535,
                        required: true,
                      })}
                      {text("主机发布端口", [...row, "published"], {
                        number: true,
                        min: 1,
                        max: 65535,
                      })}
                      {text("端口名称", [...row, "name"])}
                      {select(
                        "协议",
                        [...row, "protocol"],
                        ["tcp", "udp"],
                        "tcp",
                      )}
                    </>
                  ),
                )}
                {rows(
                  "环境变量",
                  p("env"),
                  { name: "", value: "" },
                  (row, environment) => (
                    <>
                      {text("变量名称", [...row, "name"], { required: true })}
                      <label className="flex flex-col gap-1.5 text-sm">
                        取值方式
                        <select
                          aria-label="取值方式"
                          className="h-9 rounded-md border border-input bg-background px-2"
                          disabled={readOnly}
                          value={
                            environment.valueFrom !== undefined
                              ? "reference"
                              : "literal"
                          }
                          onChange={(e) =>
                            change([
                              {
                                path: [...row, "value"],
                                value:
                                  e.target.value === "literal" ? "" : undefined,
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
                          <option value="literal">普通文本</option>
                          <option value="reference">引用已保存变量</option>
                        </select>
                      </label>
                      {environment.valueFrom !== undefined
                        ? text("变量引用", [...row, "valueFrom"], {
                            required: true,
                            hint: "格式 variable/变量名，不在此填写密钥明文。",
                          })
                        : text("变量值", [...row, "value"], {
                            required: true,
                            multiline: true,
                          })}
                    </>
                  ),
                )}
                {rows("数据卷", p("volumes"), { name: "", path: "" }, (row) => (
                  <>
                    {text("卷名称", [...row, "name"], { required: true })}
                    {text("挂载绝对路径", [...row, "path"], { required: true })}
                  </>
                ))}
                {rows(
                  "配置文件",
                  p("files"),
                  { path: "", fromTemplate: "" },
                  (row, file) => (
                    <>
                      {text("文件绝对路径", [...row, "path"], {
                        required: true,
                      })}
                      {text(
                        "配置文件模板 ID 或名称",
                        [...row, "fromTemplate"],
                        { required: true },
                      )}
                      <JsonField
                        fieldId={JSON.stringify(row)}
                        report={reportJson}
                        label="配置文件参数"
                        value={file.values ?? {}}
                        readOnly={readOnly}
                        onChange={(value) => set([...row, "values"], value)}
                      />
                    </>
                  ),
                )}
                <details className="space-y-4">
                  <summary className="cursor-pointer text-sm font-medium">
                    高级配置：命令、依赖、健康检查与资源限制
                  </summary>
                  <section aria-label="启动命令" className="space-y-2">
                    <h4 className="text-sm font-medium">启动命令和参数</h4>
                    <p className="text-xs text-muted-foreground">
                      每项是一个参数，空格和换行原样保留。
                    </p>
                    {((service.command ?? []) as string[]).map((_, i) => (
                      <div key={i} className="flex items-end gap-2">
                        <div className="min-w-0 flex-1">
                          {text(`命令参数 ${i + 1}`, p("command", i), {
                            required: true,
                          })}
                        </div>
                        {!readOnly && (
                          <Button
                            type="button"
                            variant="ghost"
                            onClick={() => remove(p("command"), i)}
                          >
                            删除参数 {i + 1}
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
                        添加命令参数
                      </Button>
                    )}
                  </section>
                  <fieldset className="space-y-2">
                    <legend className="text-sm font-medium">依赖服务</legend>
                    {mode === "stack" && (
                      <p className="text-xs text-muted-foreground">
                        Swarm 不支持启动依赖，请让应用自身处理重连。
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
                      ["readiness", "就绪检查"],
                      ["liveness", "存活检查"],
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
                              `${label}路径`,
                              [...healthPath, "httpGet", "path"],
                              { required: true },
                            )}
                            <label className="flex flex-col gap-1.5 text-sm">
                              {label}端口
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
                            </label>
                            {text(
                              `${label}周期（秒）`,
                              [...healthPath, "periodSeconds"],
                              { number: true, min: 1, max: 3600, fallback: 10 },
                            )}
                            {text(
                              `${label}超时（秒）`,
                              [...healthPath, "timeoutSeconds"],
                              { number: true, min: 1, max: 600, fallback: 1 },
                            )}
                            {text(
                              `${label}失败阈值`,
                              [...healthPath, "failureThreshold"],
                              { number: true, min: 1, max: 100, fallback: 3 },
                            )}
                          </div>
                        )}
                      </section>
                    );
                  })}
                  <section className="space-y-3">
                    <h4 className="text-sm font-medium">资源限制</h4>
                    <div className={grid}>
                      {(
                        [
                          ["requests", "预留"],
                          ["limits", "上限"],
                        ] as const
                      ).map(([key, label]) => (
                        <div key={key} className="space-y-3">
                          {text(`CPU ${label}`, p("resources", key, "cpu"), {
                            hint: "例如 0.5 或 500m",
                          })}
                          {text(`内存${label}`, p("resources", key, "memory"), {
                            hint: "例如 256Mi 或 1Gi",
                          })}
                        </div>
                      ))}
                    </div>
                  </section>
                </details>
              </section>
            );
          })}
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
              添加服务
            </Button>
          )}
          <details className="space-y-3">
            <summary className="cursor-pointer text-sm font-medium">
              应用高级配置：更新策略、标签与模式覆盖
            </summary>
            <div className={grid}>
              {select(
                "更新策略",
                ["strategy", "type"],
                mode === "stack" ? ["recreate", "rolling"] : ["recreate"],
                "recreate",
              )}
              {text("等待健康检查（秒）", ["strategy", "healthTimeout"], {
                number: true,
                min: 1,
                max: 600,
                fallback: 120,
              })}
              {mode === "stack" &&
                text("最大不可用副本数", ["strategy", "maxUnavailable"], {
                  number: true,
                  min: 0,
                  max: 100,
                })}
            </div>
            <JsonField
              fieldId="labels"
              report={reportJson}
              label="应用标签"
              value={plain(["metadata", "labels"]) ?? {}}
              readOnly={readOnly}
              onChange={(value) => set(["metadata", "labels"], value)}
            />
            <p className="text-xs text-muted-foreground">
              模式覆盖是高级 JSON 对象，会覆盖生成的部署配置；普通部署无需填写。
            </p>
            <JsonField
              fieldId="overrides"
              report={reportJson}
              label="模式覆盖"
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

function JsonField({
  label,
  fieldId,
  value,
  readOnly,
  onChange,
  report,
}: {
  label: string;
  fieldId: string;
  value: unknown;
  readOnly: boolean;
  onChange: (value: Row) => void;
  report: (key: string, invalid: boolean) => void;
}) {
  const serialized = JSON.stringify(value, null, 2);
  const [text, setText] = useState(serialized);
  const [error, setError] = useState("");
  const sent = useRef(serialized);
  useEffect(() => {
    if (sent.current !== serialized) {
      setText(serialized);
      setError("");
      sent.current = serialized;
    }
  }, [serialized]);
  useEffect(() => {
    report(fieldId, !!error);
    return () => report(fieldId, false);
  }, [fieldId, error, report]);
  return (
    <label className="flex min-w-0 flex-col gap-1.5 text-sm sm:col-span-2">
      {label}（JSON 对象）
      <textarea
        className={area}
        aria-label={`${label}（JSON 对象）`}
        value={text}
        readOnly={readOnly}
        aria-invalid={!!error}
        ref={(element) => element?.setCustomValidity(error)}
        onChange={(e) => {
          setText(e.target.value);
          try {
            const result: unknown = JSON.parse(e.target.value);
            if (!object(result)) throw new Error("请填写 JSON 对象。");
            setError("");
            sent.current = JSON.stringify(result, null, 2);
            onChange(result);
          } catch {
            setError("请填写有效的 JSON 对象；修正前无法保存或切换源码。");
          }
        }}
      />
      {error && (
        <span role="alert" className="text-destructive">
          {error}
        </span>
      )}
    </label>
  );
}
