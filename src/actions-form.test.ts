import { afterEach, expect, it, vi } from "vitest";
import { formConfigs } from "./resources";
afterEach(() => vi.unstubAllGlobals());
it("legacy pipeline templates select the legacy editor instead of treating JSON as Actions YAML", async () => {
  vi.stubGlobal("fetch", vi.fn(async (url: string) => ({ ok: true, status: 200, json: async () => url.endsWith("/renderings") ? {definition:'{"workdir":"/tmp/build","steps":[{"name":"Build","run":"echo ok"}]}'} : {parameters:"[]"} })));
  const config = formConfigs("ops", new Set(["pipeline-templates.read"])).pipelines!;
  const generated = await config.generator!.generate("1", 1, {});
  expect(generated.definition_format).toBe("legacy");
  const draft={...config.blank,...generated};
  expect(draft.target_kind).toBe("host");
  expect(draft.target_ref).toBe("");
  expect(generated.checkout_bindings).toEqual([]);
  expect(JSON.parse(String(generated.definition)).steps[0].run).toBe("echo ok");
});
