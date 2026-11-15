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
  value,
  emptyLabel,
  help,
  notes,
  readOnly,
  onChange,
  onValidityChange,
}: {
  label: string;
  value: string;
  emptyLabel: string;
  help?: ReactNode;
  notes?: ReactNode;
  readOnly: boolean;
  onChange: (zone: string) => void;
  onValidityChange?: ((blocked: boolean) => void) | undefined;
}) {
  const zones = useMemo(timeZoneNames, []);
  const invalid = !validTimeZone(value.trim());
  useEffect(() => {
    onValidityChange?.(!readOnly && invalid);
  }, [readOnly, invalid, onValidityChange]);
  return (
    <AutoField
      label={label}
      help={help}
      error={invalid ? commonText("resource-domain:timezone.invalid") : undefined}
    >
      {readOnly ? (
        <p className="break-words text-sm">{value || emptyLabel}</p>
      ) : (
        <ComboInput
          aria-label={label}
          options={zones}
          value={value}
          placeholder={emptyLabel}
          onValueChange={(zone) => onChange(zone.trim())}
        />
      )}
      {notes}
    </AutoField>
  );
}
