import { useEffect, useRef, useState } from "react";
import { AutoField } from "../../../shared/forms/auto-form";
import {
  hostRepositoryText,
  useCommonTranslation,
} from "../../../shared/i18n/i18n";
import { area } from "./fields";
import { object, type Row } from "./model";
export function JsonField({
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
  useCommonTranslation();
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
    <AutoField
      label={
        <>
          {label}
          {hostRepositoryText("manifest.099")}
        </>
      }
      className="flex min-w-0 flex-col gap-1.5 text-sm sm:col-span-2"
    >
      <textarea
        className={area}
        aria-label={hostRepositoryText("manifest.098", { value1: label })}
        value={text}
        readOnly={readOnly}
        aria-invalid={!!error}
        ref={(element) => element?.setCustomValidity(error)}
        onChange={(e) => {
          setText(e.target.value);
          try {
            const result: unknown = JSON.parse(e.target.value);
            if (!object(result))
              throw new Error(hostRepositoryText("manifest.100"));
            setError("");
            sent.current = JSON.stringify(result, null, 2);
            onChange(result);
          } catch {
            setError(hostRepositoryText("manifest.101"));
          }
        }}
      />
      {error && (
        <span role="alert" className="text-destructive">
          {error}
        </span>
      )}
    </AutoField>
  );
}
