import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { api } from "../../shared/api/api";
import { i18n, workflowText as t } from "../../shared/i18n/i18n";
import { WorkflowAssetContext } from "./WorkflowAssets";
import { WorkflowNodeParameters } from "./WorkflowNodeParameters";
import { formatsProblem, parseFormats, schemaProblem } from "./WorkflowAgentParameters";
import { emptyGraph, newNode, type GraphNode } from "./graph";
import { WorkflowExecutionTarget } from "./WorkflowExecutionTarget";

vi.mock("../../shared/api/api", () => ({
  api: vi.fn(),
  errorText: (cause: unknown) => (cause instanceof Error ? cause.message : String(cause)),
}));
const calls = vi.mocked(api);
const agents = [
  { id: "11", name: "Analyst", state: "enabled", runtime: "codex", host_id: "5", host_name: "Build runner" },
  { id: "12", name: "Writer", state: "enabled", runtime: "kimi", host_id: "5", host_name: "Build runner" },
  { id: "13", name: "Retired", state: "disabled", runtime: "claude", host_id: "5", host_name: "Build runner" },
  { id: "14", name: "Floating", state: "enabled", runtime: "claude", host_id: "0" },
  { id: "15", name: "API analyst", state: "enabled", runtime: "api", host_id: "0" },
];

let host: HTMLDivElement, root: Root;
let current: Record<string, unknown>;
const invalid = vi.fn();
beforeEach(async () => {
  await i18n.changeLanguage("zh-CN");
  calls.mockReset();
  invalid.mockReset();
  calls.mockImplementation(async (path: string) => {
    const detail = path.match(/\/ai-agents\/(\d+)$/);
    if (detail) return agents.find(agent => agent.id === detail[1]) as never;
    if (path.includes("/ai-agents")) return { items: agents } as never;
    if (path.includes("/nodes")) return { items: [{ id: "9", name: "GPU runner" }], total: 1, page: 1, page_size: 200 } as never;
    throw new Error(`unexpected ${path}`);
  });
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  current = newNode(emptyGraph(), "ai.agent.run", "agent").params;
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); });

function Harness({ readOnly = false, settings }: { readOnly?: boolean; settings?: Record<string, unknown> }) {
  const [value, setValue] = useState(current);
  current = value;
  const node: GraphNode = { id: "agent", type: "ai.agent.run", type_version: 1, name: "agent", position: [0, 0], params: value, ...(settings ? { settings } : {}) };
  return <WorkflowAssetContext.Provider value={{ workspace: "7", workflow: "1" }}>
    <WorkflowNodeParameters node={node} readOnly={readOnly} update={setValue} invalid={invalid} />
  </WorkflowAssetContext.Provider>;
}
const render = async (element: React.ReactElement) => { await act(async () => root.render(element)); await act(async () => {}); };
const field = <T extends HTMLElement>(label: string) => host.querySelector<T>(`[aria-label="${label}"]`)!;
async function change(label: string, value: string) {
  const element = field<HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement>(label);
  expect(element, label).not.toBeNull();
  const prototype = element instanceof HTMLSelectElement ? HTMLSelectElement.prototype : element instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  await act(async () => {
    Object.getOwnPropertyDescriptor(prototype, "value")!.set!.call(element, value);
    element.dispatchEvent(new Event(element instanceof HTMLSelectElement ? "change" : "input", { bubbles: true }));
  });
}
const reported = (key: string) => invalid.mock.calls.filter(([name]) => name === key).at(-1)?.[1];
const location = () => host.querySelector('[data-testid="agent-location"]')?.textContent;

it("explains that API Agents need no host even when a CLI host target was saved", async () => {
  current.agent_id = "15";
  await render(<Harness settings={{ runtime: "host", target_kind: "host", target_ref: "9" }} />);
  expect(location()).toBe(t("agent.locationApi"));
  expect(host.querySelector('[role="alert"]')).toBeNull();
});

it("uses model candidate selection for an untargeted CLI Agent instead of claiming no host", async () => {
  current.agent_id = "14";
  await render(<Harness />);
  expect(location()).toBe(t("agent.locationAutomatic"));
  expect(host.querySelector('[role="alert"]')).toBeNull();
});

it("hides host controls for API models and restores CLI controls after switching", async () => {
  const value = { runtime: "host", target_kind: "host", target_ref: "9" };
  const change = vi.fn();
  await render(<WorkflowExecutionTarget workspace="7" value={value} change={change} readOnly={false} hostOnly inherit agentId="15" />);
  expect(host.querySelector("select")).toBeNull();
  expect(calls.mock.calls.some(([path]) => String(path).includes("/nodes"))).toBe(false);
  await render(<WorkflowExecutionTarget workspace="7" value={value} change={change} readOnly={false} hostOnly inherit agentId="14" />);
  expect(host.querySelector("select")).not.toBeNull();
  expect(change).not.toHaveBeenCalled();
});

it("starts unconfigured, lists usable agents and states the Runner and CLI it runs on", async () => {
  expect(current).toMatchObject({ agent_id: "", prompt: "", output_schema: { type: "object" } });
  await render(<Harness />);
  expect(calls).toHaveBeenCalledWith("/workspaces/7/ai-agents?page_size=100");
  expect(location()).toBe("执行位置：选择智能体后显示");
  const options = [...field<HTMLSelectElement>(t("agent.agent")).options];
  const option = (id: string) => options.find((item) => item.value === id)!;
  expect(option("11").disabled).toBe(false);
  // Kimi cannot run read-only; disabled agents cannot be chosen either.
  expect(option("12").disabled).toBe(true);
  expect(option("12").textContent).toContain(t("agent.readOnlyUnsupported"));
  expect(option("13").disabled).toBe(true);
  await change(t("agent.agent"), "11");
  expect(current.agent_id).toBe("11");
  expect(location()).toBe("执行位置：Runner Build runner 上的 Codex");
  await change(t("agent.prompt"), "=分析 {{ $json.since }}");
  expect(current.prompt).toBe("=分析 {{ $json.since }}");
  expect(field(t("agent.timeout"))).toBeNull();
  expect(field(t("agent.maxToolCalls"))).toBeNull();
  expect(current.timeout_s).toBeUndefined();
  expect(current.max_tool_calls).toBeUndefined();
  // An untargeted CLI uses model candidates, not an obligatory Agent binding.
  await change(t("agent.agent"), "14");
  expect(location()).toBe(t("agent.locationAutomatic"));
  expect(host.querySelector('[role="alert"]')).toBeNull();
  await render(<Harness settings={{ runtime: "host", target_kind: "host", target_ref: "9" }} />);
  await act(async () => {});
  expect(location()).toBe("执行位置：Runner GPU runner 上的 Claude Code");
  expect(host.querySelector('[role="alert"]')).toBeNull();
});

it("validates the output schema like the server before storing it", async () => {
  await render(<Harness />);
  const before = current.output_schema;
  await change(t("agent.outputSchema"), "[1]");
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(t("agent.schemaRoot"));
  expect(reported("agentSchema")).toBe(true);
  expect(current.output_schema).toBe(before);
  await change(t("agent.outputSchema"), "{\"type\":");
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(t("agent.schemaJson"));
  const schema = { type: "object", properties: { verdict: { type: "string", enum: ["ok", "risk"] } }, required: ["verdict"] };
  await change(t("agent.outputSchema"), JSON.stringify(schema));
  expect(current.output_schema).toEqual(schema);
  expect(host.querySelector('[role="alert"]')).toBeNull();
  expect(reported("agentSchema")).toBe(false);
  expect(schemaProblem(JSON.stringify({ type: "object", description: "x".repeat(64 * 1024) }))).toBe("");
  expect(schemaProblem("{\"properties\":{}}")).toBe("agent.schemaRoot");
  expect(schemaProblem("{\"type\":\"object\"}")).toBe("");
  // Legacy prompt byte ceilings no longer block default-unlimited drafts.
  await change(t("agent.prompt"), "数".repeat(12_000));
  expect(host.querySelector('[role="alert"]')).toBeNull();
  expect(reported("agentPrompt")).toBe(false);
});

it("is read-only in view mode and localizes the execution location in six languages", async () => {
  current = { ...current, agent_id: "11", prompt: "Summarize" };
  await render(<Harness readOnly />);
  expect([...host.querySelectorAll("input,textarea")].every((item) => (item as HTMLInputElement).readOnly)).toBe(true);
  expect(field<HTMLSelectElement>(t("agent.agent")).disabled).toBe(true);
  const expected: Record<string, string> = {
    "en-US": "Execution location: Codex on Runner Build runner",
    "zh-CN": "执行位置：Runner Build runner 上的 Codex",
    "zh-TW": "執行位置：Runner Build runner 上的 Codex",
    "ja-JP": "実行場所：Runner Build runner 上の Codex",
    "ko-KR": "실행 위치: Runner Build runner의 Codex",
    "ms-MY": "Lokasi pelaksanaan: Codex pada Runner Build runner",
  };
  for (const [locale, text] of Object.entries(expected)) {
    await act(async () => { await i18n.changeLanguage(locale); });
    await render(<Harness readOnly />);
    expect(location(), locale).toBe(text);
  }
});

it("configures file output with platform quotas: formats, an optional schema and confinable CLIs only", async () => {
  await render(<Harness />);
  await change(t("agent.agent"), "11");
  expect(field<HTMLInputElement>(t("agent.filesFormats"))).toBeNull();
  await act(async () => field<HTMLInputElement>(t("agent.files")).click());
  expect(current.files).toEqual({ enabled: true });
  await change(t("agent.filesFormats"), ".PDF, pptx，xlsx pdf");
  expect(current.files).toMatchObject({ enabled: true, formats: ["pdf", "pptx", "xlsx"] });
  expect(field(t("agent.filesMax"))).toBeNull();
  expect(field(t("agent.filesTotal"))).toBeNull();
  await change(t("agent.filesInstructions"), "用公司配色");
  expect(current.files).toEqual({ enabled: true, formats: ["pdf", "pptx", "xlsx"], instructions: "用公司配色" });
  // A bad extension is reported and not stored.
  await change(t("agent.filesFormats"), "pdf, ../x");
  expect(host.textContent).toContain(t("agent.filesFormatsInvalid"));
  expect(reported("agentFiles")).toBe(true);
  expect(current.files).toMatchObject({ formats: ["pdf", "pptx", "xlsx"] });
  await change(t("agent.filesFormats"), "");
  expect(current.files).toMatchObject({ formats: "any" });
  expect(reported("agentFiles")).toBe(false);
  // With files the schema may be left empty; it is then removed.
  await change(t("agent.outputSchema"), "");
  expect(current.output_schema).toBeUndefined();
  expect(reported("agentSchema")).toBe(false);
  expect(host.textContent).toContain(t("agent.schemaOptional"));
  // Claude can only write text formats here; the editor says so.
  await render(<Harness settings={{ runtime: "host", target_kind: "host", target_ref: "9" }} />);
  await change(t("agent.agent"), "14");
  expect(host.textContent).toContain(t("agent.filesClaude"));
  expect(reported("agentFiles")).toBe(false);
  // Turning files off makes the schema required again.
  await act(async () => field<HTMLInputElement>(t("agent.files")).click());
  expect(current.files).toMatchObject({ enabled: false });
  expect(reported("agentSchema")).toBe(true);
  expect(parseFormats(" .Docx ; md;md ")).toEqual(["docx", "md"]);
  expect(formatsProblem(["toolongextension"])).toBe(true);
});
