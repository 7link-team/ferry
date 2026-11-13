import { expect, it } from "vitest";
import { alertsForms } from "./resource-config";

it("deployment_target_offers_failover_attention_metric_without_numeric_threshold", () => {
  const form = alertsForms({ workspace: "ops", navigate: () => {} })["alert-rules"]!;
  const fields = form.sections.flatMap(section => section.fields);
  const metricsFor = (kind: string) => fields.filter(field => field.key === "metric" && field.when?.({ target_kind: kind })).flatMap(field => Array.isArray(field.options) ? field.options.map(option => option.value) : []);
  expect(metricsFor("deployment")).toEqual(["failover-attention", "execution-failed"]);
  for (const kind of ["host", "pipeline", "cron-job", "health-check", "application-health"]) expect(metricsFor(kind)).not.toContain("failover-attention");
  expect(fields.find(field => field.key === "threshold")!.when?.({ metric: "failover-attention" })).toBe(false);
});
