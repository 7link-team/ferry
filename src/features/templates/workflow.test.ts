import { expect, it } from "vitest";
import { patchTemplateWorkflow, templateWorkflow } from "./workflow";

it("round-trips template parameters, runtime expressions, bindings and extension metadata through workflow edits", () => {
  const workflow = "name: Example\njobs:\n  build:\n    runs-on: self-hosted\n    steps:\n      - run: echo {{ .Values.command }} ${{ github.ref }}\n";
  const envelope = {
    engine: "github-actions", workflow,
    executor_bindings: [{ job_id: "build", target_kind: "host", target_ref: "7", image: "host" }],
    checkout_bindings: [{ job_id: "build", step_id: "checkout", repository_id: "9" }],
    graph_layout: { build: { x: 20, y: 40 } }, extension: { retained: true },
  };
  const loaded = templateWorkflow(JSON.stringify(envelope));
  const edited = templateWorkflow(patchTemplateWorkflow(loaded, { definition: workflow.replace("echo ", "printf ") }).content);
  expect(edited).toEqual({ ...envelope, workflow: workflow.replace("echo ", "printf ") });
  expect(patchTemplateWorkflow(edited, { graph_layout: { build: { x: 80, y: 100 } } }).content).toContain('"x": 80');
  expect(templateWorkflow(patchTemplateWorkflow(loaded, { checkout_bindings: [] }).content).checkout_bindings).toEqual([]);
});

it("reads YAML envelopes without expanding creation-time placeholders", () => {
  expect(templateWorkflow('engine: github-actions\nworkflow: |\n  name: "{{ .Values.name }}"\n')).toEqual({ engine: "github-actions", workflow: 'name: "{{ .Values.name }}"\n' });
});

it.each(['null', '[]', '{broken', '{"engine":"other","workflow":"test"}', '{"engine":"github-actions","workflow":1}'])("refuses malformed template content instead of overwriting it: %s", content => {
  expect(() => templateWorkflow(content)).toThrow();
});
