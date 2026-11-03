import { commonText } from "../../shared/i18n/i18n";
export const text = (key: string) =>
  commonText(`reports-reviews-domain:${key}`);
export const key = (name: string) => `reports-reviews-domain:${name}`;
export const statusText = (value: unknown, kind?: "test" | "ai") => {
  const name = String(value ?? "disabled");
  return text(
    (
      {
        disabled: "not_enabled",
        pending: "queued",
        completed: "complete",
        completed_clear: "clean",
        completed_findings: "issues",
        unavailable: "unavailable",
        success: "passed",
        ready: "complete",
        collecting: "running",
        fetching: "running",
        analyzing: "running",
        error: "failed",
      } as Record<string, string>
    )[name] ?? (name === "failed" && kind === "test" ? "testsFailed" : name),
  );
};
