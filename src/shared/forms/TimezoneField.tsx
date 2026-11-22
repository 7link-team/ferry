import { useEffect, useMemo, type ReactNode } from "react";
import { commonText } from "../i18n/i18n";
import { AutoField } from "./auto-form";
import { ComboInput } from "./ComboInput";

/** IANA zones the browser knows; empty where `Intl.supportedValuesOf` is missing. */
export function timeZoneNames(): string[] {
  try {
    return (Intl as unknown as { supportedValuesOf?: (key: string) => string[] }).supportedValuesOf?.("timeZone") ?? [];
  } catch {
    return [];
  }
}

/** Empty (not set) or a zone the browser can format in; the server checks again. */
export function validTimeZone(zone: string): boolean {
  if (!zone) return true;
  if (!/^[A-Za-z0-9/_+-]{1,64}$/.test(zone)) return false;
  try {
    new Intl.DateTimeFormat("en-US", { timeZone: zone });
    return true;
  } catch {
    return false;
  }
}

/** An optional IANA time zone: searchable list, free text checked, empty = `emptyLabel`. */
export function TimezoneField({
  label,
  required = false,
  value,
  emptyLabel,
  help,
  notes,
  readOnly,
  onChange,
  onValidityChange,
}: {
  label: string;
  required?: boolean;
  value: string;
  emptyLabel: string;
  help?: ReactNode;
  notes?: ReactNode;
  readOnly: boolean;
  onChange: (zone: string) => void;
  onValidityChange?: ((blocked: boolean) => void) | undefined;
}) {
  const zones = useMemo(timeZoneNames, []);
  const invalid = (required && !value.trim()) || !validTimeZone(value.trim());
  useEffect(() => {
    onValidityChange?.(!readOnly && invalid);
  }, [readOnly, invalid, onValidityChange]);
  return (
    <AutoField
      label={label}
      required={required}
      help={help}
      error={invalid ? commonText("resource-domain:timezone.invalid") : undefined}
    >
      {readOnly ? (
        <p className="break-words text-sm">{value || emptyLabel}</p>
      ) : (
        <div className="[&_button]:min-h-11"><ComboInput
          className="min-h-11"
          aria-label={label}
          required={required}
          options={zones}
          value={value}
          placeholder={emptyLabel}
          onValueChange={(zone) => onChange(zone.trim())}
        /></div>
      )}
      {notes}
    </AutoField>
  );
}
