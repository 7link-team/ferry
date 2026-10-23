/**
 * The one form implementation, in three modes driven by the route:
 *
 *   new   create, every field editable
 *   edit  update, `immutable` fields locked, `version` sent for the
 *         optimistic-concurrency check
 *   view  the detail page — the same configuration rendered read-only
 *
 * Read-only renders values as text rather than disabled inputs: a disabled
 * input is low-contrast, awkward to read and awkward to copy out of. A
 * `secret` field never shows a stored value in any mode, only whether one is
 * set (§8 of the design doc).
 */
import { useEffect, useState } from "react";
import { Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { cn } from "cn";
import { errorText } from "../api";
import { createResource, getResource, patchResource } from "../resource";
import { href, navigate, useRoute } from "../router";
import type { FormConfig, FormField, Option, Options } from "./dsl";

function useOptions(source: Options | undefined) {
  const [options, setOptions] = useState<Option[]>(Array.isArray(source) ? source : []);
  useEffect(() => {
    if (typeof source !== "function") {
      setOptions(Array.isArray(source) ? source : []);
      return;
    }
    let live = true;
    void source().then((loaded) => live && setOptions(loaded));
    return () => {
      live = false;
    };
  }, [source]);
  return options;
}

export function ResourceForm<T extends Record<string, unknown>>({
  config,
  workspace,
  permissions,
}: {
  config: FormConfig<T>;
  workspace: string;
  permissions: Set<string>;
}) {
  const route = useRoute();
  const creating = route.mode === "new";
  const readOnly = route.mode === "view";

  const [draft, setDraft] = useState<Partial<T>>(config.blank);
  const [loading, setLoading] = useState(!creating);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    if (creating || !route.id) return;
    let live = true;
    setLoading(true);
    getResource<T>(workspace, config.resource, route.id)
      .then((row) => live && (setDraft(row), setError("")))
      .catch((cause: unknown) => live && setError(errorText(cause)))
      .finally(() => live && setLoading(false));
    return () => {
      live = false;
    };
  }, [creating, route.id, workspace, config.resource]);

  const backToList = href({ ...route, mode: "list" }, {});
  const canEdit = permissions.has(`${config.resource}.${creating ? "create" : "update"}`);

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    setSaving(true);
    setError("");
    try {
      if (creating) {
        await createResource(workspace, config.resource, draft);
      } else {
        // `version` rides along so the server can refuse a stale write.
        await patchResource(workspace, config.resource, route.id!, draft);
      }
      navigate({ ...route, mode: "list" }, {});
    } catch (cause) {
      setError(errorText(cause));
    } finally {
      setSaving(false);
    }
  };

  if (loading)
    return (
      <p className="text-muted-foreground">
        <Loader2 className="mr-2 inline size-4 animate-spin" />
        载入中…
      </p>
    );

  return (
    <form className="max-w-3xl space-y-6 pb-8" onSubmit={submit}>
      {error && (
        <p className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {error}
        </p>
      )}

      {config.sections.map((section, index) => {
        const fields = section.fields.filter((field) => !field.when || field.when(draft));
        if (!fields.length) return null;
        return (
          <fieldset key={section.title ?? index} className="space-y-4">
            {section.title && (
              <legend className="mb-2 text-sm font-medium text-muted-foreground">
                {section.title}
              </legend>
            )}
            <div className="grid gap-4 sm:grid-cols-2">
              {fields.map((field) => (
                <Field
                  key={field.key}
                  field={field}
                  value={draft[field.key]}
                  readOnly={readOnly || (!creating && !!field.immutable)}
                  asText={readOnly}
                  onChange={(value) => setDraft((d) => ({ ...d, [field.key]: value }) as Partial<T>)}
                />
              ))}
            </div>
          </fieldset>
        );
      })}

      <div className="flex gap-2">
        {readOnly ? (
          <>
            <Button asChild variant="outline">
              <a href={backToList}>返回列表</a>
            </Button>
            {permissions.has(`${config.resource}.update`) && route.id && (
              <Button asChild>
                <a href={href({ ...route, mode: "edit" }, {})}>编辑</a>
              </Button>
            )}
          </>
        ) : (
          <>
            <Button type="submit" disabled={saving || !canEdit}>
              {saving && <Loader2 className="animate-spin" />}
              {creating ? "创建" : "保存"}
            </Button>
            <Button type="button" variant="outline" asChild>
              <a href={backToList}>取消</a>
            </Button>
          </>
        )}
      </div>
    </form>
  );
}

function Field<T>({
  field,
  value,
  readOnly,
  asText,
  onChange,
}: {
  field: FormField<T>;
  value: unknown;
  readOnly: boolean;
  asText: boolean;
  onChange: (value: unknown) => void;
}) {
  const options = useOptions(field.options);
  const text = String(value ?? "");

  const label = (
    <span className="text-sm font-medium">
      {field.label}
      {field.required && !asText && <span className="ml-0.5 text-destructive">*</span>}
    </span>
  );

  // A stored secret is never sent to the browser; the server reports only
  // whether one exists, so there is nothing to render but that fact.
  if (field.type === "secret" && asText)
    return (
      <Wrapper field={field}>
        {label}
        <p className="text-sm text-muted-foreground">{value ? "已设置" : "未设置"}</p>
      </Wrapper>
    );

  if (asText)
    return (
      <Wrapper field={field}>
        {label}
        <p className="min-h-5 text-sm break-words">
          {field.type === "switch" ? (
            value ? "是" : "否"
          ) : field.type === "select" ? (
            (options.find((option) => option.value === text)?.label ?? (text || "—"))
          ) : (
            text || "—"
          )}
        </p>
      </Wrapper>
    );

  return (
    <Wrapper field={field}>
      <label className="flex flex-col gap-1.5">
        {label}
        {field.type === "select" ? (
          <Select value={text} onValueChange={onChange} disabled={readOnly}>
            <SelectTrigger>
              <SelectValue placeholder={field.placeholder ?? "请选择"} />
            </SelectTrigger>
            <SelectContent>
              {options.map((option) => (
                <SelectItem key={option.value} value={option.value}>
                  {option.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        ) : field.type === "textarea" ? (
          <textarea
            className="min-h-20 rounded-md border border-input bg-transparent px-3 py-2 text-sm shadow-xs outline-none focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:opacity-50"
            value={text}
            placeholder={field.placeholder}
            readOnly={readOnly}
            onChange={(event) => onChange(event.target.value)}
          />
        ) : field.type === "switch" ? (
          <Checkbox
            checked={!!value}
            disabled={readOnly}
            onCheckedChange={(checked) => onChange(!!checked)}
          />
        ) : (
          <Input
            type={
              field.type === "number" ? "number" : field.type === "text" ? "text" : "password"
            }
            value={text}
            placeholder={field.placeholder}
            readOnly={readOnly}
            required={field.required}
            autoComplete={field.type === "password" ? "new-password" : undefined}
            onChange={(event) =>
              onChange(field.type === "number" ? Number(event.target.value) : event.target.value)
            }
          />
        )}
      </label>
      {field.help && <p className="mt-1 text-xs text-muted-foreground">{field.help}</p>}
    </Wrapper>
  );
}

const Wrapper = <T,>({
  field,
  children,
}: {
  field: FormField<T>;
  children: React.ReactNode;
}) => (
  <div className={cn("flex flex-col gap-1.5", field.span === 2 && "sm:col-span-2")}>{children}</div>
);
