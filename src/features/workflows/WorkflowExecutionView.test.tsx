import { afterEach, expect, it } from "vitest";
import { i18n, workflowText } from "../../shared/i18n/i18n";
import { formatExecutionLogs } from "./WorkflowExecutionView";

afterEach(async () => { await i18n.changeLanguage("zh-CN"); });

it("translates lifecycle logs and preserves native, malformed and future records", async () => {
  const prefix = "[2026-11-24T02:00:00Z] [event] ";
  const raw = "[2026-11-24T02:00:00Z] [stdout] script output";
  const unknown = prefix + '{"event":"future_event","data":{"value":1}}';
  const malformed = prefix + "not JSON";
  const record = prefix + '{"event":"agent_started","data":{"input_items":152,"runtime":"codex"}}';
  for (const language of ["en-US", "zh-CN", "ms-MY"]) {
    await i18n.changeLanguage(language);
    const result = formatExecutionLogs([raw, record, unknown, malformed, ""].join("\n"));
    expect(result).toContain(workflowText("execution.log.agentStarted"));
    expect(result).toContain('"input_items":152');
    expect(result).toContain(raw);
    expect(result).toContain(unknown);
    expect(result).toContain(malformed);
    expect(result.endsWith("\n")).toBe(true);
  }
});
