import { afterEach, expect, it, vi } from "vitest";
import { formConfigs } from "./resources";
import { DEFAULT_ACTIONS, parseActions } from "./actions-workflow";
afterEach(() => vi.unstubAllGlobals());
it("opens a converted legacy template in the Actions editor with its bindings", async () => {
  const binding = { job_id: "build", target_kind: "host", target_ref: "7", image: "host" };
  vi.stubGlobal("fetch", vi.fn(async (url: string) => ({ ok: true, status: 200, json: async () => url.endsWith("/renderings") ? {definition:JSON.stringify({engine:"github-actions",workflow:DEFAULT_ACTIONS,executor_bindings:[binding],checkout_bindings:[]})} : {parameters:"[]"} })));
  const config = formConfigs("ops", new Set(["pipeline-templates.read"])).pipelines!;
  const generated = await config.generator!.generate("1", 1, {});
  expect(generated.definition_format).toBe("github-actions");
  expect(generated.executor_bindings).toEqual([binding]);
  expect(generated.checkout_bindings).toEqual([]);
  expect(parseActions(String(generated.definition)).jobs[0]?.steps[0]?.run).toBe("echo hello");
  const definitionFields = config.sections.flatMap(section => section.fields).filter(field => field.key === "definition");
  expect(definitionFields).toHaveLength(1);
  expect(definitionFields[0]?.type).toBe("actions-editor");
  expect(definitionFields[0]?.when).toBeUndefined();
});
it("creates new pipeline templates as Actions workflows", () => {
  const config = formConfigs("ops")["pipeline-templates"]!;
  const content = JSON.parse(String(config.blank.content));
  expect(content.engine).toBe("github-actions");
  expect(content.workflow).toContain("{{ .Values.command }}");
  expect(content).not.toHaveProperty("workdir");
});
it("rejects a malformed template rendering instead of selecting a legacy editor", async () => {
  vi.stubGlobal("fetch", vi.fn(async (url: string) => ({ ok: true, status: 200, json: async () => url.endsWith("/renderings") ? {definition:'{"steps":[]}'} : {parameters:"[]"} })));
  const config = formConfigs("ops", new Set(["pipeline-templates.read"])).pipelines!;
  await expect(config.generator!.generate("1", 1, {})).rejects.toThrow("Actions");
});
