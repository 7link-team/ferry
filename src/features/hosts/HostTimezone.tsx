import { commonText } from "../../shared/i18n/i18n";
import { TimezoneField } from "../../shared/forms/TimezoneField";
/** The host fields this control reads (kept local to avoid an import cycle). */
type Host = { id?: string; agent_version?: string; os?: string; timezone?: string; detected_timezone?: string; timezone_supported?: boolean; connection?: string };

const t = (key: string, values?: Record<string, unknown>) =>
  commonText(`resource-domain:timezone.${key}`, values);

/** The Runner time zone: automatic (host's own) by default, applied without restart. */
export function HostTimezone({
  host,
  readOnly,
  onChange,
  onValidityChange,
}: {
  host: Partial<Host>;
  readOnly: boolean;
  onChange: (zone: string) => void;
  onValidityChange?: ((blocked: boolean) => void) | undefined;
}) {
  const detected = host.detected_timezone ?? "";
  // Only an enrolled Runner reports whether it can apply a zone.
  const enrolled = !!host.id && !!host.agent_version;
  const windows = (host.os ?? "").toLowerCase().includes("windows");
  return (
    <TimezoneField
      label={t("label")}
      value={host.timezone ?? ""}
      emptyLabel={detected ? t("auto", { zone: detected }) : t("autoUnknown")}
      help={t("hostHelp")}
      readOnly={readOnly}
      onChange={onChange}
      onValidityChange={onValidityChange}
      notes={
        (enrolled && host.timezone_supported === false) || windows ? (
          <div className="space-y-1 text-sm text-muted-foreground">
            {enrolled && host.timezone_supported === false && (
              <p role="status" className="text-amber-700 dark:text-amber-400">
                {t("upgrade")}
              </p>
            )}
            {windows && <p>{t("windows")}</p>}
          </div>
        ) : null
      }
    />
  );
}
