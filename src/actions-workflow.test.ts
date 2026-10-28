import { describe, expect, it } from "vitest";
import { parseActions, editActionJob, editActionStep, connectActionJobs, removeActionJob, ensureActionCheckout, addActionJob, DEFAULT_ACTIONS } from "./actions-workflow";

const source = `# preserved workflow comment
name: build
on: workflow_dispatch
permissions:
  contents: read
jobs:
  test:
    runs-on: self-hosted
    strategy:
      matrix:
        node: [20, 22]
    steps:
      - uses: actions/checkout@v4
      - name: Compile
        run: npm run build
        env:
          TOKEN: \${{ secrets.BUILD_TOKEN }}
  package:
    runs-on: self-hosted
    steps:
      - run: echo package
`;
describe("Actions document editing", () => {
 it("visual command edits retain comments, expressions, unknown job configuration and workflow permissions", () => {
   const changed = editActionStep(source, "test", 1, { run: "npm ci && npm run build", "timeout-minutes": 10 });
   expect(changed).toContain("# preserved workflow comment");
   const parsed = parseActions(changed);
   expect(parsed.document.getIn(["jobs", "test", "strategy", "matrix", "node"])).toBeDefined();
   expect(parsed.jobs[0]!.steps[1]!.env).toEqual({ TOKEN: "${{ secrets.BUILD_TOKEN }}" });
   expect(parsed.jobs[0]!.steps[1]!.run).toBe("npm ci && npm run build");
   expect(parsed.document.toJS().permissions).toEqual({ contents: "read" });
 });
 it("connections become actual needs and cycles are rejected", () => {
   const changed = connectActionJobs(source, "test", "package");
   expect(parseActions(changed).jobs[1]!.needs).toEqual(["test"]);
   expect(() => connectActionJobs(changed, "package", "test")).toThrow(/循环/);
   expect(() => connectActionJobs(changed, "missing", "test")).toThrow(/不存在/);
 });
 it("job removal removes its references without corrupting other jobs", () => {
   const changed = removeActionJob(connectActionJobs(source, "test", "package"), "test");
   expect(parseActions(changed).jobs.map(job => job.id)).toEqual(["package"]);
   expect(parseActions(changed).jobs[0]!.needs).toEqual([]);
 });
 it("job panel edits preserve all unrelated steps", () => {
   const changed = editActionJob(source,"test",{name:"Build","timeout-minutes":30,"runs-on":["self-hosted","linux"]});
   const job = parseActions(changed).jobs[0]!;
   expect(job.name).toBe("Build");expect(job.runsOn).toEqual(["self-hosted","linux"]);
   expect(job.steps).toHaveLength(2);
 });
 it("invalid YAML, duplicate jobs and dangling dependencies block edits", () => {
   expect(() => parseActions("jobs: [")).toThrow();
   expect(() => parseActions("jobs:\n  a: {}\n  a: {}\n")).toThrow();
   expect(() => parseActions("jobs:\n  a:\n    needs: missing\n    runs-on: self-hosted\n    steps: []\n")).toThrow(/不存在/);
 });
});

it("adding a repository ensures checkout in each ordinary job without duplicating it", () => {
 const changed=ensureActionCheckout(source);
 const jobs=parseActions(changed).jobs;
 expect(jobs.every(job=>job.steps[0]?.uses==="actions/checkout@v4")).toBe(true);
 expect(ensureActionCheckout(changed)).toBe(changed);
 expect(changed).toContain("# preserved workflow comment");
});

it("new jobs without a repository remain executable command-only jobs", () => {
 const added = addActionJob(DEFAULT_ACTIONS);
 const job = parseActions(added.source).jobs.find(job => job.id === added.id)!;
 expect(job.steps.some(step => step.uses)).toBe(false);
 expect(job.steps.some(step => step.run)).toBe(true);
});

it("new jobs with a selected repository include checkout before commands", () => {
 const added = addActionJob(DEFAULT_ACTIONS, true);
 const job = parseActions(added.source).jobs.find(job => job.id === added.id)!;
 expect(job.steps[0]?.uses).toBe("actions/checkout@v4");
 expect(job.steps[1]?.run).toBe("echo hello");
});

it("serial successors and parallel siblings persist their different dependencies", () => {
 const serial = addActionJob(source, false, { job: "test", mode: "serial" });
 expect(parseActions(serial.source).jobs.find(job => job.id === serial.id)?.needs).toEqual(["test"]);
 const parallel = addActionJob(serial.source, false, { job: serial.id, mode: "parallel" });
 expect(parseActions(parallel.source).jobs.find(job => job.id === parallel.id)?.needs).toEqual(["test"]);
 expect(parallel.source).toContain("# preserved workflow comment");
 expect(() => connectActionJobs(parallel.source, parallel.id, "test")).toThrow(/循环/);
 const disconnected = editActionJob(parallel.source, parallel.id, { needs: [] });
 expect(parseActions(disconnected).jobs.find(job => job.id === parallel.id)?.needs).toEqual([]);
});

it("checkout steps can be added explicitly without changing command steps", async () => {
 const { addActionStep } = await import("./actions-workflow");
 const changed = addActionStep(DEFAULT_ACTIONS, "build", "checkout");
 expect(parseActions(changed).jobs[0]?.steps[1]?.uses).toBe("actions/checkout@v4");
 expect(parseActions(changed).jobs[0]?.steps[0]?.run).toBe("echo hello");
});
