package main

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
	"time"
)

func TestPlanAndSingleInstanceProtocol(t *testing.T) {
	root := repository(t)
	workflow := `name: planning
on:
  workflow_dispatch:
    inputs:
      flavor:
        type: string
jobs:
  prepare:
    runs-on: self-hosted
    steps:
      - run: exit 99
  build:
    needs: prepare
    name: build-${{ matrix.os }}-${{ inputs.flavor }}
    runs-on: deliberately-unmapped
    strategy:
      max-parallel: 2
      matrix: ${{ fromJSON(needs.prepare.outputs.matrix) }}
    outputs:
      value: ${{ steps.probe.outputs.value }}
    steps:
      - id: probe
        run: |
          echo must-not-run
`
	r := Request{Workflow: workflow, Job: "build", Workdir: root, Mode: "plan", CacheDir: filepath.Join(root, "must-not-exist"), Inputs: map[string]string{"flavor": "debug"}, Needs: map[string]JobResult{"prepare": {State: "success", Outputs: map[string]string{"matrix": `{"os":["one","two"],"include":[{"os":"three"}],"exclude":[{"os":"two"}]}`}}}}
	var logs bytes.Buffer
	plan, err := execute(context.Background(), r, &logs)
	if err != nil || plan.State != "success" || len(plan.Instances) != 2 {
		t.Fatalf("plan=%+v err=%v logs=%s", plan, err, logs.String())
	}
	for index, want := range []string{"one", "three"} {
		if plan.Instances[index].Name != "build-"+want+"-debug" || plan.Instances[index].Matrix["os"] != want {
			t.Fatalf("instance %d: %+v", index, plan.Instances[index])
		}
	}
	if _, err := os.Stat(r.CacheDir); !os.IsNotExist(err) {
		t.Fatal("plan mode started cache or steps")
	}
	if logs.Len() != 0 {
		t.Fatalf("plan unexpectedly ran engine: %s", logs.String())
	}
	// Execute exactly the server-selected leg, retaining the complete strategy.
	r.Mode = ""
	r.Matrix = plan.Instances[1].Matrix
	r.Platforms = map[string]string{"deliberately-unmapped": "-self-hosted"}
	r.Workflow = strings.Replace(r.Workflow, "echo must-not-run", platformScript("echo 'value=${{ matrix.os }}/${{ strategy.job-total }}/${{ strategy.max-parallel }}' >> \"$GITHUB_OUTPUT\"", "'value=${{ matrix.os }}/${{ strategy.job-total }}/${{ strategy.max-parallel }}' >> $env:GITHUB_OUTPUT"), 1)
	value, err := execute(context.Background(), r, &logs)
	if err != nil || value.Outputs["value"] != "three/2/2" {
		t.Fatalf("single instance=%+v err=%v logs=%s", value, err, logs.String())
	}
	encoded, err := json.Marshal(struct {
		Kind string `json:"kind"`
		JobResult
	}{"result", value})
	if err != nil {
		t.Fatal(err)
	}
	var wire map[string]any
	if err = json.Unmarshal(encoded, &wire); err != nil || wire["kind"] != "result" || wire["continue_on_error"] != false {
		t.Fatalf("wire=%s err=%v", encoded, err)
	}
}

func TestRequestGithubContext(t *testing.T) {
	root := repository(t)
	script := platformScript("echo 'value=${{ github.actor }}/${{ github.run_number }}/${{ github.run_attempt }}/${{ github.ref }}/${{ github.ref_name }}/${{ github.sha }}/${{ github.repository }}/${{ github.server_url }}' >> \"$GITHUB_OUTPUT\"", "'value=${{ github.actor }}/${{ github.run_number }}/${{ github.run_attempt }}/${{ github.ref }}/${{ github.ref_name }}/${{ github.sha }}/${{ github.repository }}/${{ github.server_url }}' >> $env:GITHUB_OUTPUT")
	wf := "name: context\non: workflow_dispatch\njobs:\n  build:\n    runs-on: self-hosted\n    outputs:\n      value: ${{ steps.probe.outputs.value }}\n    steps:\n      - id: probe\n        run: |\n          " + script + "\n"
	r := Request{Workflow: wf, Job: "build", Workdir: root, CacheDir: t.TempDir(), Platforms: map[string]string{"self-hosted": "-self-hosted"}, Actor: "tester", RunNumber: 27, RunAttempt: 3, Ref: "refs/heads/test", SHA: "012345", Repository: "owner/repository", ServerURL: "https://example.invalid"}
	var logs bytes.Buffer
	value, err := execute(context.Background(), r, &logs)
	if err != nil || value.Outputs["value"] != "tester/27/3/refs/heads/test/test/012345/owner/repository/https://example.invalid" {
		t.Fatalf("context=%+v err=%v logs=%s", value, err, logs.String())
	}
}

func TestAlreadyCancelledCleanupJob(t *testing.T) {
	script := platformScript("echo cleanup", "Write-Output cleanup")
	wf := "name: cancellation\non: workflow_dispatch\njobs:\n  build:\n    if: cancelled() && !success() && always()\n    runs-on: self-hosted\n    steps:\n      - id: ordinary\n        run: exit 99\n      - id: cleanup\n        if: cancelled() && !success() && always()\n        run: " + script + "\n"
	var logs bytes.Buffer
	value, err := execute(context.Background(), Request{Workflow: wf, Job: "build", Workdir: repository(t), CacheDir: t.TempDir(), RunCancelled: true, Platforms: map[string]string{"self-hosted": "-self-hosted"}}, &logs)
	if err != nil || value.Steps["ordinary"] != "skipped" || value.Steps["cleanup"] != "success" {
		t.Fatalf("cleanup=%+v err=%v logs=%s", value, err, logs.String())
	}
}

func TestCacheSaveThenRestoreAcrossInvocations(t *testing.T) {
	root, cache := repository(t), t.TempDir()
	compatWrite(t, root, "cache-action/action.yml", "name: cache protocol fixture\ndescription: offline cache client\nruns:\n  using: node20\n  main: main.cjs\n")
	compatWrite(t, root, "cache-action/main.cjs", `const http = require('http');
const fs = require('fs');
async function request(url, method='GET', body, headers={}) {
  return new Promise((resolve,reject) => {
    const r = http.request(url, {method,headers:{Authorization:'Bearer '+process.env.ACTIONS_RUNTIME_TOKEN,...headers}}, s => {
      let chunks=[]; s.on('data',c=>chunks.push(c)); s.on('end',()=> {
        const data=Buffer.concat(chunks).toString();
        if(s.statusCode>=300) reject(new Error(s.statusCode+':'+data)); else resolve(data);
      });
    }); r.on('error',reject); if(body) r.write(body); r.end();
  });
}
(async()=>{
  const url=new URL(process.env.ACTIONS_CACHE_URL);
  if(url.hostname!=='127.0.0.1'||!url.port||!process.env.ACTIONS_RUNTIME_TOKEN) throw Error('missing private runtime');
  const api=url.href+'_apis/artifactcache';
  if(process.env.PHASE==='save') {
    const data='cache-proof';
    const reserve=JSON.parse(await request(api+'/caches','POST',JSON.stringify({key:'proof',version:'v1',cacheSize:data.length}),{'Content-Type':'application/json'}));
    await request(api+'/caches/'+reserve.cacheId,'PATCH',data,{'Content-Range':'bytes 0-'+(data.length-1)+'/*'});
    await request(api+'/caches/'+reserve.cacheId,'POST',JSON.stringify({size:data.length}),{'Content-Type':'application/json'});
    fs.appendFileSync(process.env.GITHUB_OUTPUT,'value=saved\n');
  } else {
    const hit=JSON.parse(await request(api+'/cache?keys=proof&version=v1'));
    if(hit.cacheKey!=='proof'||await request(hit.archiveLocation)!=='cache-proof') throw Error('restore miss or corrupt content');
    fs.appendFileSync(process.env.GITHUB_OUTPUT,'value=hit\n');
  }
})().catch(e=>{console.error(e);process.exitCode=1});
`)
	wf := "name: cache\non: workflow_dispatch\njobs:\n  build:\n    runs-on: self-hosted\n    outputs:\n      value: ${{ steps.cache.outputs.value }}\n    steps:\n      - uses: actions/checkout@v4\n      - id: cache\n        uses: ./cache-action\n"
	for index, phase := range []string{"save", "restore"} {
		t.Run(phase, func(t *testing.T) {
			var logs bytes.Buffer
			value, err := execute(context.Background(), Request{Workflow: wf, Job: "build", Workdir: root, CacheDir: cache, Env: map[string]string{"PHASE": phase}, Platforms: map[string]string{"self-hosted": "-self-hosted"}}, &logs)
			if err != nil || value.Outputs["value"] != []string{"saved", "hit"}[index] {
				t.Fatalf("cache=%+v err=%v logs=%s", value, err, logs.String())
			}
		})
	}
}

func TestCleanupContextBound(t *testing.T) {
	workflow := "on: workflow_dispatch\njobs:\n  build:\n    if: always()\n    runs-on: self-hosted\n    steps:\n      - id: hung\n        if: always()\n        run: " + platformScript("sleep 70", "Start-Sleep -Seconds 70") + "\n"
	start := time.Now()
	var logs bytes.Buffer
	value, err := execute(context.Background(), Request{Workflow: workflow, Job: "build", Workdir: repository(t), CacheDir: t.TempDir(), RunCancelled: true, TimeoutSeconds: 60, Platforms: map[string]string{"self-hosted": "-self-hosted"}}, &logs)
	elapsed := time.Since(start)
	if err == nil || value.State != "cancelled" || elapsed < cancellationGrace-time.Second || elapsed > cancellationGrace+4*time.Second {
		t.Fatalf("hung cleanup=%+v err=%v elapsed=%s bound=%s logs=%s", value, err, elapsed, cancellationGrace, logs.String())
	}
}

func TestProtocolRequestJSON(t *testing.T) {
	want := Request{Actor: "a", RunNumber: 7, RunAttempt: 2, Ref: "r", SHA: "s", Repository: "o/r", ServerURL: "u", RunCancelled: true, Mode: "plan", Matrix: map[string]any{"n": "x"}, CacheDir: "cache"}
	data, err := json.Marshal(want)
	if err != nil {
		t.Fatal(err)
	}
	var got Request
	if err = json.Unmarshal(data, &got); err != nil || !reflect.DeepEqual(got, want) {
		t.Fatalf("round trip=%+v %v", got, err)
	}
	for _, name := range []string{"actor", "run_number", "run_attempt", "ref", "sha", "repository", "server_url", "run_cancelled", "mode", "matrix", "cache_dir"} {
		if !bytes.Contains(data, []byte(`"`+name+`":`)) {
			t.Fatalf("missing contract name %s in %s", name, data)
		}
	}
}

func TestPlanPreservesDeclaredAxisOrder(t *testing.T) {
	wf := "on: workflow_dispatch\njobs:\n  build:\n    runs-on: no-execution\n    strategy:\n      matrix:\n        z: [a, b]\n        a: [1, 2]\n    steps:\n      - run: exit 99\n"
	for _, source := range []string{wf, strings.Replace(wf, "matrix:\n        z: [a, b]\n        a: [1, 2]", "matrix: ${{ fromJSON(vars.MATRIX) }}", 1)} {
		for attempt := 0; attempt < 10; attempt++ {
			value, err := execute(context.Background(), Request{Workflow: source, Job: "build", Mode: "plan", Vars: map[string]string{"MATRIX": `{"z":["a","b"],"a":[1,2]}`}}, &bytes.Buffer{})
			if err != nil || len(value.Instances) != 4 {
				t.Fatalf("plan=%+v err=%v", value, err)
			}
			for index, want := range []string{"a/1", "a/2", "b/1", "b/2"} {
				matrix := value.Instances[index].Matrix
				got := fmt.Sprint(matrix["z"]) + "/" + fmt.Sprint(matrix["a"])
				if got != want {
					t.Fatalf("plan order %d=%s want=%s", index, got, want)
				}
			}
		}
	}
}

func TestReusableWorkflowMatrixIsolatedAndTolerated(t *testing.T) {
	root := repository(t)
	compatWrite(t, root, "local/action.yml", "name: matrix probe\ndescription: offline matrix probe\ninputs:\n  leg:\n    description: matrix leg\n  strategy:\n    description: strategy context\nruns:\n  using: node20\n  main: main.cjs\n")
	compatWrite(t, root, "local/main.cjs", `const fs=require('fs'), path=require('path');
const leg=process.env.INPUT_LEG;
if(process.env.INPUT_STRATEGY!=='3/2') throw Error('incorrect strategy '+process.env.INPUT_STRATEGY);
if(leg==='bad') process.exit(7);
const start=Date.now();
setTimeout(()=>{
  fs.writeFileSync(path.join(process.env.PROOF_ROOT,leg+'.json'),JSON.stringify({start,end:Date.now()}));
  fs.appendFileSync(process.env.GITHUB_OUTPUT,leg+'='+leg+'\n');
},1500);
`)
	compatWrite(t, root, ".github/workflows/called.yml", `name: nested matrix
on:
  workflow_call:
    outputs:
      one:
        value: ${{ jobs.child.outputs.one }}
      two:
        value: ${{ jobs.child.outputs.two }}
jobs:
  child:
    runs-on: self-hosted
    continue-on-error: ${{ matrix.leg == 'bad' }}
    strategy:
      max-parallel: 2
      fail-fast: true
      matrix:
        leg: [one, two, bad]
    outputs:
      one: ${{ steps.probe.outputs.one }}
      two: ${{ steps.probe.outputs.two }}
    steps:
      - uses: actions/checkout@v4
      - id: probe
        uses: ./local
        with:
          leg: ${{ matrix.leg }}
          strategy: ${{ strategy.job-total }}/${{ strategy.max-parallel }}
`)
	var logs bytes.Buffer
	value, err := execute(context.Background(), Request{Workflow: "on: workflow_dispatch\njobs:\n  build:\n    uses: ./.github/workflows/called.yml\n", Job: "build", Workdir: root, CacheDir: t.TempDir(), Platforms: map[string]string{"self-hosted": "-self-hosted"}, Env: map[string]string{"PROOF_ROOT": root}}, &logs)
	if err != nil || value.State != "success" || value.Outputs["one"] != "one" || value.Outputs["two"] != "two" {
		t.Fatalf("nested=%+v err=%v logs=%s", value, err, logs.String())
	}
	var intervals [2]struct{ Start, End int64 }
	for index, name := range []string{"one", "two"} {
		data, e := os.ReadFile(filepath.Join(root, name+".json"))
		if e != nil {
			t.Fatal(e)
		}
		if e = json.Unmarshal(data, &intervals[index]); e != nil {
			t.Fatal(e)
		}
	}
	if intervals[0].Start >= intervals[1].End || intervals[1].Start >= intervals[0].End {
		t.Fatalf("nested matrix legs serialized: %+v", intervals)
	}
}

func TestMatrixOutputsFollowCompletionOrder(t *testing.T) {
	root := repository(t)
	compatWrite(t, root, "local/action.yml", "name: output probe\ndescription: offline output probe\ninputs:\n  leg:\n    description: matrix leg\nruns:\n  using: node20\n  main: main.cjs\n")
	compatWrite(t, root, "local/main.cjs", `const fs=require('fs');
const leg=process.env.INPUT_LEG;
setTimeout(()=>fs.appendFileSync(process.env.GITHUB_OUTPUT,'value='+(process.env.EMPTY==='true'&&leg==='slow'?'':leg)+'\n'),leg==='slow'?1200:10);
`)
	wf := `on: workflow_dispatch
jobs:
  build:
    runs-on: self-hosted
    strategy:
      max-parallel: 2
      matrix:
        leg: [slow, fast]
    outputs:
      value: ${{ steps.probe.outputs.value }}
    steps:
      - uses: actions/checkout@v4
      - id: probe
        uses: ./local
        with:
          leg: ${{ matrix.leg }}
`
	for _, empty := range []bool{false, true} {
		t.Run(fmt.Sprint(empty), func(t *testing.T) {
			var logs bytes.Buffer
			value, err := execute(context.Background(), Request{Workflow: wf, Job: "build", Workdir: root, CacheDir: t.TempDir(), Env: map[string]string{"EMPTY": fmt.Sprint(empty)}, Platforms: map[string]string{"self-hosted": "-self-hosted"}}, &logs)
			want := "slow"
			if empty {
				want = ""
			}
			got, present := value.Outputs["value"]
			if err != nil || !present || got != want {
				t.Fatalf("completion merge=%+v err=%v logs=%s", value, err, logs.String())
			}
		})
	}
}

func TestMatrixJobTimeoutAppliesPerInstance(t *testing.T) {
	root := repository(t)
	wf := "on: workflow_dispatch\njobs:\n  build:\n    runs-on: self-hosted\n    timeout-minutes: 1\n    strategy:\n      max-parallel: 1\n      matrix:\n        leg: [one, two]\n    steps:\n      - run: " + platformScript("sleep 35", "Start-Sleep -Seconds 35") + "\n"
	var logs bytes.Buffer
	start := time.Now()
	value, err := execute(context.Background(), Request{Workflow: wf, Job: "build", Workdir: root, CacheDir: t.TempDir(), TimeoutSeconds: 90, Platforms: map[string]string{"self-hosted": "-self-hosted"}}, &logs)
	if err != nil || value.State != "success" || time.Since(start) < 70*time.Second {
		t.Fatalf("per-instance timeout=%+v err=%v elapsed=%s logs=%s", value, err, time.Since(start), logs.String())
	}
}

func TestCancellationRunsRemainingStepsAndPostHooks(t *testing.T) {
	root := repository(t)
	compatWrite(t, root, "local/action.yml", "name: hooks\ndescription: offline hooks fixture\nruns:\n  using: node20\n  main: main.cjs\n  post: post.cjs\n")
	compatWrite(t, root, "local/main.cjs", `console.log('registered post');`)
	compatWrite(t, root, "local/post.cjs", `require('fs').writeFileSync(require('path').join(process.env.PROOF_ROOT,'post-marker'),'post');`)
	wf := "on: workflow_dispatch\njobs:\n  build:\n    runs-on: self-hosted\n    steps:\n      - uses: actions/checkout@v4\n      - uses: ./local\n      - id: waiting\n        run: " + platformScript("touch \"$PROOF_ROOT/started\"; sleep 20", "Set-Content $env:PROOF_ROOT/started yes; Start-Sleep -Seconds 20") + "\n      - id: cancelled\n        if: cancelled()\n        run: " + platformScript("echo cleaned", "Write-Output cleaned") + "\n      - id: always\n        if: always()\n        run: " + platformScript("echo always", "Write-Output always") + "\n"
	// Use a portable marker to cancel only after post registration and the
	// waiting step have started, avoiding a timing assumption about shell startup.
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	done := make(chan struct{})
	go func() {
		ticker := time.NewTicker(10 * time.Millisecond)
		defer ticker.Stop()
		for {
			select {
			case <-ticker.C:
				if _, err := os.Stat(filepath.Join(root, "started")); err == nil {
					cancel()
					return
				}
			case <-done:
				return
			}
		}
	}()
	var logs bytes.Buffer
	value, err := execute(ctx, Request{Workflow: wf, Job: "build", Workdir: root, CacheDir: t.TempDir(), TimeoutSeconds: 10, Env: map[string]string{"PROOF_ROOT": root}, Platforms: map[string]string{"self-hosted": "-self-hosted"}}, &logs)
	close(done)
	if err == nil || value.State != "cancelled" || value.Steps["cancelled"] != "success" || value.Steps["always"] != "success" {
		t.Fatalf("cancel=%+v err=%v logs=%s", value, err, logs.String())
	}
	if data, err := os.ReadFile(filepath.Join(root, "post-marker")); err != nil || string(data) != "post" {
		t.Fatalf("post hook missing: %q %v logs=%s", data, err, logs.String())
	}
}
