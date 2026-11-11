import { Switch } from "../../shared/ui/switch";
import { NotificationParameters } from "../notifications/NotificationParameters";
import { WorkflowAssetContext } from "./WorkflowAssets";
import { FieldCaption, FieldHelpContext } from "../../shared/forms/FieldHelp";
import { workflowFieldHelp } from "./workflow-field-help";
import { Plus, X } from "lucide-react";
import {
  createContext,
  useContext,
  useEffect,
  useId,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { workflowText as t } from "../../shared/i18n/i18n";
import { Button } from "../../shared/ui/button";
import { Input } from "../../shared/ui/input";
import { manifest, type GraphNode } from "./graph";
import { WorkflowAssetUpload, type WorkflowAsset } from "./WorkflowAssets";
import { WorkflowAIParameters } from "./WorkflowAIParameters";
import { WorkflowIntegrationParameters } from "./WorkflowIntegrationParameters";
import { integrationManifest } from "./integrations";

const control =
  "h-10 w-full min-w-0 rounded-md border border-input bg-background px-3 text-sm";
type Params = Record<string, unknown>;
export const WorkflowInputFields = createContext<string[]>([]);
type TextKey = Parameters<typeof t>[0];
type Rule = { left: unknown; operator: string; right?: unknown };
type Condition = { combinator: string; rules: Rule[] };
type Shape = {
  type?: string;
  required?: string[];
  properties?: Record<string, Shape>;
  items?: Shape;
  enum?: unknown[];
};
function matchesShape(value: unknown, schema: Shape): boolean {
  if (schema.enum && !schema.enum.includes(value)) return false;
  if (
    schema.type === "object" &&
    (!value || typeof value !== "object" || Array.isArray(value))
  )
    return false;
  if (schema.type === "array" && !Array.isArray(value)) return false;
  if (schema.type === "integer" && !Number.isInteger(value)) return false;
  if (
    schema.type === "number" &&
    (typeof value !== "number" || !Number.isFinite(value))
  )
    return false;
  if (
    ["string", "boolean"].includes(schema.type ?? "") &&
    typeof value !== schema.type
  )
    return false;
  if (
    Array.isArray(value) &&
    schema.items &&
    !value.every((item) => matchesShape(item, schema.items!))
  )
    return false;
  if (value && !Array.isArray(value) && typeof value === "object") {
    const object = value as Params;
    if (schema.required?.some((key) => !Object.hasOwn(object, key)))
      return false;
    for (const [key, child] of Object.entries(schema.properties ?? {}))
      if (Object.hasOwn(object, key) && !matchesShape(object[key], child))
        return false;
  }
  return true;
}
function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="block min-w-0 space-y-2 text-sm">
      <span className="block font-medium"><FieldCaption label={<>{label}</>} /></span>
      {children}
    </label>
  );
}
function Choice({
  label,
  value,
  values,
  readOnly,
  change,
}: {
  label: string;
  value: string;
  values: readonly string[];
  readOnly: boolean;
  change: (value: string) => void;
}) {
  return (
    <Field label={label}>
      <select
        className={control}
        aria-label={label}
        value={value}
        disabled={readOnly}
        onChange={(event) => change(event.target.value)}
      >
        {values.map((value) => (
          <option key={value} value={value}>
            {t(`option.${value}` as TextKey)}
          </option>
        ))}
      </select>
    </Field>
  );
}
function Check({
  label,
  value,
  readOnly,
  change,
}: {
  label: string;
  value: boolean;
  readOnly: boolean;
  change: (value: boolean) => void;
}) {
  return (
    <label className="flex items-center gap-2 text-sm">
      <Switch
        checked={value}
        disabled={readOnly}
        onCheckedChange={(checked) => change(checked)}
      />
      <FieldCaption label={label} />
    </label>
  );
}
function Rows<T>({
  label,
  rows,
  create,
  change,
  readOnly,
  minimum = 0,
  children,
}: {
  label: string;
  rows: T[];
  create: () => T;
  change: (rows: T[]) => void;
  readOnly: boolean;
  minimum?: number;
  children: (
    row: T,
    index: number,
    patch: (value: Partial<T>) => void,
  ) => ReactNode;
}) {
  return (
    <section aria-label={label} className="space-y-3">
      {rows.map((row, index) => (
        <div key={index} className="space-y-3 rounded-lg border p-3">
          <div className="flex items-center justify-between gap-2">
            <span className="text-xs font-medium text-muted-foreground">
              {label} {index + 1}
            </span>
            {!readOnly && (
              <Button
                type="button"
                size="icon-sm"
                variant="ghost"
                aria-label={`${t("delete")} ${label} ${index + 1}`}
                disabled={rows.length <= minimum}
                onClick={() => change(rows.filter((_, at) => at !== index))}
              >
                <X />
              </Button>
            )}
          </div>
          {children(row, index, (value) =>
            change(
              rows.map((existing, at) =>
                at === index ? { ...existing, ...value } : existing,
              ),
            ),
          )}
        </div>
      ))}
      {!readOnly && (
        <Button
          type="button"
          variant="outline"
          className="w-full"
          onClick={() => change([...rows, create()])}
        >
          <Plus />
          {`${t("addRow")} ${label}`}
        </Button>
      )}
    </section>
  );
}

/** JSON is an explicit advanced tool; normal values and conditions use form controls. */
export function AdvancedJson({
  value,
  readOnly,
  change,
  invalid,
  validate = () => true,
}: {
  value: unknown;
  readOnly: boolean;
  change: (value: unknown) => void;
  invalid: (invalid: boolean) => void;
  validate?: (value: unknown) => boolean;
}) {
  const serialized = JSON.stringify(value, null, 2);
  const [source, setSource] = useState(serialized);
  const [error, setError] = useState(false);
  const committed = useRef(serialized),
    report = useRef(invalid);
  report.current = invalid;
  useEffect(() => {
    if (serialized !== committed.current) {
      setSource(serialized);
      committed.current = serialized;
      setError(false);
      report.current(false);
    }
  }, [serialized]);
  useEffect(() => () => report.current(false), []);
  return (
    <div className="space-y-2">
      <textarea
        aria-label={t("advanced")}
        className={`${control} min-h-40 py-3 font-mono text-xs`}
        value={source}
        readOnly={readOnly}
        onChange={(event) => {
          setSource(event.target.value);
          try {
            const next: unknown = JSON.parse(event.target.value);
            if (!validate(next)) throw new Error("Invalid parameter shape");
            change(next);
            committed.current = JSON.stringify(next, null, 2);
            setError(false);
            report.current(false);
          } catch {
            setError(true);
            report.current(true);
          }
        }}
      />
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {t("parameterError")}
        </p>
      )}
    </div>
  );
}

function Value({
  label,
  value,
  readOnly,
  change,
  invalid,
}: {
  label: string;
  value: unknown;
  readOnly: boolean;
  change: (value: unknown) => void;
  invalid: (invalid: boolean) => void;
}) {
  const inputFields = useContext(WorkflowInputFields);
  const fieldList = useId();
  const directField =
    typeof value === "string"
      ? value.match(/^=\{\{\s*\$json((?:\.[\w$]+)+)\s*\}\}$/)?.[1]?.slice(1)
      : undefined;
  const bracketField =
    typeof value === "string"
      ? value.match(/^=\{\{\s*\$json((?:\["(?:[^"\\]|\\.)*"\])+)\s*\}\}$/)?.[1]
      : undefined;
  const field =
    directField ??
    (bracketField === undefined
      ? undefined
      : [...bracketField.matchAll(/\[("(?:[^"\\]|\\.)*")\]/g)]
          .map((match) => JSON.parse(match[1]!))
          .join("."));
  const reference = (path: string) =>
    `={{ $json${path
      .split(".")
      .map((key) => `[${JSON.stringify(key)}]`)
      .join("")} }}`;
  const expression = typeof value === "string" && value.startsWith("={{");
  const type =
    field !== undefined
      ? "field"
      : expression
        ? "expression"
        : value === null
          ? "null"
          : Array.isArray(value)
            ? "array"
            : typeof value === "object"
            ? "object"
            : typeof value;
  return (
    <div className="space-y-2">
      <Choice
        label={`${label} · ${t("valueType")}`}
        value={type}
        values={[
          "string",
          "number",
          "boolean",
          "field",
          "expression",
          "null",
          "object",
          "array",
        ]}
        readOnly={readOnly}
        change={(type) => {
          invalid(false);
          change(
            type === "number"
              ? 0
              : type === "boolean"
                ? false
                : type === "null"
                  ? null
                  : type === "array"
                    ? []
                  : type === "object"
                    ? {}
                    : type === "field"
                      ? reference(inputFields[0] ?? "")
                      : type === "expression"
                        ? "={{ $json }}"
                        : "",
          );
        }}
      />
      {type === "field" ? (
        <Field label={label}>
          <Input
            aria-label={label}
            required
            readOnly={readOnly}
            list={fieldList}
            placeholder={t("inputFieldHelp")}
            value={field ?? ""}
            onChange={(event) => change(event.target.value.startsWith("=") ? event.target.value : reference(event.target.value))}
          />
          <datalist id={fieldList}>
            {inputFields.map((path) => (
              <option key={path} value={path} />
            ))}
          </datalist>
          <span className="block text-xs leading-5 text-muted-foreground">
            {t("inputFieldHelp")}
          </span>
        </Field>
      ) : type === "object" || type === "array" ? (
        <StructuredValue label={label} value={value} readOnly={readOnly} change={change} />
      ) : type === "boolean" ? (
        <Check
          label={label}
          value={value === true}
          readOnly={readOnly}
          change={change}
        />
      ) : (
        type !== "null" && (
          <Field label={label}>
            <Input
              aria-label={label}
              readOnly={readOnly}
              className={expression ? "font-mono text-sm" : ""}
              type={type === "number" ? "number" : "text"}
              step={type === "number" ? "any" : undefined}
              value={String(value ?? "")}
              onChange={(event) =>
                change(
                  type === "number"
                    ? Number(event.target.value)
                    : event.target.value,
                )
              }
            />
            {expression && (
              <span className="block text-xs leading-5 text-muted-foreground">
                {t("expressionHelp")}
              </span>
            )}
          </Field>
        )
      )}
    </div>
  );
}

export function StructuredValue({ label, value, readOnly, change }: {
  label: string; value: unknown; readOnly: boolean; change: (value: unknown) => void;
}) {
  const array = Array.isArray(value);
  const entries = array ? value.map((item, index) => [String(index), item] as const)
    : Object.entries((value ?? {}) as Params);
  const [names, setNames] = useState<Record<string, string>>({});
  return <section aria-label={label} className="space-y-3 border-l-2 pl-3">
    {entries.map(([key, item]) => <div key={key} className="space-y-2">
      <div className="flex items-center gap-2">
        {array ? <span className="text-sm">{Number(key) + 1}</span> : <Input
          aria-label={`${label} · ${t("field")} ${key}`} value={names[key] ?? key} readOnly={readOnly}
          onChange={event => setNames(current => ({ ...current, [key]: event.target.value }))}
          onBlur={() => {
            const name = names[key]?.trim();
            if (name && name !== key && !entries.some(([existing]) => existing === name))
              change(Object.fromEntries(entries.map(([existing, v]) => [existing === key ? name : existing, v])));
            setNames(current => { const next = { ...current }; delete next[key]; return next; });
          }} />}
        {!readOnly && <Button type="button" variant="ghost" size="icon-sm" aria-label={`${t("delete")} ${label}.${key}`}
          onClick={() => change(array ? (value as unknown[]).filter((_, index) => index !== Number(key)) : Object.fromEntries(entries.filter(([name]) => name !== key)))}><X /></Button>}
      </div>
      <Value label={`${label}.${key}`} value={item} readOnly={readOnly} change={next => change(array ? (value as unknown[]).map((v, index) => index === Number(key) ? next : v) : { ...(value as Params), [key]: next })} invalid={() => {}} />
    </div>)}
    {!readOnly && <Button type="button" variant="outline" onClick={() => {
      if (array) change([...(value as unknown[]), ""]);
      else { let key = "field", index = 2; while (entries.some(([name]) => name === key)) key = `field_${index++}`; change({ ...(value as Params), [key]: "" }); }
    }}><Plus />{t("addRow")} {label}</Button>}
  </section>;
}

function Conditions({
  value,
  readOnly,
  change,
  invalid,
}: {
  value: Condition;
  readOnly: boolean;
  change: (value: Condition) => void;
  invalid: (key: string, value: boolean) => void;
}) {
  return (
    <div className="space-y-4">
      <Choice
        label={t("matchRules")}
        value={value.combinator}
        values={["and", "or"]}
        readOnly={readOnly}
        change={(combinator) => change({ ...value, combinator })}
      />
      <Rows
        label={t("condition")}
        rows={value.rules}
        readOnly={readOnly}
        minimum={1}
        create={() => ({ left: "", operator: "eq", right: "" })}
        change={(rules) => change({ ...value, rules })}
      >
        {(rule, index, patch) => (
          <>
            <Value
              label={t("leftValue")}
              value={rule.left}
              readOnly={readOnly}
              change={(left) => patch({ left })}
              invalid={(value) => invalid(`${index}.left`, value)}
            />
            <Choice
              label={t("operator")}
              value={rule.operator}
              values={[
                "eq",
                "ne",
                "gt",
                "gte",
                "lt",
                "lte",
                "contains",
                "exists",
                "matches",
              ]}
              readOnly={readOnly}
              change={(operator) =>
                patch({
                  operator,
                  ...(operator !== "exists" && !("right" in rule)
                    ? { right: "" }
                    : {}),
                })
              }
            />
            {rule.operator !== "exists" && (
              <Value
                label={t("rightValue")}
                value={rule.right}
                readOnly={readOnly}
                change={(right) => patch({ right })}
                invalid={(value) => invalid(`${index}.right`, value)}
              />
            )}
          </>
        )}
      </Rows>
    </div>
  );
}

function SchemaFields({
  schema,
  readOnly,
  change,
}: {
  schema: Params;
  readOnly: boolean;
  change: (value: Params) => void;
}) {
  const properties = (schema.properties ?? {}) as Record<string, Params>;
  const required = (schema.required ?? []) as string[];
  const [draftNames, setDraftNames] = useState<Record<string, string>>({});
  return (
    <div className="space-y-3">
      <p className="text-sm leading-6 text-muted-foreground">
        {t("inputFieldsHelp")}
      </p>
      {Object.entries(properties).map(([name, property]) => (
        <div key={name} className="space-y-3 rounded-lg border p-3">
          <Field label={t("field")}>
            <Input
              aria-label={t("field")}
              readOnly={readOnly}
              value={draftNames[name] ?? name}
              onChange={(event) =>
                setDraftNames((current) => ({
                  ...current,
                  [name]: event.target.value,
                }))
              }
              onBlur={() => {
                const next = draftNames[name]?.trim();
                if (next && next !== name && !Object.hasOwn(properties, next)) {
                  change({
                    ...schema,
                    properties: Object.fromEntries(
                      Object.entries(properties).map(([key, value]) => [
                        key === name ? next : key,
                        value,
                      ]),
                    ),
                    required: required.map((key) =>
                      key === name ? next : key,
                    ),
                  });
                }
                setDraftNames((current) => {
                  const next = { ...current };
                  delete next[name];
                  return next;
                });
              }}
            />
          </Field>
          <Choice
            label={t("valueType")}
            value={typeof property.type === "string" ? property.type : "object"}
            values={[
              "string",
              "number",
              "integer",
              "boolean",
              "object",
              "array",
            ]}
            readOnly={readOnly}
            change={(type) =>
              change({
                ...schema,
                properties: { ...properties, [name]: { ...property, type } },
              })
            }
          />
          {property.type === "object" && <SchemaFields schema={property} readOnly={readOnly} change={next => change({ ...schema, properties: { ...properties, [name]: next } })} />}
          {property.type === "array" && <ArraySchema schema={property} readOnly={readOnly} change={next => change({ ...schema, properties: { ...properties, [name]: next } })} />}
          <div className="flex items-center justify-between">
            <Check
              label={t("required")}
              readOnly={readOnly}
              value={required.includes(name)}
              change={(checked) =>
                change({
                  ...schema,
                  required: checked
                    ? [...required.filter((key) => key !== name), name]
                    : required.filter((key) => key !== name),
                })
              }
            />
            {!readOnly && (
              <Button
                type="button"
                variant="ghost"
                size="icon-sm"
                aria-label={`${t("delete")} ${name}`}
                onClick={() =>
                  change({
                    ...schema,
                    properties: Object.fromEntries(
                      Object.entries(properties).filter(
                        ([key]) => key !== name,
                      ),
                    ),
                    required: required.filter((key) => key !== name),
                  })
                }
              >
                <X />
              </Button>
            )}
          </div>
        </div>
      ))}
      {!readOnly && (
        <Button
          type="button"
          variant="outline"
          className="w-full"
          onClick={() => {
            let name = "field",
              index = 2;
            while (Object.hasOwn(properties, name)) name = `field_${index++}`;
            change({
              ...schema,
              type: "object",
              properties: { ...properties, [name]: { type: "string" } },
            });
          }}
        >
          <Plus />
          {t("addInput")}
        </Button>
      )}
    </div>
  );
}

function ArraySchema({ schema, readOnly, change }: { schema: Params; readOnly: boolean; change: (value: Params) => void }) {
  const items = (schema.items ?? { type: "string" }) as Params;
  return <div className="space-y-3 border-l-2 pl-3">
    <Choice label={t("option.array") + " ? " + t("valueType")} value={String(items.type ?? "string")} values={["string", "number", "integer", "boolean", "object", "array"]} readOnly={readOnly} change={type => change({ ...schema, items: { ...items, type } })} />
    {items.type === "object" && <SchemaFields schema={items} readOnly={readOnly} change={next => change({ ...schema, items: next })} />}
    {items.type === "array" && <ArraySchema schema={items} readOnly={readOnly} change={next => change({ ...schema, items: next })} />}
  </div>;
}

export const validRunInputs = (schema: Params, value: Params) =>
  matchesShape(value, schema as Shape);
function defaultInput(schema: Shape): unknown {
  if (schema.enum?.length) return schema.enum[0];
  if (schema.type === "array") return [];
  if (schema.type === "object") return Object.fromEntries(
    Object.entries(schema.properties ?? {}).filter(([key]) => schema.required?.includes(key)).map(([key, child]) => [key, defaultInput(child)]),
  );
  return schema.type === "boolean" ? false : ["number", "integer"].includes(schema.type ?? "") ? 0 : "";
}
function RunInput({ name, schema, value, change }: {
  name: string; schema: Shape; value: unknown; change: (value: unknown) => void;
}) {
  if (schema.enum) return <select className={control} aria-label={name} value={value === undefined ? "" : String(schema.enum.indexOf(value))}
    onChange={event => change(event.target.value === "" ? undefined : schema.enum![Number(event.target.value)])}>
    <option value="">{t("selectValue")}</option>{schema.enum.map((item, index) => <option key={index} value={index}>{String(item)}</option>)}</select>;
  if (schema.type === "object") {
    if (!Object.keys(schema.properties ?? {}).length) return <StructuredValue label={name} value={value ?? {}} readOnly={false} change={change} />;
    return <div className="space-y-3 border-l-2 pl-3">{Object.entries(schema.properties ?? {}).map(([key, child]) => <div key={key} className="space-y-2">
      <span className="block text-sm font-medium">{key}{schema.required?.includes(key) ? ` ? ${t("required")}` : ""}</span>
      <RunInput name={`${name}.${key}`} schema={child} value={(value as Params | undefined)?.[key]} change={next => {
        const object = { ...((value ?? {}) as Params) }; if (next === undefined) delete object[key]; else object[key] = next; change(object);
      }} />
    </div>)}</div>;
  }
  if (schema.type === "array") {
    const list = Array.isArray(value) ? value : [];
    return <section aria-label={name} className="space-y-3 border-l-2 pl-3">
      {list.map((item, index) => <div key={index} className="space-y-2">
        <div className="flex items-center justify-between"><span className="text-sm">{index + 1}</span><Button type="button" variant="ghost" size="icon-sm" aria-label={`${t("delete")} ${name}.${index}`} onClick={() => change(list.filter((_, at) => at !== index))}><X /></Button></div>
        <RunInput name={`${name}.${index}`} schema={schema.items ?? { type: "string" }} value={item} change={next => change(list.map((existing, at) => at === index ? next ?? defaultInput(schema.items ?? { type: "string" }) : existing))} />
      </div>)}
      <Button type="button" variant="outline" onClick={() => change([...list, defaultInput(schema.items ?? { type: "string" })])}><Plus />{t("addRow")} {name}</Button>
    </section>;
  }
  if (schema.type === "boolean") return <select className={control} aria-label={name} value={value === undefined ? "" : String(value)} onChange={event => change(event.target.value === "" ? undefined : event.target.value === "true")}>
    <option value="">{t("selectValue")}</option><option value="true">{t("yes")}</option><option value="false">{t("no")}</option></select>;
  return <Input aria-label={name} type={["number", "integer"].includes(schema.type ?? "") ? "number" : "text"} step={schema.type === "integer" ? "1" : "any"} value={String(value ?? "")} onChange={event => change(event.target.value === "" ? undefined : ["number", "integer"].includes(schema.type ?? "") ? Number(event.target.value) : event.target.value)} />;
}
export function WorkflowRunInputs({ schema, value, change, invalid }: {
  schema: Params; value: Params; change: (value: Params) => void; invalid: (value: boolean) => void;
}) {
  const properties = (schema.properties ?? {}) as Record<string, Shape>;
  const required = (schema.required ?? []) as string[];
  return <div className="space-y-4">
    {Object.entries(properties).map(([name, field]) => <div key={name} className="space-y-2">
      <span className="block text-sm font-medium">{name}{required.includes(name) ? ` ? ${t("required")}` : ""}</span>
      <RunInput name={name} schema={field} value={value[name]} change={next => { const object = { ...value }; if (next === undefined) delete object[name]; else object[name] = next; change(object); }} />
    </div>)}
    {!Object.keys(properties).length && <p className="text-sm text-muted-foreground">{t("noRunInputs")}</p>}
    <details><summary className="cursor-pointer text-sm text-muted-foreground">{t("advanced")}</summary>
      <p className="my-3 text-xs leading-5 text-muted-foreground">{t("advancedHelp")}</p>
      <AdvancedJson value={value} readOnly={false} change={next => change(next as Params)} invalid={invalid} validate={next => matchesShape(next, schema as Shape)} />
    </details>
  </div>;
}

export function WorkflowNodeParameters({
  node,
  readOnly,
  update,
  invalid,
}: {
  node: GraphNode;
  readOnly: boolean;
  update: (value: Params) => void;
  invalid: (key: string, value: boolean) => void;
}) {
  const { workspace } = useContext(WorkflowAssetContext);
  const p = node.params;
  const patch = (value: Params) => update({ ...p, ...value });
  const text = (key: string, label: TextKey) => (
    <Field label={t(label)}>
      <Input
        aria-label={t(label)}
        readOnly={readOnly}
        value={String(p[key] ?? "")}
        onChange={(event) => patch({ [key]: event.target.value })}
      />
    </Field>
  );
  const number = (key: string, label: TextKey, min = 0) => (
    <Field label={t(label)}>
      <Input
        aria-label={t(label)}
        type="number"
        min={min}
        step="1"
        required
        readOnly={readOnly}
        value={String(p[key] ?? min)}
        onChange={(event) => patch({ [key]: Number(event.target.value) })}
      />
    </Field>
  );
  const choice = (
    key: string,
    label: TextKey,
    values: readonly string[],
    extra?: (value: string) => Params,
  ) => (
    <Choice
      label={t(label)}
      value={String(p[key] ?? values[0])}
      values={values}
      readOnly={readOnly}
      change={(value) => patch({ [key]: value, ...extra?.(value) })}
    />
  );
  const condition = (
    value: Condition,
    change: (value: Condition) => void,
    prefix = "",
  ) => (
    <Conditions
      value={value}
      change={change}
      readOnly={readOnly}
      invalid={(key, value) => invalid(prefix + key, value)}
    />
  );
  const pairs = (key: string, label: TextKey) => (
    <Rows
      label={t(label)}
      rows={(p[key] ?? []) as { from: string; to: string }[]}
      readOnly={readOnly}
      minimum={key === "keys" ? 1 : 0}
      create={() => ({ from: "", to: "" })}
      change={(rows) => patch({ [key]: rows })}
    >
      {(row, _, change) => (
        <>
          <Field label={t("fromField")}>
            <Input
              aria-label={t("fromField")}
              value={row.from}
              readOnly={readOnly}
              onChange={(event) => change({ from: event.target.value })}
            />
          </Field>
          <Field label={t("toField")}>
            <Input
              aria-label={t("toField")}
              value={row.to}
              readOnly={readOnly}
              onChange={(event) => change({ to: event.target.value })}
            />
          </Field>
        </>
      )}
    </Rows>
  );
  let fields: ReactNode;
  switch (node.type) {
    case "notification.send":
      fields = <NotificationParameters workspace={workspace} value={p} readOnly={readOnly} onChange={update} invalid={blocked => invalid("notification", blocked)} />;
      break;
    case "host.command":
      fields = <>
        <p className="text-sm leading-6 text-muted-foreground">{t("hostCommandHelp")}</p>
        <Field label={t("commandShell")}>
          <select aria-label={t("commandShell")} className={control} disabled={readOnly} value={String(p.shell ?? "sh")} onChange={event=>patch({shell:event.target.value})}>
            <option value="sh">sh</option>
            <option value="auto">{t("commandShellAuto")}</option>
            {!["sh","auto"].includes(String(p.shell)) && <option value={String(p.shell)} disabled>{String(p.shell)}</option>}
          </select>
        </Field>
        <Field label={t("commandScript")}>
          <textarea aria-label={t("commandScript")} className="min-h-48 w-full rounded-md border border-input bg-background p-3 font-mono text-sm" readOnly={readOnly} value={String(p.command??"")} onChange={event=>patch({command:event.target.value})}/>
        </Field>
        <StructuredValue label={t("commandInputEnv")} value={p.input_env??{}} readOnly={readOnly} change={value=>patch({input_env:value})}/>
      </>;
      break;
    case "host.preview":
      fields = <>
        <p className="text-sm leading-6 text-muted-foreground">{t("hostPreviewHelp")}</p>
        {text("host_id", "previewHost")}
        <p className="text-xs leading-5 text-muted-foreground">{t("previewHostHelp")}</p>
        <Field label={t("previewCommand")}>
          <textarea aria-label={t("previewCommand")} maxLength={4000} className="min-h-32 w-full rounded-md border border-input bg-background p-3 font-mono text-sm" readOnly={readOnly} value={String(p.command??"")} onChange={event=>patch({command:event.target.value})}/>
        </Field>
        <StructuredValue label={t("commandInputEnv")} value={p.input_env??{}} readOnly={readOnly} change={value=>patch({input_env:value})}/>
        {number("port", "previewPort", 1)}
        {text("name", "previewName")}
        {number("ready_timeout_seconds", "previewReadyTimeout", 1)}
        <Field label={t("previewReview")}>
          <select aria-label={t("previewReview")} className={control} disabled={readOnly} value={p.review === "human" ? "human" : "none"} onChange={event=>patch({review:event.target.value})}>
            <option value="none">{t("previewReviewNone")}</option>
            <option value="human">{t("previewReviewHuman")}</option>
          </select>
        </Field>
        {p.review === "human" && number("review_timeout_minutes", "previewReviewTimeout", 1)}
      </>;
      break;
    case "ai.llm.chat":
    case "ai.image.generate":
    case "ai.audio.speech":
    case "ai.audio.transcribe":
      fields = (
        <WorkflowAIParameters
          type={node.type}
          value={p}
          readOnly={readOnly}
          change={update}
        />
      );
      break;
    case "file.input":
    case "image.input":
    case "audio.input":
    case "video.input":
      fields = (
        <>
          <WorkflowAssetUpload
            file={p.file as WorkflowAsset}
            kind={node.type.split(".")[0]!}
            readOnly={readOnly}
            change={(file) => patch({ file })}
            busyChange={(busy) => invalid("upload", busy)}
          />
          {text("field", "binaryField")}
        </>
      );
      break;
    case "file.encode":
      fields = (
        <>
          {choice("format", "fileFormat", ["json", "text"], (format) => {
            const next: Params = { ...p, format };
            if (format === "text") next.value_path = "/text";
            else delete next.value_path;
            return next;
          })}
          {p.format === "text" && (
            <Field label={t("fileValuePath")}>
              <Input
                disabled={readOnly}
                value={String(p.value_path ?? "")}
                onChange={(event) => {
                  const next = { ...p };
                  if (event.target.value) next.value_path = event.target.value;
                  else delete next.value_path;
                  update(next);
                }}
              />
            </Field>
          )}
          {text("output_name", "fileName")}
          {text("output_field", "binaryField")}
        </>
      );
      break;
    case "file.decode":
      fields = (
        <>
          {choice("format", "fileFormat", ["text", "json"])}
          <p className="text-xs text-muted-foreground">{t("decodeHelp")}</p>
          {text("asset", "inputFile")}
          {text("output_field", "outputField")}
        </>
      );
      break;
    case "core.trigger.manual":
      fields = (
        <SchemaFields
          schema={p.input_schema as Params}
          readOnly={readOnly}
          change={(input_schema) => patch({ input_schema })}
        />
      );
      break;
    case "data.set":
      fields = (
        <>
          <Rows
            label={t("field")}
            rows={p.fields as { path: string; value: unknown }[]}
            readOnly={readOnly}
            create={() => ({ path: "", value: "" })}
            change={(fields) => patch({ fields })}
          >
            {(row, index, change) => (
              <>
                <Field label={t("fieldPath")}>
                  <Input
                    aria-label={t("fieldPath")}
                    value={row.path}
                    readOnly={readOnly}
                    onChange={(event) => change({ path: event.target.value })}
                  />
                </Field>
                <Value
                  label={t("value")}
                  value={row.value}
                  readOnly={readOnly}
                  change={(value) => change({ value })}
                  invalid={(value) => invalid(`field.${index}`, value)}
                />
              </>
            )}
          </Rows>
          <Check
            label={t("keepFields")}
            value={p.keep_other_fields === true}
            readOnly={readOnly}
            change={(keep_other_fields) => patch({ keep_other_fields })}
          />
        </>
      );
      break;
    case "core.if":
    case "core.filter":
      fields = condition(p as unknown as Condition, (value) =>
        update({ ...p, ...value }),
      );
      break;
    case "core.switch":
      fields = (
        <>
          {choice("match", "matching", ["first", "all"])}
          <Rows
            label={t("branch")}
            rows={p.cases as { key: string; condition: Condition }[]}
            readOnly={readOnly}
            minimum={1}
            create={() => {
              const names = new Set(
                (p.cases as { key: string }[]).map((branch) => branch.key),
              );
              let index = 1;
              while (names.has(`branch_${index}`)) index++;
              return {
                key: `branch_${index}`,
                condition: {
                  combinator: "and",
                  rules: [{ left: "", operator: "eq", right: "" }],
                },
              };
            }}
            change={(cases) => patch({ cases })}
          >
            {(row, index, change) => (
              <>
                <Field label={t("branchName")}>
                  <Input
                    aria-label={t("branchName")}
                    value={row.key}
                    readOnly={readOnly}
                    onChange={(event) => change({ key: event.target.value })}
                  />
                </Field>
                {condition(
                  row.condition,
                  (condition) => change({ condition }),
                  `case.${index}.`,
                )}
              </>
            )}
          </Rows>
          <Check
            label={t("fallback")}
            value={p.fallback === true}
            readOnly={readOnly}
            change={(fallback) => patch({ fallback })}
          />
        </>
      );
      break;
    case "core.merge":
      fields = (
        <>
          {choice(
            "mode",
            "mergeMode",
            ["append", "position", "fields", "join"],
            (mode) => ({
              ...(["fields", "join"].includes(mode) && !p.keys
                ? { keys: [{ from: "", to: "" }] }
                : {}),
              ...(mode === "join" && !p.join ? { join: "inner" } : {}),
            }),
          )}
          {number("input_count", "inputCount", 1)}
          {choice("wait", "waitFor", ["any_items", "all_inputs"])}
          {["fields", "join"].includes(String(p.mode)) &&
            pairs("keys", "joinKey")}
          {p.mode === "join" &&
            choice("join", "joinMode", ["inner", "left", "right", "full"])}
        </>
      );
      break;
    case "core.wait":
      fields = (
        <>
          {choice(
            "mode",
            "waitMode",
            ["duration", "until", "callback"],
            (mode) =>
              mode === "duration"
                ? { duration_ms: p.duration_ms ?? 1000 }
                : mode === "until"
                  ? { until: p.until ?? "" }
                  : {
                      response_schema: p.response_schema ?? { type: "object" },
                    },
          )}
          {p.mode === "duration" && number("duration_ms", "durationMs")}
          {p.mode === "until" && (
            <Field label={t("untilTime")}>
              <Input
                aria-label={t("untilTime")}
                type="datetime-local"
                readOnly={readOnly}
                value={
                  typeof p.until === "string" &&
                  !Number.isNaN(Date.parse(p.until))
                    ? new Date(
                        Date.parse(p.until) -
                          new Date(p.until).getTimezoneOffset() * 60000,
                      )
                        .toISOString()
                        .slice(0, 16)
                    : ""
                }
                onChange={(event) => {
                  if (event.target.value)
                    patch({
                      until: new Date(event.target.value).toISOString(),
                    });
                }}
              />
              <span className="block text-xs text-muted-foreground">
                {t("localTime")}
              </span>
            </Field>
          )}
          {p.mode === "callback" && (
            <SchemaFields
              schema={p.response_schema as Params}
              readOnly={readOnly}
              change={(response_schema) => patch({ response_schema })}
            />
          )}
        </>
      );
      break;
    case "core.limit":
      fields = (
        <>
          {number("count", "maxItems")}
          {number("offset", "skipItems")}
        </>
      );
      break;
    case "core.sort":
      fields = (
        <Rows
          label={t("sortKey")}
          rows={p.keys as { path: string; direction: string; nulls: string }[]}
          readOnly={readOnly}
          minimum={1}
          create={() => ({ path: "", direction: "asc", nulls: "last" })}
          change={(keys) => patch({ keys })}
        >
          {(row, _, change) => (
            <>
              <Field label={t("fieldPath")}>
                <Input
                  aria-label={t("fieldPath")}
                  value={row.path}
                  readOnly={readOnly}
                  onChange={(event) => change({ path: event.target.value })}
                />
              </Field>
              <Choice
                label={t("sortDirection")}
                value={row.direction}
                values={["asc", "desc"]}
                readOnly={readOnly}
                change={(direction) => change({ direction })}
              />
              <Choice
                label={t("emptyValues")}
                value={row.nulls}
                values={["first", "last"]}
                readOnly={readOnly}
                change={(nulls) => change({ nulls })}
              />
            </>
          )}
        </Rows>
      );
      break;
    case "core.split":
      fields = (
        <>
          {text("path", "arrayField")}
          {choice(
            "include",
            "includeFields",
            ["all", "none", "selected"],
            (include) =>
              include === "selected" ? { fields: p.fields ?? [] } : {},
          )}
          {p.include === "selected" && (
            <Rows
              label={t("field")}
              rows={(p.fields ?? []) as string[]}
              readOnly={readOnly}
              create={() => ""}
              change={(fields) => patch({ fields })}
            >
              {(row, index) => (
                <Input
                  aria-label={t("fieldPath")}
                  value={row}
                  readOnly={readOnly}
                  onChange={(event) =>
                    patch({
                      fields: (p.fields as string[]).map((value, at) =>
                        at === index ? event.target.value : value,
                      ),
                    })
                  }
                />
              )}
            </Rows>
          )}
        </>
      );
      break;
    case "data.rename":
      fields = (
        <>
          {pairs("mappings", "mapping")}
          {choice("on_collision", "collision", ["error", "overwrite"])}
        </>
      );
      break;
    case "data.text_split":
      fields = (
        <>
          <Value
            label={t("sourceText")}
            value={p.value}
            readOnly={readOnly}
            change={(value) => patch({ value })}
            invalid={(value) => invalid("text", value)}
          />
          {choice("mode", "splitMode", ["delimiter", "length"], (mode) =>
            mode === "delimiter"
              ? { delimiter: p.delimiter ?? "," }
              : { length: p.length ?? 100, overlap: p.overlap ?? 0 },
          )}
          {p.mode === "delimiter" ? (
            text("delimiter", "delimiter")
          ) : (
            <>
              {number("length", "chunkLength", 1)}
              {number("overlap", "overlap")}
            </>
          )}
        </>
      );
      break;
    case "core.stop":
      fields = text("message", "stopMessage");
      break;
    case "core.no_op":
      fields = (
        <p className="text-sm leading-6 text-muted-foreground">
          {t("noParameters")}
        </p>
      );
      break;
    default:
      fields = integrationManifest(node.type) ? <WorkflowIntegrationParameters nodeId={node.id} spec={integrationManifest(node.type)!} value={p} readOnly={readOnly} change={update} /> : (
        <p className="text-sm text-muted-foreground">{t("advancedHelp")}</p>
      );
  }
  return (
    <FieldHelpContext.Provider value={workflowFieldHelp}><div className="space-y-5">
      {fields}
      <details className="border-t pt-4">
        <summary className="cursor-pointer text-sm text-muted-foreground">
          {t("advanced")}
        </summary>
        <p className="my-3 text-xs leading-5 text-muted-foreground">
          {t("advancedHelp")}
        </p>
        <AdvancedJson
          value={p}
          readOnly={readOnly}
          validate={(value) =>
            matchesShape(value, manifest(node.type)!.params_schema as Shape)
          }
          change={(value) => {
            if (value && !Array.isArray(value) && typeof value === "object") {
              update(value as Params);
              invalid("advanced-shape", false);
            } else invalid("advanced-shape", true);
          }}
          invalid={(value) => invalid("advanced-json", value)}
        />
      </details>
    </div></FieldHelpContext.Provider>
  );
}
