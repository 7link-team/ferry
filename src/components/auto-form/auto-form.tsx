/** Adapted from 7link-mall-web AutoForm: configuration, conditional fields, renderer extension and shared layout. */
import {
  createContext,
  useContext,
  useId,
  useRef,
  useState,
  type ComponentProps,
  type ReactNode,
} from "react";
import { cn } from "cn";
import { Input } from "../ui/input";
import { Checkbox } from "../ui/checkbox";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "../ui/select";
import {
  evaluate,
  isVisible,
  type AutoFormProps,
  type FieldConfig,
  type FormValues,
} from "./types";

const Errors = createContext<Record<string, string>>({});
const controlClass =
  "w-full min-w-0 rounded-md border border-input bg-background px-3 py-2 text-sm shadow-xs outline-none transition-colors focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:cursor-not-allowed disabled:opacity-50 aria-invalid:border-destructive";
const fieldControls =
  "input:not([type=hidden]):not([aria-hidden=true]),select:not([aria-hidden=true]),textarea:not([aria-hidden=true]),[role=combobox]:not([aria-hidden=true]),[role=checkbox]:not([aria-hidden=true])";
function focusField(form: HTMLFormElement | null, key: string) {
  const frame = [
    ...(form?.querySelectorAll<HTMLElement>("[data-auto-field]") ?? []),
  ].find((element) => element.dataset.autoField === key);
  const control =
    frame?.querySelector<HTMLElement>(
      fieldControls,
    ) ??
    ([...(form?.elements ?? [])].find(
      (element) => (element as HTMLInputElement).name === key,
    ) as HTMLElement | undefined);
  control?.focus();
}

/** Common field frame, also used by custom editors and compound controls. Never nests labels. */
export function AutoField({
  label,
  name,
  required,
  help,
  error: ownError,
  className,
  children,
  hideLabel,
  ...props
}: {
  label?: ReactNode;
  name?: string;
  required?: boolean | undefined;
  help?: ReactNode;
  error?: string | undefined;
  children: ReactNode;
  className?: string | undefined;
  hideLabel?: boolean;
} & Omit<ComponentProps<"div">, "children">) {
  const id = useId();
  const errors = useContext(Errors);
  const error = ownError ?? errors[name ?? id];
  return (
    <div
      {...props}
      className={cn(
        "auto-field resource-field flex min-w-0 flex-col gap-1.5",
        className,
      )}
      data-auto-field={name ?? id}
      ref={(element) => {
        // Associate arbitrary custom controls with the same label/help/error, including Radix triggers.
        if (!element) return;
        const controls = element.querySelectorAll<HTMLElement>(
          fieldControls,
        );
        const first = [...controls].find(
          (control) => control.closest("[data-auto-field]") === element,
        );
        const caption = element.querySelector<HTMLLabelElement>(":scope > label[data-auto-label]");
        if (first && caption) {
          if (!first.id) first.id = `${id}-control`;
          caption.htmlFor = first.id;
        }
        for (const control of controls) {
          if (control.closest("[data-auto-field]") !== element) continue;
          if ("name" in control && !(control as HTMLInputElement).name)
            (control as HTMLInputElement).name = name ?? id;
          if (label && !control.hasAttribute("aria-label"))
            control.setAttribute("aria-labelledby", `${id}-label`);
          const existing = (control.getAttribute("aria-describedby") ?? "")
            .split(" ")
            .filter(
              (value) =>
                value && value !== `${id}-help` && value !== `${id}-error`,
            );
          const described = [
            ...existing,
            ...(help ? [`${id}-help`] : []),
            ...(error ? [`${id}-error`] : []),
          ];
          if (described.length)
            control.setAttribute("aria-describedby", described.join(" "));
          else control.removeAttribute("aria-describedby");
          if (error) control.setAttribute("aria-invalid", "true");
          else if (control.dataset.autoInvalid === "true")
            control.removeAttribute("aria-invalid");
          control.dataset.autoInvalid = String(!!error);
        }
      }}
    >
      {label != null && (
        <label
          data-auto-label
          className={cn(
            "text-sm font-medium leading-5",
            hideLabel && "sr-only",
          )}
        >
          <span id={`${id}-label`}>{label}</span>
          {required && (
            <span className="ml-1 text-destructive" aria-hidden="true">
              *
            </span>
          )}
        </label>
      )}
      {children}
      {help && (
        <p
          id={`${id}-help`}
          className="text-xs leading-relaxed text-muted-foreground"
        >
          {help}
        </p>
      )}
      {error && (
        <p
          id={`${id}-error`}
          role="alert"
          className="text-xs leading-relaxed text-destructive"
        >
          {error}
        </p>
      )}
    </div>
  );
}

export function AutoFields({
  fields,
  values,
  onChange,
  readonly = false,
  columns = 2,
  className,
}: {
  fields: AutoFormProps["fields"];
  values: FormValues;
  onChange: (values: FormValues) => void;
  readonly?: boolean;
  columns?: 1 | 2;
  className?: string | undefined;
}) {
  const errors = useContext(Errors);
  return (
    <div
      className={cn(
        "grid min-w-0 gap-x-6 gap-y-5",
        columns === 2 && "sm:grid-cols-2",
        className,
      )}
      data-auto-fields
    >
      {Object.entries(fields ?? {})
        .filter(([, field]) => isVisible(field, values))
        .map(([name, field]) => {
          const value = values[name] ?? field.value;
          const required = evaluate(field.required, values) === true;
          const disabled =
            readonly || evaluate(field.disabled, values) === true;
          const full =
            field.col === 2 ||
            (field.col === undefined &&
              ["textarea", "code", "custom", "title", "multi-select"].includes(
                field.type,
              ));
          if (field.type === "title")
            return (
              <h3
                key={name}
                className="col-span-full border-b pb-3 text-sm font-semibold"
              >
                {field.name}
              </h3>
            );
          if (field.framed === false && field.render)
            return (
              <div
                key={name}
                className={cn("min-w-0", full && "sm:col-span-2")}
              >
                {field.render({
                  name,
                  value,
                  values,
                  required,
                  disabled,
                  readonly,
                  error: errors[name],
                  onChange: (value) => onChange({ ...values, [name]: value }),
                })}
              </div>
            );
          return (
            <AutoField
              key={name}
              name={name}
              label={field.name}
              required={required && !readonly}
              help={field.help}
              error={errors[name]}
              className={cn(full && "sm:col-span-2")}
            >
              {field.render ? (
                field.render({
                  name,
                  value,
                  values,
                  required,
                  disabled,
                  readonly,
                  error: errors[name],
                  onChange: (value) => onChange({ ...values, [name]: value }),
                })
              ) : (
                <FieldControl
                  name={name}
                  field={field}
                  value={value}
                  required={required}
                  disabled={disabled}
                  readonly={readonly}
                  onChange={(value) => onChange({ ...values, [name]: value })}
                />
              )}
            </AutoField>
          );
        })}
    </div>
  );
}

export function FieldControl({
  name,
  field,
  value,
  required,
  disabled,
  readonly,
  onChange,
}: {
  name: string;
  field: FieldConfig;
  value: unknown;
  required?: boolean | undefined;
  disabled?: boolean;
  readonly?: boolean;
  onChange: (value: unknown) => void;
}) {
  const config = field.config ?? {};
  const text = String(value ?? "");
  if (readonly)
    return (
      <p
        className={cn(
          "min-h-5 break-words text-sm",
          ["code", "textarea"].includes(field.type) && "whitespace-pre-wrap",
          field.type === "code" && "font-mono",
        )}
      >
        {field.type === "password"
          ? value
            ? "已设置"
            : "未设置"
          : field.type === "switch"
            ? value
              ? "是"
              : "否"
            : field.type === "select"
              ? (config.options?.find((option) => option.value === text)
                  ?.label ??
                (text || "—"))
              : text || "—"}
      </p>
    );
  if (field.type === "switch")
    return (
      <Checkbox
        name={name}
        checked={!!value}
        disabled={disabled}
        onCheckedChange={(checked) => onChange(!!checked)}
      />
    );
  if (field.type === "select")
    return (
      <Select
        name={name}
        value={text}
        required={required}
        disabled={disabled}
        onValueChange={(value) =>
          onChange(value === "__auto_empty__" ? "" : value)
        }
      >
        <SelectTrigger className="w-full">
          <SelectValue
            placeholder={
              config.placeholder ??
              config.options?.find((option) => option.value === "")?.label ??
              "请选择"
            }
          />
        </SelectTrigger>
        <SelectContent>
          {text && !config.options?.some((option) => option.value === text) && (
            <SelectItem value={text}>{text}</SelectItem>
          )}
          {config.options?.map((option) => (
            <SelectItem
              key={option.value}
              value={option.value || "__auto_empty__"}
              disabled={option.disabled}
            >
              {option.label}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    );
  if (field.type === "multi-select")
    return (
      <select
        name={name}
        className={controlClass}
        value={Array.isArray(value) ? value.map(String) : []}
        required={required}
        disabled={disabled}
        multiple
        onChange={(event) =>
          onChange(
            [...event.target.selectedOptions].map((option) => option.value),
          )
        }
      >
        {config.options?.map((option) => (
          <option
            key={option.value}
            value={option.value}
            disabled={option.disabled}
          >
            {option.label}
          </option>
        ))}
      </select>
    );
  if (["textarea", "code"].includes(field.type))
    return (
      <textarea
        name={name}
        className={cn(
          controlClass,
          "min-h-24",
          field.type === "code" && "font-mono",
        )}
        value={text}
        required={required}
        disabled={disabled}
        placeholder={config.placeholder}
        minLength={config.minLength}
        maxLength={config.maxLength}
        onChange={(event) => onChange(event.target.value)}
      />
    );
  return (
    <Input
      name={name}
      type={
        field.type === "custom" || field.type === "title" ? "text" : field.type
      }
      value={text}
      required={required}
      disabled={disabled}
      placeholder={config.placeholder}
      min={config.min}
      max={config.max}
      minLength={config.minLength}
      maxLength={config.maxLength}
      pattern={config.pattern}
      autoComplete={
        config.autoComplete ??
        (field.type === "password" ? "new-password" : undefined)
      }
      onChange={(event) =>
        onChange(
          field.type === "number" && event.target.value !== ""
            ? Number(event.target.value)
            : event.target.value,
        )
      }
    />
  );
}

/** Accepts configuration or composed children, so rich editors share validation and form behavior. */
export function AutoForm({
  fields,
  values: controlled,
  initialData,
  onValuesChange,
  onValuesSubmit,
  onError,
  mode = "add",
  readonly,
  columns = 2,
  children,
  onSubmit,
  className,
  ...props
}: AutoFormProps) {
  const [local, setLocal] = useState<FormValues>(() =>
    Object.fromEntries(
      Object.entries(fields ?? {})
        .filter(([, field]) => field.type !== "title")
        .map(([key, field]) => [
          key,
          (mode !== "add" && !(mode === "copy" && field.copy === false)
            ? initialData?.[key]
            : undefined) ??
            field.value ??
            "",
        ]),
    ),
  );
  const values = controlled ?? local;
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [submitError, setSubmitError] = useState("");
  const summary = useRef<HTMLDivElement>(null);
  const submitting = useRef(false);
  const change = (next: FormValues) => {
    setLocal(next);
    onValuesChange?.(next);
    setErrors({});
  };
  return (
    <Errors.Provider value={errors}>
      <form
        {...props}
        className={className}
        data-auto-form
        noValidate
        onInput={(event) => {
          const key = (event.target as Element).closest<HTMLElement>(
            "[data-auto-field]",
          )?.dataset.autoField;
          if (key)
            setErrors((previous) => {
              const next = { ...previous };
              delete next[key];
              return next;
            });
          props.onInput?.(event);
        }}
        onSubmit={async (event) => {
          event.preventDefault();
          event.stopPropagation();
          if (submitting.current || readonly || mode === "detail") return;
          const form = event.currentTarget;
          submitting.current = true;
          setSubmitError("");
          try {
            const nextErrors: Record<string, string> = {};
            const nativeInvalid = [...form.elements].filter(
              (control): control is HTMLInputElement =>
                "validity" in control &&
                !(control as HTMLInputElement).validity.valid,
            );
            for (const control of nativeInvalid) {
              const frame = control.closest<HTMLElement>("[data-auto-field]");
              const key =
                frame?.dataset.autoField ||
                control.name ||
                control.id ||
                "字段";
              const label =
                frame
                  ?.querySelector("[data-auto-label]")
                  ?.textContent?.replace(/\*$/, "") ||
                control.getAttribute("aria-label") ||
                control.name ||
                "此项";
              nextErrors[key] = `${label}：${control.validationMessage}`;
            }
            for (const [key, field] of Object.entries(fields ?? {})) {
              if (!isVisible(field, values) || field.type === "title") continue;
              const value = values[key];
              if (
                evaluate(field.required, values) &&
                (value == null ||
                  value === "" ||
                  (Array.isArray(value) && !value.length))
              )
                nextErrors[key] =
                  `请填写${typeof field.name === "string" ? field.name : "此项"}。`;
              for (const rule of field.config?.rules ?? []) {
                const message = await rule(value, values);
                if (message) {
                  nextErrors[key] = message;
                  break;
                }
              }
            }
            setErrors(nextErrors);
            if (Object.keys(nextErrors).length) {
              requestAnimationFrame(() => summary.current?.focus());
              return;
            }
            if (onSubmit) {
              event.currentTarget = form;
              await onSubmit(event);
              return;
            }
            if (!onValuesSubmit) return;
            await onValuesSubmit(
              Object.fromEntries(
                Object.entries(values).filter(
                  ([key]) =>
                    !fields?.[key] ||
                    (fields[key].type !== "title" &&
                      fields[key].submit !== false &&
                      isVisible(fields[key], values)),
                ),
              ),
            );
          } catch (error) {
            if (onError) onError(error);
            else
              setSubmitError(
                error instanceof Error ? error.message : "操作失败，请重试。",
              );
          } finally {
            submitting.current = false;
          }
        }}
      >
        {submitError && (
          <p
            role="alert"
            className="mb-4 rounded-md border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive"
          >
            {submitError}
          </p>
        )}
        {!!Object.keys(errors).length && (
          <div
            ref={summary}
            tabIndex={-1}
            role="alert"
            className="mb-4 rounded-md border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive"
          >
            <p className="font-medium">请检查以下内容</p>
            <ul className="mt-1 space-y-1">
              {Object.entries(errors).map(([key, message]) => (
                <li key={key}>
                  <button
                    type="button"
                    className="text-left underline underline-offset-2"
                    onClick={(event) =>
                      focusField(event.currentTarget.closest("form"), key)
                    }
                  >
                    {message}
                  </button>
                </li>
              ))}
            </ul>
          </div>
        )}
        {fields && (
          <AutoFields
            fields={fields}
            values={values}
            onChange={change}
            readonly={readonly || mode === "detail"}
            columns={columns}
          />
        )}
        {children}
      </form>
    </Errors.Provider>
  );
}
