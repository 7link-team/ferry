/** Ported from 7link-mall-web/ui/components/auto-form; Ferry renderers use existing Radix/Monaco controls. */
import type { ComponentProps, ReactNode } from "react";

export type FormValues = Record<string, unknown>;
export type Evaluable<T> = T | ((values: FormValues) => T);
export type FormMode = "add" | "edit" | "copy" | "detail";
export interface SelectOption {
  value: string;
  label: string;
  disabled?: boolean;
}
export interface FieldConfig {
  name: ReactNode;
  type:
    | "text"
    | "email"
    | "password"
    | "number"
    | "textarea"
    | "code"
    | "select"
    | "switch"
    | "multi-select"
    | "title"
    | "custom";
  value?: unknown;
  col?: 1 | 2;
  show?: Evaluable<boolean>;
  required?: Evaluable<boolean>;
  disabled?: Evaluable<boolean>;
  help?: ReactNode;
  submit?: boolean;
  copy?: boolean;
  config?: {
    placeholder?: string;
    options?: SelectOption[];
    min?: number;
    max?: number;
    minLength?: number;
    maxLength?: number;
    pattern?: string;
    autoComplete?: string;
    rules?: ((
      value: unknown,
      values: FormValues,
    ) => string | undefined | Promise<string | undefined>)[];
  };
  render?: (props: FieldRendererProps) => ReactNode;
  /** Composite editors already provide their own shared AutoField frame. */
  framed?: boolean;
}
export type FormConfig = Record<string, FieldConfig>;
export interface FieldRendererProps {
  name: string;
  value: unknown;
  values: FormValues;
  onChange: (value: unknown) => void;
  readonly: boolean;
  disabled: boolean;
  required: boolean;
  error?: string | undefined;
}
export interface AutoFormProps extends Omit<
  ComponentProps<"form">,
  "onChange"
> {
  fields?: FormConfig;
  initialData?: FormValues;
  values?: FormValues;
  onValuesChange?: (values: FormValues) => void;
  onValuesSubmit?: (values: FormValues) => void | Promise<void>;
  onError?: (error: unknown) => void;
  mode?: FormMode;
  readonly?: boolean;
  columns?: 1 | 2;
}
export function evaluate<T>(
  value: Evaluable<T> | undefined,
  values: FormValues,
): T | undefined {
  return typeof value === "function"
    ? (value as (values: FormValues) => T)(values)
    : value;
}
export function isVisible(field: FieldConfig, values: FormValues) {
  return evaluate(field.show, values) !== false;
}
