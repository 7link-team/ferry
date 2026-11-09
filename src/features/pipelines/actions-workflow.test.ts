import { beforeEach,describe,expect,it } from "vitest";
import { i18n } from "../../shared/i18n/i18n";
import { artifactNames, outputSources } from "./action-references";
import { addActionCheckouts,addActionStep,actionFragment,editActionFragment,addActionJob,connectActionJobs,DEFAULT_ACTIONS,duplicateActionJob,duplicateActionStep,editActionJob,editActionStep,editActionWorkflow,ensureActionCheckout,formatActionYaml,insertActionJob,parseActions,removeActionJob,setActionJobApproval } from "./actions-workflow";
beforeEach(() => { localStorage.setItem("ui.locale", "zh-CN"); void i18n.changeLanguage("zh-CN"); });

it("replaces the selected checkout in place with a batch before build, preserving its ID and controls", () => {
 const source = '# retain\njobs:\n  build:\n    runs-on: self-hosted\n    steps:\n      - id: source\n        uses: actions/checkout@v4\n        if: success()\n        env: &checkout_env\n          CI_TEST: yes\n        with:\n          fetch-depth: 5\n          repository: old/app\n      - run: make test\n        env: *checkout_env\n';
 const a={connection_id:'7',url:'https://git.example/team/app.git',name:'app',default_branch:'main'};
 const result=addActionCheckouts(source,'build',[a,{...a,url:'https://git.example/team/lib.git',name:'lib'}],[],0);
 const job=parseActions(result.definition).jobs[0]!;
 expect(job.steps).toHaveLength(3);
 expect(job.steps[0]).toMatchObject({id:'source',if:'success()',with:{'fetch-depth':5},env:{CI_TEST:'yes'}});
 expect(job.steps[1]).toMatchObject({if:'success()',env:{CI_TEST:'yes'}});
 expect(job.steps[2]?.run).toBe('make test');
 expect(job.steps[2]?.env).toEqual({CI_TEST:'yes'});
 expect(result.checkout_bindings.map(row=>row.step_id)).toEqual(['source',job.steps[1]!.id]);
 expect(job.steps[0]?.with).not.toHaveProperty('repository');
 expect(result.definition).toContain('# retain');
 const original={job_id:'build',step_id:'source',connection_id:a.connection_id,repository_url:a.url,ref:'feature/keep',path:'existing-source'};
 const retained=addActionCheckouts(source,'build',[{...a,url:'https://git.example/team/lib.git',name:'lib'},a],[original],0);
 expect(retained.checkout_bindings[0]).toMatchObject({...original,step_id:'source'});
});

it("creates distinct checkout steps from overlapping groups and preserves comments and unknown YAML", () => {
 const source='# keep\non: workflow_dispatch\nx-extension: keep\njobs:\n  build:\n    runs-on: self-hosted\n    steps:\n      - id: checkout_1\n        uses: actions/checkout@v4\n        with:\n          path: sources/app/child\n      - run: make test\n';
 const a={connection_id:'7',url:'https://git.example/team/app.git',name:'app',default_branch:'main'};
 const b={...a,url:'https://git.example/other/app.git',name:'App',default_branch:'release'};
 const added=addActionCheckouts(source,'build',[a,a,b],[]);
 expect(added.checkout_bindings).toHaveLength(2);
 expect(added.checkout_bindings.map(row=>row.step_id)).toEqual(['checkout_2','checkout_3']);
 expect(added.checkout_bindings.map(row=>row.path.toLowerCase())).toEqual(['sources/app-2','sources/app-3']);
 expect(added.checkout_bindings[1]?.ref).toBe('release');
 expect(added.definition).toContain('# keep');expect(added.definition).toContain('x-extension: keep');
 expect(parseActions(added.definition).jobs[0]?.steps).toHaveLength(4);
 expect(addActionCheckouts(added.definition,'build',[a,b],added.checkout_bindings)).toEqual(added);
 const differentConnection=addActionCheckouts(added.definition,'build',[{...a,connection_id:'8'}],added.checkout_bindings);
 expect(differentConnection.checkout_bindings).toHaveLength(3);
});

it("avoids a parent checkout directory and exposes separate clone, command and Action types", () => {
 const source='jobs:\n  build:\n    runs-on: self-hosted\n    steps:\n      - uses: actions/checkout@v4\n        with:\n          path: sources\n';
 const added=addActionCheckouts(source,'build',[{connection_id:'7',url:'https://git.example/team/a.git',name:'../测试',default_branch:'main'}],[]);
 expect(added.checkout_bindings[0]?.path).toBe('source-checkout_1');
 expect(parseActions(addActionStep(source,'build','checkout')).jobs[0]?.steps.at(-1)?.uses).toBe('actions/checkout@v4');
 expect(parseActions(addActionStep(source,'build','action')).jobs[0]?.steps.at(-1)?.uses).toBe('actions/setup-node@v4');
 expect(parseActions(addActionStep(source,'build','run')).jobs[0]?.steps.at(-1)?.run).toBeTruthy();
});

it("expands compact and JSON-shaped YAML without changing scripts, expressions, aliases or comments", () => {
 const compact = '# retained\non: workflow_dispatch\njobs: { build: { runs-on: self-hosted, env: &vars { FLAG: "false" }, steps: [ { run: "echo \\\"{\\\\\\\"ok\\\\\\\":true}\\\"\\necho done" }, { env: *vars, run: "${{ github.job }}" } ] } }\n';
 const original = parseActions(compact).document.toJS();
 const pretty = formatActionYaml(compact);
 expect(pretty).toContain('# retained');
 expect(pretty).toContain('jobs:\n  build:\n');
 expect(pretty).toContain('run: |-');
 expect(pretty).toContain('*vars');
 expect(parseActions(pretty).document.toJS()).toEqual(original);
 const json = JSON.stringify({ on: 'workflow_dispatch', jobs: { build: { steps: [{run:'echo "{\\"ok\\":true}"\r\necho done'}] } } });
 expect(parseActions(formatActionYaml(json)).document.toJS()).toEqual(JSON.parse(json));
 expect(formatActionYaml(pretty)).toBe(pretty);
 expect(() => formatActionYaml('jobs: [')).toThrow();
 expect(() => formatActionYaml('jobs: {}\njobs: {}')).toThrow();
});

it("keeps form-generated jobs in indented block YAML when starting from jobs: {}", () => {
 const added = addActionJob('on: workflow_dispatch\njobs: {}\n');
 expect(added.source).toContain('jobs:\n');
 expect(added.source).not.toContain('jobs: {');
 expect(actionFragment(added.source, added.id)).toContain('steps:\n');
});

it("starts from an empty draft and adds the first usable job without relaxing execution validation", () => {
 const empty = "on: workflow_dispatch\njobs: {}\n";
 expect(parseActions(empty, true).jobs).toHaveLength(0);
 expect(() => parseActions(empty)).toThrow();
 const added = addActionJob(empty);
 expect(parseActions(added.source).jobs).toHaveLength(1);
 expect(parseActions(added.source).jobs[0]?.id).toBe(added.id);
 expect(parseActions(added.source).jobs[0]?.steps[0]?.run).toContain("CI_");
});

it("suggests reachable artifacts and known step outputs without guessing producer expressions", () => {
 const { jobs } = parseActions(`jobs:
  build:
    runs-on: self-hosted
    outputs:
      version: \${{ steps.compile.outputs.version }}
    steps:
      - id: bundle
        uses: actions/upload-artifact@v4
        with: { name: bundle, path: dist }
      - uses: actions/upload-artifact@v4
        with: { name: '\${{ matrix.target }}', path: dist }
  test:
    needs: build
    steps:
      - uses: actions/upload-artifact@v4
        with: { path: test-results }
  unrelated:
    steps:
      - uses: actions/upload-artifact@v4
        with: { name: unrelated }
  release:
    needs: test
    steps:
      - id: local
        uses: actions/upload-artifact@v4
        with: { name: local }
      - id: download
        uses: actions/download-artifact@v4
      - uses: actions/upload-artifact@v4
        with: { name: future }
`);
 expect(artifactNames(jobs, "release", 1)).toEqual(["bundle", "artifact", "local"]);
 const references = outputSources(jobs, "release", 1);
 expect(references.find(source => source.id === "steps.local")?.outputs).toEqual(["artifact-id", "artifact-url", "artifact-digest"]);
 expect(references.some(source => source.id === "steps.download")).toBe(false);
 expect(references.some(source => source.id === "needs.build")).toBe(false);
 expect(outputSources(jobs, "test").find(source => source.id === "needs.build")?.outputs).toEqual(["version"]);
});

it("retains a custom runs-on expression through workflow, job, step and fragment edits", () => {
 const runner = 'runs-on=${{ github.run_id }}/family=c8a+m8a/cpu=4/ram=8+16/image=ubuntu24-full-x64/extras=s3-cache/volume=100gb:gp3';
 let changed = editActionJob(DEFAULT_ACTIONS, "build", { "runs-on": runner });
 changed = editActionWorkflow(changed, { name: "Custom runner" });
 changed = editActionStep(changed, "build", 0, { run: "make test" });
 changed = editActionFragment(changed, "build", undefined, actionFragment(changed, "build"));
 expect(parseActions(changed).jobs[0]!.raw["runs-on"]).toBe(runner);
});

it("copies jobs and steps without losing YAML comments, scalar types or anchor ownership", () => {
 const original = `env: &global
  GLOBAL: '001' # shared workflow value
jobs:
  build:
    runs-on: self-hosted
    env: *global
    steps:
      # keep step explanation
      - id: compile
        # explain command
        run: echo compile # keep command comment
        env: &local
          FLAG: false # keep boolean comment
          CODE: '002'
      - run: echo second
        env: *local
`;
 const copiedJob = duplicateActionJob(original, "build");
 const copiedStep = duplicateActionStep(copiedJob.source, "build", 0);
 const value = parseActions(copiedStep).document.toJS();
 expect(copiedStep.match(/# keep command comment/g)).toHaveLength(3);
 // This comment belongs to the steps sequence, copied with the job only.
 expect(copiedStep.match(/# keep step explanation/g)).toHaveLength(2);
 expect(copiedStep.match(/# explain command/g)).toHaveLength(3);
 expect(copiedStep.match(/# keep boolean comment/g)).toHaveLength(3);
 expect(copiedStep).toContain("env: *global");
 expect(value.jobs.build.steps[2].id).toBe("compile_copy");
 expect(value.jobs.build.steps[2].env).toEqual({ FLAG: false, CODE: "002" });
 const edited = editActionStep(copiedStep, copiedJob.id, 0, { env: { FLAG: true, CODE: "003" } });
 const final = parseActions(edited).document.toJS();
 expect(final.jobs[copiedJob.id].steps[1].env).toEqual({ FLAG: true, CODE: "003" });
 expect(final.jobs.build.steps[1].env).toEqual({ FLAG: false, CODE: "002" });
 expect(final.jobs.build.steps[2].env).toEqual({ FLAG: false, CODE: "002" });
});

it("inserts on one dependency without changing sibling branches, join dependencies or YAML comments", () => {
  const source = '# retain comment\non: workflow_dispatch\njobs:\n  a:\n    runs-on: self-hosted\n    steps: [{run: echo a}]\n  b:\n    runs-on: self-hosted\n    steps: [{run: echo b}]\n  join:\n    needs: [a, b]\n    runs-on: self-hosted\n    steps: [{run: echo joined}]\n  sibling:\n    needs: a\n    runs-on: self-hosted\n    steps: [{run: echo sibling}]\n';
  const added = insertActionJob(source, 'a', 'join');
  const parsed = parseActions(added.source).jobs;
  expect(parsed.find(job => job.id === added.id)?.needs).toEqual(['a']);
  expect(parsed.find(job => job.id === 'join')?.needs).toEqual([added.id, 'b']);
  expect(parsed.find(job => job.id === 'sibling')?.needs).toEqual(['a']);
  expect(added.source).toContain('# retain comment');
  expect(() => insertActionJob(source, 'b', 'sibling')).toThrow();
});

it("starts with neutral numbered steps and retains Action inputs in YAML", () => {
  expect(parseActions(DEFAULT_ACTIONS).jobs[0]!.steps[0]!.name).toBe("步骤1");
  const changed = editActionStep(DEFAULT_ACTIONS, "build", 0, { uses: "actions/setup-node@v4", run: null, with: { "node-version": "20", cache: "npm" } });
  expect(parseActions(changed).jobs[0]!.steps[0]!.with).toEqual({ "node-version": "20", cache: "npm" });
});

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
 it("job YAML edits preserve other jobs, validate needs and retain workflow anchor references", () => {
   const original = 'env: &shared\n  KEY: value\njobs:\n  first:\n    runs-on: self-hosted\n    env: *shared\n    steps: [{run: echo first}]\n  second:\n    needs: first\n    runs-on: self-hosted\n    steps: [{run: echo second}]\n';
   const fragment = actionFragment(original,"first");
   expect(fragment).toContain("env: *shared");
   const changed=editActionFragment(original,"first",undefined,fragment+'outputs:\n  version: ${{ steps.build.outputs.version }}\n');
   const value=parseActions(changed).document.toJS();
   expect(value.jobs.first.env).toEqual({KEY:"value"});
   expect(value.jobs.first.outputs.version).toBe("${{ steps.build.outputs.version }}");
   expect(value.jobs.second).toEqual(parseActions(original).document.toJS().jobs.second);
   expect(()=>editActionFragment(original,"first",undefined,fragment+'needs: second\n')).toThrow(/循环/);
   expect(()=>editActionFragment(original,"first",undefined,'- invalid shape\n')).toThrow();
 });
 it("workflow and job form edits preserve mapping comments, scalar types, expressions and unknown fields", () => {
   const original = `# workflow header
name: Before
on: [push, workflow_dispatch]
env:
  KEEP: '001' # keep quote and comment
  NUMBER: 42 # keep number
  EMPTY: null
  FLAG: false
  EDIT: before # keep edited comment
defaults:
  run:
    shell: bash # keep shell comment
    working-directory: ./source
  extension: unchanged
jobs:
  build:
    runs-on:
      group: build-workers # keep group
      labels: [self-hosted, linux]
    env:
      TOKEN: \${{ secrets.TOKEN }} # keep reference
      EDIT: before # job comment
    steps:
      - run: echo test
`;
   const value = parseActions(original).document.toJS();
   const changed = editActionWorkflow(original, { name: "After", env: { ...value.env, EDIT: "after" }, defaults: { ...value.defaults, run: { ...value.defaults.run, "working-directory": "./app" } } });
   const job = parseActions(changed).jobs[0]!;
   expect(job.runsOn).toEqual(["self-hosted", "linux"]);
   const final = editActionJob(changed, "build", { env: { ...(job.raw.env as object), EDIT: "changed" }, "runs-on": { ...(job.raw["runs-on"] as object), labels: ["self-hosted", "windows"] } });
   for (const comment of ["# workflow header", "# keep quote and comment", "# keep number", "# keep edited comment", "# keep shell comment", "# keep group", "# keep reference", "# job comment"]) expect(final).toContain(comment);
   expect(final).toContain("'001'");
   const result = parseActions(final).document.toJS();
   expect(result.env).toEqual({ KEEP: "001", NUMBER: 42, EMPTY: null, FLAG: false, EDIT: "after" });
   expect(result.on).toEqual(["push", "workflow_dispatch"]);
   expect(result.defaults).toEqual({ run: { shell: "bash", "working-directory": "./app" }, extension: "unchanged" });
   expect(result.jobs.build["runs-on"]).toEqual({ group: "build-workers", labels: ["self-hosted", "windows"] });
   expect(result.jobs.build.env.TOKEN).toBe("${{ secrets.TOKEN }}");
 });
 it("mapping edits detach an alias without changing the shared source or other jobs", () => {
   const original = `env: &shared
  SHARED: 'yes' # source comment
jobs:
  first:
    runs-on: self-hosted
    env: *shared
    steps: [{run: echo first}]
  second:
    runs-on: self-hosted
    env: *shared
    steps: [{run: echo second}]
`;
   const changed = editActionJob(original, "first", { env: { SHARED: "changed" } });
   const value = parseActions(changed).document.toJS();
   expect(value.env.SHARED).toBe("yes");
   expect(value.jobs.second.env.SHARED).toBe("yes");
   expect(value.jobs.first.env.SHARED).toBe("changed");
   expect(changed).toContain("# source comment");
   expect(changed).toContain("env: *shared");
 });
 it("removing a map row preserves unrelated typed rows and comments", () => {
   const original = editActionWorkflow(source, { env: { A: "one", B: 42, C: false, D: null } }).replace("B: 42", "B: 42 # typed row");
   const changed = editActionWorkflow(original, { env: { B: 42, C: false, D: null } });
   expect(parseActions(changed).document.toJS().env).toEqual({ B: 42, C: false, D: null });
   expect(changed).toContain("# typed row");
 });
 it("approval conversion removes unsupported execution fields and preserves sibling jobs", () => {
   const original = editActionJob(source, "test", { env: { KEY: "value" }, if: "success()", defaults: { run: { shell: "bash" } }, "continue-on-error": true });
   const changed = setActionJobApproval(original, "test", true);
   expect(parseActions(changed).jobs[0]!.raw).toEqual({ "ferry-approval": true });
   expect(parseActions(changed).jobs[1]!.raw).toEqual(parseActions(original).jobs[1]!.raw);
   expect(parseActions(setActionJobApproval(changed, "test", false)).jobs[0]!.steps[0]!.run).toBe('echo "CI_${{ github.job }}_STEP_1"');
 });
 it("writes a step fragment into the same document while retaining workflow and job configuration", () => {
   const fragment = actionFragment(source, "test", 1);
   const changed = editActionFragment(source, "test", 1, fragment + 'if: success()\ncontinue-on-error: true\n');
   const before = parseActions(source).document.toJS();
   before.jobs.test.steps[1].if = "success()";
   before.jobs.test.steps[1]["continue-on-error"] = true;
   expect(parseActions(changed).document.toJS()).toEqual(before);
   expect(changed).toContain("# preserved workflow comment");
   expect(() => editActionFragment(source, "test", 1, "- run: wrong shape\n")).toThrow();
   expect(() => editActionFragment(source, "test", 1, "with: [broken\n")).toThrow();
 });
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
 expect(job.steps[1]?.run).toBe('echo "CI_${{ github.job }}_STEP_2"');
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
 expect(parseActions(changed).jobs[0]?.steps[0]?.run).toBe('echo "CI_${{ github.job }}_STEP_1"');
});
