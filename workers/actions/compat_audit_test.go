package main

// Table-driven compatibility regressions exercise real host executions. Network
// cases are opt-in; capabilities absent on this machine are explicit skips.
import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"testing"
	"time"
)

type compatObservation struct {
	Result  JobResult
	Err     error
	Log     string
	Elapsed time.Duration
	Root    string
}

type compatCase struct {
	name, workflow, state string
	setup                 func(*testing.T, string, *Request)
	check                 func(compatObservation) (string, string)
	want                  map[string]string
	remote                bool
}

func compatWorkflow(top, job, steps string) string {
	return "name: compatibility-audit\non: workflow_dispatch\n" + top + "jobs:\n  audit:\n    runs-on: self-hosted\n" + job + "    steps:\n" + steps
}

func compatRun(script string) string {
	return "      - id: probe\n        run: |\n          " + strings.ReplaceAll(strings.TrimSuffix(script, "\n"), "\n", "\n          ") + "\n"
}

func compatWrite(t *testing.T, root, name, value string) {
	t.Helper()
	path := filepath.Join(root, filepath.FromSlash(name))
	if err := os.MkdirAll(filepath.Dir(path), 0700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(value), 0600); err != nil {
		t.Fatal(err)
	}
}

// Only actual JSON messages/command fields are evidence. Do not mistake a
// displayed run script for proof that the script successfully executed.
func compatMessages(raw string) string {
	var messages []string
	for _, line := range strings.Split(raw, "\n") {
		var v map[string]any
		if json.Unmarshal([]byte(line), &v) == nil {
			msg, _ := v["msg"].(string)
			if v["command"] != nil {
				msg = fmt.Sprintf("command=%v level=%v properties=%v %s", v["command"], v["level"], v["kvPairs"], msg)
			}
			messages = append(messages, msg)
		}
	}
	return strings.Join(messages, " | ")
}

func TestCompatAudit(t *testing.T) {
	if runtime.GOOS != "windows" {
		t.Skip("this audit measures Windows host mode")
	}
	outputJob := "    outputs:\n      value: ${{ steps.probe.outputs.value }}\n"
	var cases []compatCase
	add := func(name, top, job, steps string, want map[string]string) {
		cases = append(cases, compatCase{name: name, workflow: compatWorkflow(top, job, steps), want: want})
	}
	for _, shell := range []string{"default", "pwsh", "powershell", "cmd", "bash"} {
		steps := compatRun("'value=PowerShell' >> $env:GITHUB_OUTPUT\nWrite-Output ('SHELL_VERSION=' + $PSVersionTable.PSVersion.ToString())")
		want := "PowerShell"
		if shell == "cmd" {
			steps = compatRun("echo value=cmd>> %GITHUB_OUTPUT%")
			want = "cmd"
		}
		if shell == "bash" {
			steps = compatRun("printf 'value=bash\\n' >> \"$GITHUB_OUTPUT\"")
			want = "bash"
		}
		if shell != "default" {
			steps = strings.Replace(steps, "        run:", "        shell: "+shell+"\n        run:", 1)
		}
		add("shell/"+shell, "", outputJob, steps, map[string]string{"value": want})
	}
	add("steps/working-directory", "", outputJob, strings.Replace(compatRun("'value=' + (Split-Path -Leaf (Get-Location)) >> $env:GITHUB_OUTPUT"), "        run:", "        working-directory: sub\n        run:", 1), map[string]string{"value": "sub"})
	cases[len(cases)-1].setup = func(t *testing.T, root string, _ *Request) { compatWrite(t, root, "sub/marker", "x") }
	add("env/workflow-job-step-precedence", "env:\n  A: workflow\n  B: workflow\n  C: workflow\n", "    env:\n      B: job\n      C: job\n"+outputJob, strings.Replace(compatRun("\"value=$env:A/$env:B/$env:C\" >> $env:GITHUB_OUTPUT"), "        run:", "        env:\n          C: step\n        run:", 1), map[string]string{"value": "workflow/job/step"})
	for _, scope := range []string{"workflow", "job"} {
		top, job := "", outputJob
		if scope == "workflow" {
			top = "defaults:\n  run:\n    shell: cmd\n    working-directory: sub\n"
		} else {
			top = "defaults:\n  run:\n    shell: bash\n    working-directory: wrong\n"
			job += "    defaults:\n      run:\n        shell: cmd\n        working-directory: sub\n"
		}
		add("defaults/"+scope+"-shell-directory", top, job, compatRun("if not exist marker exit /b 7\necho value=defaults-ok>> %GITHUB_OUTPUT%"), map[string]string{"value": "defaults-ok"})
		cases[len(cases)-1].setup = func(t *testing.T, root string, _ *Request) { compatWrite(t, root, "sub/marker", "x") }
	}
	add("if/success-failure-always-expressions", "", "    outputs:\n      value: ${{ steps.final.outputs.value }}\n", `      - id: initial
        if: success() && contains('abc', 'b')
        run: Write-Output INITIAL
      - id: bad
        run: exit 7
      - id: skipped
        if: success()
        run: exit 9
      - id: rescue
        if: failure()
        run: Write-Output RESCUE
      - id: final
        if: always() && !cancelled()
        run: "'value=${{ steps.skipped.outcome }}' >> $env:GITHUB_OUTPUT"
`, map[string]string{"value": "skipped"})
	cases[len(cases)-1].state = "failure"
	cases[len(cases)-1].check = func(o compatObservation) (string, string) {
		for id, want := range map[string]string{"initial": "success", "bad": "failure", "rescue": "success", "final": "success"} {
			if o.Result.Steps[id] != want {
				return "FAIL", fmt.Sprintf("step %s=%s want=%s", id, o.Result.Steps[id], want)
			}
		}
		return "PASS", "success() skipped after failure; failure()/always() executed"
	}
	add("steps/continue-on-error-outcome-conclusion", "", outputJob, `      - id: bad
        continue-on-error: true
        run: exit 7
      - id: probe
        if: success() && !failure() && always() && !cancelled()
        run: "'value=${{ steps.bad.outcome }}/${{ steps.bad.conclusion }}' >> $env:GITHUB_OUTPUT"
`, map[string]string{"value": "failure/success"})
	add("files/GITHUB_OUTPUT-multiline", "", outputJob, compatRun("'value<<END' >> $env:GITHUB_OUTPUT\n'line-one' >> $env:GITHUB_OUTPUT\n'line-two' >> $env:GITHUB_OUTPUT\n'END' >> $env:GITHUB_OUTPUT"), map[string]string{"value": "line-one\nline-two"})
	add("files/GITHUB_ENV", "", outputJob, "      - run: \"'AUDIT_ENV=next-step' >> $env:GITHUB_ENV\"\n"+compatRun("\"value=$env:AUDIT_ENV/${{ env.AUDIT_ENV }}\" >> $env:GITHUB_OUTPUT"), map[string]string{"value": "next-step/next-step"})
	for _, encoding := range []string{"utf8", "default-redirect"} {
		write := "$env:GITHUB_WORKSPACE + '/tools' | Out-File -Encoding utf8 -Append $env:GITHUB_PATH"
		if encoding == "default-redirect" {
			write = "$env:GITHUB_WORKSPACE + '/tools' >> $env:GITHUB_PATH"
		}
		add("files/GITHUB_PATH-"+encoding, "", outputJob, "      - run: \""+write+"\"\n"+compatRun("$v = audit-path.cmd\n\"value=$v\" >> $env:GITHUB_OUTPUT"), map[string]string{"value": "path-ok"})
		cases[len(cases)-1].setup = func(t *testing.T, root string, _ *Request) {
			compatWrite(t, root, "tools/audit-path.cmd", "@echo path-ok\r\n")
		}
	}
	for _, encoding := range []string{"utf8", "default-redirect"} {
		write := "'## Audit summary' | Out-File -Encoding utf8 -Append $env:GITHUB_STEP_SUMMARY"
		if encoding == "default-redirect" {
			write = "'## Audit summary' >> $env:GITHUB_STEP_SUMMARY"
		}
		add("files/GITHUB_STEP_SUMMARY-"+encoding, "", outputJob, compatRun(write+"\n'value=file-written' >> $env:GITHUB_OUTPUT"), map[string]string{"value": "file-written"})
		cases[len(cases)-1].check = func(o compatObservation) (string, string) {
			for _, line := range strings.Split(o.Log, "\n") {
				var record map[string]any
				if json.Unmarshal([]byte(line), &record) == nil && record["command"] == "summary" {
					content, _ := record["content"].(string)
					if strings.TrimSpace(strings.TrimPrefix(content, "\ufeff")) != "## Audit summary" {
						return "FAIL", fmt.Sprintf("structured summary content is corrupted: %q", content)
					}
					return "PASS", "summary exported as JSON command=summary with intact content; rendering belongs to the consumer"
				}
			}
			return "FAIL", "no structured summary record was emitted"
		}
	}
	add("commands/group-endgroup", "", "", compatRun("Write-Output '::group::Audit group'\nWrite-Output 'GROUP_BODY'\nWrite-Output '::endgroup::'"), nil)
	cases[len(cases)-1].check = func(o compatObservation) (string, string) {
		var commands []string
		for _, line := range strings.Split(o.Log, "\n") {
			var record map[string]any
			if json.Unmarshal([]byte(line), &record) == nil {
				if cmd, ok := record["command"].(string); ok {
					commands = append(commands, cmd)
				}
			}
		}
		if strings.Join(commands, "/") != "group/endgroup" {
			return "FAIL", fmt.Sprintf("group command sequence=%v", commands)
		}
		return "PASS", "group/endgroup retained in ordered JSON command records; collapsing is a consumer responsibility"
	}
	add("commands/error-warning", "", "", compatRun("Write-Output '::warning file=proof.ps1,line=3::audit-warning'\nWrite-Output '::error file=proof.ps1,line=4::audit-error'\nWrite-Output ANNOTATIONS_DONE"), nil)
	cases[len(cases)-1].check = func(o compatObservation) (string, string) {
		if !strings.Contains(o.Log, `"command":"warning"`) || !strings.Contains(o.Log, `"command":"error"`) {
			return "FAIL", "annotation command fields missing"
		}
		return "PASS", "warning/error retain levels and file/line properties; error annotation alone does not fail step"
	}
	add("commands/stop-commands", "", outputJob+"      ignored: ${{ steps.probe.outputs.ignored }}\n", compatRun("Write-Output '::stop-commands::audit-resume'\nWrite-Output '::set-output name=ignored::must-be-ignored'\nWrite-Output '::audit-resume::'\n'value=resumed' >> $env:GITHUB_OUTPUT"), map[string]string{"value": "resumed", "ignored": ""})
	maskJob := "    outputs:\n      hidden: ${{ steps.probe.outputs.hidden }}\n      value: ${{ steps.probe.outputs.value }}\n"
	add("commands/add-mask", "", maskJob, compatRun("$mask = 'dynamic-' + 'audit-value'\nWrite-Output \"::add-mask::$mask\"\nWrite-Output $mask\n\"hidden=$mask\" >> $env:GITHUB_OUTPUT\n'value=public' >> $env:GITHUB_OUTPUT"), map[string]string{"value": "public"})
	cases[len(cases)-1].check = func(o compatObservation) (string, string) {
		if strings.Contains(o.Log, "dynamic-audit-value") || o.Result.Outputs["hidden"] != "" {
			return "FAIL", "dynamic mask leaked"
		}
		return "PASS", "dynamic log redacted; secret output removed"
	}
	for _, kind := range []string{"plain", "multiline", "unicode"} {
		add("masking/"+kind, "", maskJob, strings.Replace(compatRun("Write-Output $env:TOKEN\n'hidden<<END' >> $env:GITHUB_OUTPUT\n$env:TOKEN >> $env:GITHUB_OUTPUT\n'END' >> $env:GITHUB_OUTPUT\n'value=public' >> $env:GITHUB_OUTPUT"), "        run:", "        env:\n          TOKEN: ${{ secrets.TOKEN }}\n        run:", 1), map[string]string{"value": "public"})
		secret := map[string]string{"plain": "audit-secret-517", "multiline": "audit-line-517\naudit-line-823", "unicode": "审计密钥-λ-audit-unicode-secret-823"}[kind]
		cases[len(cases)-1].setup = func(_ *testing.T, _ string, r *Request) { r.Secrets = map[string]string{"TOKEN": secret} }
		cases[len(cases)-1].check = func(o compatObservation) (string, string) {
			// A corrupted Unicode prefix must not hide a leaked ASCII suffix.
			if kind == "unicode" && strings.Contains(o.Log, "audit-unicode-secret-823") {
				return "FAIL", "Unicode secret's ASCII suffix leaked"
			}
			printedMask := false
			for _, line := range strings.Split(o.Log, "\n") {
				var record map[string]any
				if json.Unmarshal([]byte(line), &record) == nil && record["raw_output"] == true {
					msg, _ := record["msg"].(string)
					if strings.TrimSpace(msg) == "***" {
						printedMask = true
					}
				}
			}
			if !printedMask {
				return "FAIL", "no actually printed masked secret observed"
			}
			for _, part := range strings.Split(secret, "\n") {
				if strings.Contains(o.Log, part) {
					return "FAIL", "secret component present in serialized logs"
				}
			}
			if _, ok := o.Result.Outputs["hidden"]; ok {
				return "FAIL", "secret job output retained"
			}
			return "PASS", "secret logs redacted; secret output removed; public output preserved"
		}
	}
	for _, ex := range []struct{ name, expression, want string }{
		{"github", "format('{0}/{1}/{2}/{3}', github.repository, github.event_name, github.ref, github.sha != '')", "example/project/workflow_dispatch/refs/heads/master/true"},
		{"env", "env.AUDIT_CONTEXT", "env-value"},
		{"vars", "vars.AUDIT_VAR", "var-value"},
		{"secrets-static", "secrets.TOKEN == 'secret-context-value'", "true"},
		{"secrets-dynamic-matrix", "secrets[matrix.key] == 'secret-context-value'", "true"},
		{"secrets-dynamic-format", "secrets[format('{0}', vars.SECRET_KEY)] == 'secret-context-value'", "true"},
		{"job", "format('{0}/{1}', job.status, job.check_run_id != '')", "success/true"},
		{"runner-os", "runner.os", "Windows"},
		{"runner-temp", "runner.temp != ''", "true"},
		{"runner-tool_cache", "runner.tool_cache != ''", "true"},
		{"strategy", "format('{0}/{1}/{2}', strategy.fail-fast, strategy.max-parallel, strategy.job-total)", "false/2/1"},
		{"matrix", "matrix.key", "TOKEN"},
		{"needs", "format('{0}/{1}', needs.prepare.result, needs.prepare.outputs.ready)", "success/resolved"},
		{"inputs", "inputs.MESSAGE", "input-value"},
		{"contains", "contains(fromJSON('[\"a\",\"b\"]'), 'b')", "true"},
		{"startsWith", "startsWith('Audit', 'au')", "true"},
		{"endsWith", "endsWith('Audit', 'IT')", "true"},
		{"format", "format('{0}-{1}', 'a', 2)", "a-2"},
		{"join", "join(fromJSON('[\"a\",\"b\"]'), '/')", "a/b"},
		{"toJSON-fromJSON", "fromJSON(toJSON(fromJSON('{\"a\":7}'))).a", "7"},
	} {
		job := "    needs: prepare\n    strategy:\n      fail-fast: false\n      max-parallel: 2\n      matrix:\n        key: [TOKEN]\n" + outputJob
		wf := compatWorkflow("env:\n  AUDIT_CONTEXT: env-value\n", job, compatRun("'value=${{ "+ex.expression+" }}' >> $env:GITHUB_OUTPUT")) + "  prepare:\n    runs-on: self-hosted\n    steps:\n      - run: exit 99\n"
		if ex.name == "inputs" {
			wf = strings.Replace(wf, "on: workflow_dispatch", "on:\n  workflow_dispatch:\n    inputs:\n      MESSAGE:\n        type: string\n        required: true", 1)
		}
		if ex.name == "strategy" {
			wf = strings.Replace(wf, "key: [TOKEN]", "key: [TOKEN, OTHER]", 1)
			ex.want = "false/2/2"
		}
		cases = append(cases, compatCase{name: "expressions/" + ex.name, workflow: wf, want: map[string]string{"value": ex.want}, setup: func(_ *testing.T, _ string, r *Request) {
			r.Secrets = map[string]string{"TOKEN": "secret-context-value"}
			r.Vars = map[string]string{"AUDIT_VAR": "var-value", "SECRET_KEY": "TOKEN"}
			r.Inputs = map[string]string{"MESSAGE": "input-value"}
			r.Needs = map[string]JobResult{"prepare": {State: "success", Outputs: map[string]string{"ready": "resolved"}}}
		}})
	}
	hashScript := "'value=${{ hashFiles('hash.txt') }}' >> $env:GITHUB_OUTPUT\n$helper = Join-Path (Split-Path $env:GITHUB_OUTPUT) 'hashfiles/index.cjs'\n$env:patterns = 'hash.txt'\n& node $helper\n$global:LASTEXITCODE = 0"
	add("expressions/hashFiles", "", outputJob, compatRun(hashScript), nil)
	cases[len(cases)-1].setup = func(t *testing.T, root string, _ *Request) { compatWrite(t, root, "hash.txt", "audit-hash\n") }
	cases[len(cases)-1].check = func(o compatObservation) (string, string) {
		first := sha256.Sum256([]byte("audit-hash\n"))
		want := sha256.Sum256(first[:])
		if o.Result.Outputs["value"] != hex.EncodeToString(want[:]) {
			return "FAIL", "hashFiles differs from SHA256 of concatenated file hashes"
		}
		return "PASS", "hashFiles matches SHA256 of file digest"
	}
	// The required .tmp is inside a repository with type=module. Act's helper
	// scripts inherit that ancestor package scope. Measure an isolated cache too,
	// so a Node module-scope failure is not mistaken for broken hashing itself.
	isolatedHash := cases[len(cases)-1]
	isolatedHash.name = "expressions/hashFiles-isolated-cache"
	isolatedHash.setup = func(t *testing.T, root string, _ *Request) {
		compatWrite(t, root, "hash.txt", "audit-hash\n")
		compatWrite(t, filepath.Join(filepath.Dir(root), "action-cache"), "package.json", `{"type":"commonjs"}`)
	}
	cases = append(cases, isolatedHash)
	add("matrix/include-exclude-outputs", "", "    strategy:\n      matrix:\n        x: [one, two]\n        exclude:\n          - x: two\n        include:\n          - x: three\n    outputs:\n      one: ${{ steps.probe.outputs.one }}\n      three: ${{ steps.probe.outputs.three }}\n", compatRun("'${{ matrix.x }}=${{ matrix.x }}' >> $env:GITHUB_OUTPUT"), map[string]string{"one": "one", "three": "three"})
	for _, failFast := range []string{"true", "false"} {
		add("matrix/fail-fast-"+failFast, "", "    strategy:\n      max-parallel: 1\n      fail-fast: "+failFast+"\n      matrix:\n        leg: [bad, good]\n", compatRun("if ('${{ matrix.leg }}' -eq 'bad') { exit 7 }\nSet-Content $env:AUDIT_ROOT/later-leg executed"), nil)
		cases[len(cases)-1].state = "failure"
		stop := failFast == "true"
		cases[len(cases)-1].check = func(o compatObservation) (string, string) {
			_, err := os.Stat(filepath.Join(o.Root, "later-leg"))
			ran := err == nil
			if ran == stop {
				return "FAIL", fmt.Sprintf("later leg executed=%t", ran)
			}
			return "PASS", fmt.Sprintf("later leg executed=%t", ran)
		}
	}
	// Separate trace files avoid PowerShell 5.1 Add-Content's concurrent-file
	// sharing failure. The assertion still requires overlapping execution.
	add("matrix/max-parallel-2", "", "    strategy:\n      max-parallel: 2\n      matrix:\n        leg: [one, two]\n", compatRun("Add-Content $env:AUDIT_ROOT/${{ matrix.leg }}-order ([DateTime]::UtcNow.Ticks)\nStart-Sleep -Seconds 2\nAdd-Content $env:AUDIT_ROOT/${{ matrix.leg }}-order ([DateTime]::UtcNow.Ticks)"), nil)
	cases[len(cases)-1].check = func(o compatObservation) (string, string) {
		var ticks [4]int64
		for index, name := range []string{"one", "two"} {
			b, err := os.ReadFile(filepath.Join(o.Root, name+"-order"))
			fields := strings.Fields(string(b))
			if err != nil || len(fields) != 2 {
				return "FAIL", fmt.Sprintf("invalid trace %s: %q %v", name, b, err)
			}
			for j, field := range fields {
				ticks[index*2+j], err = strconv.ParseInt(field, 10, 64)
				if err != nil {
					return "FAIL", err.Error()
				}
			}
		}
		if ticks[0] < ticks[3] && ticks[2] < ticks[1] {
			return "PASS", fmt.Sprintf("overlapping intervals: %v", ticks)
		}
		return "FAIL", fmt.Sprintf("matrix legs serialized: %v", ticks)
	}
	add("job/if-false", "", "    if: ${{ false }}\n", compatRun("Set-Content should-not-run bad"), nil)
	cases[len(cases)-1].state = "skipped"
	cases[len(cases)-1].check = func(o compatObservation) (string, string) {
		if _, err := os.Stat(filepath.Join(o.Root, "should-not-run")); !os.IsNotExist(err) {
			return "FAIL", "false job executed"
		}
		return "PASS", "job skipped; no command executed"
	}
	add("job/continue-on-error", "", "    continue-on-error: true\n", compatRun("exit 7"), nil)
	cases[len(cases)-1].state = "failure"
	cases[len(cases)-1].check = func(o compatObservation) (string, string) {
		if !o.Result.ContinueOnError {
			return "FAIL", "job tolerance missing from result"
		}
		return "PASS", "failed result carries continue_on_error=true"
	}
	for _, scope := range []string{"step", "job"} {
		job, steps := "", compatRun("Start-Sleep -Seconds 70\nSet-Content $env:AUDIT_ROOT/late-write bad")
		if scope == "step" {
			steps = strings.Replace(steps, "        run:", "        timeout-minutes: 1\n        run:", 1)
		} else {
			job = "    timeout-minutes: 1\n"
		}
		add("timeouts/"+scope+"-one-minute", "", job, steps, nil)
		cases[len(cases)-1].state = "any"
		cases[len(cases)-1].setup = func(_ *testing.T, _ string, r *Request) { r.TimeoutSeconds = 75 }
		cases[len(cases)-1].check = func(o compatObservation) (string, string) {
			if o.Elapsed < 55*time.Second || o.Elapsed > 68*time.Second {
				return "FAIL", fmt.Sprintf("timeout elapsed=%s; wanted about 60s", o.Elapsed)
			}
			if _, err := os.Stat(filepath.Join(o.Root, "late-write")); !os.IsNotExist(err) {
				return "FAIL", "command survived timeout"
			}
			return "PASS", fmt.Sprintf("stopped at %s; state=%s error=%v", o.Elapsed.Round(time.Millisecond), o.Result.State, o.Err)
		}
	}
	add("timeouts/request-bound", "", "    timeout-minutes: 5\n", compatRun("Start-Sleep -Seconds 5\nSet-Content $env:AUDIT_ROOT/late-write bad"), nil)
	cases[len(cases)-1].state = "timed_out"
	cases[len(cases)-1].setup = func(_ *testing.T, _ string, r *Request) { r.TimeoutSeconds = 1 }
	add("uses/local-composite", "", outputJob, `      - id: action
        uses: ./composite
        with:
          message: composite-ok
      - id: probe
        run: "'value=${{ steps.action.outputs.value }}' >> $env:GITHUB_OUTPUT"
`, map[string]string{"value": "composite-ok"})
	cases[len(cases)-1].setup = func(t *testing.T, root string, _ *Request) {
		compatWrite(t, root, "composite/action.yml", `name: audit composite
description: audit fixture
inputs:
  message:
    description: value to echo
outputs:
  value:
    description: observed value
    value: ${{ steps.inner.outputs.value }}
runs:
  using: composite
  steps:
    - id: inner
      shell: powershell
      run: "'value=${{ inputs.message }}' >> $env:GITHUB_OUTPUT"
`)
	}
	add("uses/local-javascript-node20", "", outputJob, "      - id: action\n        uses: ./javascript\n"+compatRun("'value=${{ steps.action.outputs.value }}' >> $env:GITHUB_OUTPUT"), map[string]string{"value": "node-ok"})
	cases[len(cases)-1].setup = func(t *testing.T, root string, _ *Request) {
		if _, err := exec.LookPath("node"); err != nil {
			t.Log("AUDIT | uses/local-javascript-node20 | NOT SUPPORTED | node absent: " + err.Error())
			t.Skip("node absent")
		}
		compatWrite(t, root, "javascript/action.yml", "name: audit node\ndescription: audit fixture\nruns:\n  using: node20\n  main: index.js\n")
		compatWrite(t, root, "javascript/package.json", `{"type":"commonjs"}`)
		compatWrite(t, root, "javascript/index.js", "require('fs').appendFileSync(process.env.GITHUB_OUTPUT, 'value=node-ok\\n'); console.log('NODE_VERSION='+process.version);\n")
	}
	add("uses/local-docker", "", "", "      - uses: ./docker-action\n", nil)
	cases[len(cases)-1].state = "any"
	cases[len(cases)-1].check = compatUnsupported
	cases[len(cases)-1].setup = func(t *testing.T, root string, _ *Request) {
		compatWrite(t, root, "docker-action/action.yml", "name: audit docker\ndescription: audit fixture\nruns:\n  using: docker\n  image: Dockerfile\n")
		compatWrite(t, root, "docker-action/Dockerfile", "FROM alpine:3.20\nCMD [\"echo\",\"docker-ok\"]\n")
	}
	for _, scope := range []string{"container", "services"} {
		job := "    container: alpine:3.20\n"
		if scope == "services" {
			job = "    services:\n      redis:\n        image: redis:7-alpine\n        ports: ['6379:6379']\n"
		}
		add("job/"+scope, "", job, compatRun("'SERVICE_CONTEXT=${{ toJSON(job.services) }}'\nWrite-Output JOB_BODY_EXECUTED"), nil)
		cases[len(cases)-1].state = "any"
		cases[len(cases)-1].check = compatUnsupported
	}
	cases = append(cases, compatCase{name: "job/reusable-workflow", workflow: `name: reusable caller
on: workflow_dispatch
jobs:
  audit:
    uses: ./.github/workflows/reusable.yml
    with:
      message: reusable-ok
`, want: map[string]string{"value": "reusable-ok"}, setup: func(t *testing.T, root string, _ *Request) {
		compatWrite(t, root, ".github/workflows/reusable.yml", `name: audit reusable
on:
  workflow_call:
    inputs:
      message:
        required: true
        type: string
    outputs:
      value:
        value: ${{ jobs.inner.outputs.value }}
jobs:
  inner:
    runs-on: self-hosted
    outputs:
      value: ${{ steps.probe.outputs.value }}
    steps:
      - id: probe
        run: "'value=${{ inputs.message }}' >> $env:GITHUB_OUTPUT"
`)
	}})
	for _, action := range []string{"actions/checkout@v4", "actions/cache@v4"} {
		steps := "      - uses: " + action + "\n"
		if strings.Contains(action, "cache") {
			steps += "        with:\n          path: cache-data\n          key: audit-cache-unique-517\n"
			steps = "      - uses: actions/checkout@v4\n" + steps
		}
		add("uses/"+action, "", "", steps+compatRun("Write-Output REMOTE_ACTION_COMPLETED"), nil)
		cases[len(cases)-1].remote = true
		if strings.Contains(action, "cache") {
			cases[len(cases)-1].check = func(o compatObservation) (string, string) {
				if strings.Contains(o.Log, "CACHE_ENDPOINT=;RESULTS_ENDPOINT=") {
					return "NOT SUPPORTED", "action exits successfully but restore/save warn of cache backend errors; cache-hit=false; no cache endpoint/server"
				}
				return "PASS", "cache action completed with supplied endpoints (persistence not proven by this single invocation)"
			}
		}
		cases[len(cases)-1].setup = func(t *testing.T, root string, r *Request) {
			r.TimeoutSeconds = 120
			compatWrite(t, root, "cache-data/proof", "cache-proof")
			r.Workflow = strings.Replace(r.Workflow, "    steps:\n", "    steps:\n      - run: \"Write-Output ('CACHE_ENDPOINT=' + $env:ACTIONS_CACHE_URL + ';RESULTS_ENDPOINT=' + $env:ACTIONS_RESULTS_URL)\"\n", 1)
		}
	}
	add("uses/checkout-no-repository", "", "", "      - uses: actions/checkout@v4\n", nil)
	cases[len(cases)-1].remote = true
	cases[len(cases)-1].setup = func(t *testing.T, _ string, r *Request) { r.Workdir = t.TempDir(); r.TimeoutSeconds = 30 }
	for _, artifacts := range []bool{false, true} {
		add(fmt.Sprintf("cache/runtime-endpoints-artifacts-%t", artifacts), "", outputJob,
			compatRun("\"value=$([bool]$env:ACTIONS_CACHE_URL)\" >> $env:GITHUB_OUTPUT"), map[string]string{"value": "True"})
		if artifacts {
			cases[len(cases)-1].setup = func(t *testing.T, _ string, r *Request) { r.ArtifactDir = t.TempDir(); r.RunID = "518" }
		}
		cases[len(cases)-1].check = func(o compatObservation) (string, string) {
			if value := o.Result.Outputs["value"]; value != "" {
				return "PASS", "cache endpoint supplied: " + value + " (availability still requires the actions/cache case)"
			}
			return "NOT SUPPORTED", "ACTIONS_CACHE_URL is empty; worker starts no cache server; artifact runtime does not supply a cache endpoint"
		}
	}
	// Source-dependent features require checkout on GitHub too. Keep one explicit
	// no-checkout probe below rather than confusing an empty workspace with a
	// broken local action, hash function or working-directory implementation.
	for i := range cases {
		name := cases[i].name
		if name == "steps/working-directory" || strings.HasPrefix(name, "defaults/") || strings.HasPrefix(name, "files/GITHUB_PATH-") || strings.HasPrefix(name, "expressions/hashFiles") || name == "uses/local-composite" || name == "uses/local-javascript-node20" || name == "uses/local-docker" {
			cases[i].workflow = strings.Replace(cases[i].workflow, "    steps:\n", "    steps:\n      - uses: actions/checkout@v4\n", 1)
		}
	}
	add("workspace/request-workdir-without-checkout", "", outputJob, compatRun("\"value=$(Test-Path source-marker)\" >> $env:GITHUB_OUTPUT"), map[string]string{"value": "False"})
	cases[len(cases)-1].setup = func(t *testing.T, root string, _ *Request) {
		compatWrite(t, root, "source-marker", "present in request workdir")
	}
	add("reporting/skipped-step-state", "", "", "      - id: skip\n        if: ${{ false }}\n        run: exit 7\n"+compatRun("Write-Output SKIP_COMPLETED"), nil)
	cases[len(cases)-1].check = func(o compatObservation) (string, string) {
		if o.Result.Steps["skip"] != "skipped" {
			return "FAIL", "skipped step absent from JobResult.Steps (act logs skips at debug; worker collects info)"
		}
		return "PASS", "skipped step returned"
	}
	add("matrix/include-exclude", "", "    strategy:\n      max-parallel: 1\n      matrix:\n        x: [one, two]\n        exclude:\n          - x: two\n        include:\n          - x: three\n", compatRun("Add-Content $env:AUDIT_ROOT/legs '${{ matrix.x }}'"), nil)
	cases[len(cases)-1].check = func(o compatObservation) (string, string) {
		b, err := os.ReadFile(filepath.Join(o.Root, "legs"))
		if err != nil || strings.Join(strings.Fields(string(b)), "/") != "one/three" {
			return "FAIL", fmt.Sprintf("legs=%q error=%v", b, err)
		}
		return "PASS", "one/three executed; two excluded"
	}
	add("if/cancelled-after-cancellation", "", "", "      - id: sleeping\n        run: Start-Sleep -Seconds 10\n      - id: after\n        if: cancelled()\n        run: Set-Content $env:AUDIT_ROOT/cancelled-marker observed\n", nil)
	cases[len(cases)-1].state = "cancelled"
	cases[len(cases)-1].check = func(o compatObservation) (string, string) {
		if _, err := os.Stat(filepath.Join(o.Root, "cancelled-marker")); err != nil {
			return "FAIL", fmt.Sprintf("cancelled() cleanup branch did not run: %v", err)
		}
		return "PASS", "cancelled() cleanup branch ran after cancellation"
	}
	add("job/continue-on-error-matrix-fail-fast", "", "    continue-on-error: ${{ matrix.leg == 'bad' }}\n    strategy:\n      fail-fast: true\n      matrix:\n        leg: [bad, good]\n", compatRun("if ('${{ matrix.leg }}' -eq 'bad') { exit 7 }\nSet-Content $env:AUDIT_ROOT/tolerated-later-leg executed"), nil)
	cases[len(cases)-1].state = "any"
	cases[len(cases)-1].check = func(o compatObservation) (string, string) {
		if _, err := os.Stat(filepath.Join(o.Root, "tolerated-later-leg")); err != nil {
			return "FAIL", "job continue-on-error is ignored: tolerated bad matrix leg triggers fail-fast and prevents good leg"
		}
		return "PASS", "tolerated matrix failure did not cancel other legs"
	}
	add("job/if-always-failed-needs", "", "    needs: prepare\n    if: always()\n"+outputJob, compatRun("'value=${{ needs.prepare.result }}' >> $env:GITHUB_OUTPUT"), map[string]string{"value": "failure"})
	cases[len(cases)-1].workflow += "  prepare:\n    runs-on: self-hosted\n    steps:\n      - run: exit 99\n"
	cases[len(cases)-1].setup = func(_ *testing.T, _ string, r *Request) {
		r.Needs = map[string]JobResult{"prepare": {State: "failure"}}
	}
	counts := map[string]int{}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if tc.remote && os.Getenv("ACTIONS_TEST_REMOTE") != "1" {
				counts["SKIP"]++
				t.Skip("remote actions require network; local action/cache fixtures cover offline execution")
			}
			switch tc.name {
			case "uses/local-docker", "job/container", "job/services":
				counts["SKIP"]++
				t.Skip("Docker daemon unavailable on this Windows machine")
			case "expressions/job":
				counts["SKIP"]++
				t.Skip("job.check_run_id requires a check-run service; outside unit W")
			}
			root := repository(t)
			if out, err := testCommandContext(context.Background(), "git", "-C", root, "branch", "-M", "master").CombinedOutput(); err != nil {
				t.Fatalf("fixture branch: %v %s", err, out)
			}
			r := Request{Workflow: tc.workflow, Job: "audit", Workdir: root, CacheDir: t.TempDir(), Env: map[string]string{"AUDIT_ROOT": root}, Platforms: map[string]string{"self-hosted": "-self-hosted"}, TimeoutSeconds: 20}
			if tc.setup != nil {
				tc.setup(t, root, &r)
			}
			var logs bytes.Buffer
			ctx, cancel := context.WithCancel(context.Background())
			defer cancel()
			if tc.name == "if/cancelled-after-cancellation" {
				timer := time.AfterFunc(2*time.Second, cancel)
				defer timer.Stop()
			}
			start := time.Now()
			result, err := execute(ctx, r, &logs)
			o := compatObservation{Result: result, Err: err, Log: logs.String(), Elapsed: time.Since(start), Root: root}
			status, detail := "PASS", ""
			wantState := tc.state
			if wantState == "" {
				wantState = "success"
			}
			if wantState != "any" && result.State != wantState {
				status = "FAIL"
				detail = fmt.Sprintf("state=%s want=%s", result.State, wantState)
			}
			if wantState == "success" && err != nil {
				status = "FAIL"
				detail = fmt.Sprintf("execution error: %v", err)
			}
			for key, want := range tc.want {
				if got := result.Outputs[key]; got != want {
					status = "FAIL"
					detail += fmt.Sprintf(" output[%s]=%q want=%q", key, got, want)
				}
			}
			if tc.check != nil && status == "PASS" {
				status, detail = tc.check(o)
			}
			if tc.name == "shell/pwsh" && strings.Contains(o.Log, "Cannot find: pwsh") {
				status = "NOT SUPPORTED"
				detail = "PowerShell 7 absent; explicit pwsh does not fall back"
			}
			if tc.remote && status == "FAIL" && ((strings.Contains(o.Log, "git clone") && !strings.Contains(o.Log, "Run Main actions/")) || strings.Contains(o.Log, "Cannot find: node")) {
				status = "NOT SUPPORTED"
				detail = "remote action unavailable in this execution: " + detail
			}
			counts[status]++
			data, _ := json.Marshal(result)
			detail = strings.ReplaceAll(strings.ReplaceAll(detail, "\r\n", " / "), "\n", " / ")
			t.Logf("AUDIT | %s | %s | %s; result=%s; error=%v; elapsed=%s", tc.name, status, detail, data, err, o.Elapsed.Round(time.Millisecond))
			t.Logf("EVIDENCE | %s | %s", tc.name, compatMessages(o.Log))
			if status == "NOT SUPPORTED" {
				t.Skip(detail)
			}
			if status != "PASS" {
				t.Fatalf("compatibility assertion: %s", detail)
			}
		})
	}
	t.Logf("COMPAT COUNTS | PASS=%d FAIL=%d SKIP=%d", counts["PASS"], counts["FAIL"], counts["SKIP"]+counts["NOT SUPPORTED"])
}

func compatUnsupported(o compatObservation) (string, string) {
	if o.Err == nil && o.Result.State == "success" {
		return "NOT SUPPORTED", "job completed; inspect evidence for ignored declaration; container/service provisioning is not proven"
	}
	return "NOT SUPPORTED", fmt.Sprintf("Windows host action/job could not provision container: %v", o.Err)
}

func TestCompatAuditArtifacts(t *testing.T) {
	if runtime.GOOS != "windows" {
		t.Skip("Windows host audit")
	}
	root := repository(t)
	compatWrite(t, root, "proof.txt", "artifact-proof")
	directory := t.TempDir()
	workflow := `name: audit artifacts
on: workflow_dispatch
jobs:
  upload:
    runs-on: self-hosted
    steps:
      - uses: actions/upload-artifact@v4
        with:
          name: proof
          path: ${{ env.AUDIT_ROOT }}/proof.txt
  download:
    needs: upload
    runs-on: self-hosted
    steps:
      - uses: actions/download-artifact@v4
        with:
          name: proof
          path: received
      - run: |
          if ((Get-Content received/proof.txt -Raw) -ne 'artifact-proof') { exit 7 }
          New-Item -ItemType Directory -Force $env:AUDIT_ROOT/received | Out-Null
          Copy-Item received/proof.txt $env:AUDIT_ROOT/received/proof.txt
          Write-Output ARTIFACT_ROUNDTRIP_OK
`
	r := Request{Workflow: workflow, Workdir: root, ArtifactDir: directory, RunID: "517", Platforms: map[string]string{"self-hosted": "-self-hosted"}, TimeoutSeconds: 45}
	var upload JobResult
	for _, job := range []string{"upload", "download"} {
		t.Run(job, func(t *testing.T) {
			r.Job = job
			if job == "download" {
				r.Workdir = repository(t)
				r.Needs = map[string]JobResult{"upload": upload}
			}
			r.Env = map[string]string{"AUDIT_ROOT": r.Workdir}
			var logs bytes.Buffer
			result, err := execute(context.Background(), r, &logs)
			if job == "upload" {
				upload = result
			}
			status := "PASS"
			if err != nil || result.State != "success" {
				status = "FAIL"
				if strings.Contains(logs.String(), "git clone") && !strings.Contains(logs.String(), "⭐ Run Main actions/") {
					status = "NOT SUPPORTED"
				}
			}
			if status == "PASS" && job == "upload" && len(result.Artifacts) != 1 {
				status = "FAIL"
			}
			if status == "PASS" && job == "download" {
				b, e := os.ReadFile(filepath.Join(r.Workdir, "received", "proof.txt"))
				if e != nil || string(b) != "artifact-proof" {
					status = "FAIL"
				}
			}
			data, _ := json.Marshal(result)
			t.Logf("AUDIT | artifacts/%s-v4 | %s | result=%s error=%v", job, status, data, err)
			t.Logf("EVIDENCE | artifacts/%s-v4 | %s", job, compatMessages(logs.String()))
			if status != "PASS" {
				t.Fatalf("artifact compatibility: %s result=%s err=%v logs=%s", status, data, err, logs.String())
			}
		})
	}
}
