package main

import (
	"bytes"
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/nektos/act/pkg/model"
)

func TestManualGateWorkflowStillLoadsExecutableJob(t *testing.T) {
	workflow := `name: gate
on: workflow_dispatch
jobs:
  gate:
    needs: []
    runs-on: self-hosted
    steps:
      - run: echo approved
  deploy:
    needs: gate
    runs-on: self-hosted
    steps:
      - run: echo ready
`
	parsed, err := model.ReadWorkflow(strings.NewReader(workflow), false)
	if err != nil {
		t.Fatal(err)
	}
	if parsed.GetJob("deploy") == nil {
		t.Fatal("act did not find the executable job after a manual gate")
	}
}

func repository(t *testing.T) string {
	t.Helper()
	root := t.TempDir()
	for _, args := range [][]string{{"init", "-q"}, {"-c", "user.name=fixture", "-c", "user.email=fixture@example.invalid", "commit", "--allow-empty", "-qm", "fixture"}, {"remote", "add", "origin", "https://github.com/example/project.git"}} {
		c := exec.Command("git", append([]string{"-C", root}, args...)...)
		if out, err := c.CombinedOutput(); err != nil {
			t.Fatalf("git: %v %s", err, out)
		}
	}
	return root
}

func TestJobUsesDependencyOutputsWithoutRunningDependencyAgain(t *testing.T) {
	root := repository(t)
	workflow := `name: fixture
on: workflow_dispatch
jobs:
  prepare:
    runs-on: self-hosted
    steps:
      - run: exit 99
  build:
    needs: prepare
    runs-on: self-hosted
    outputs:
      artifact: ${{ steps.build.outputs.artifact }}
    steps:
      - id: build
        run: |
          test '${{ needs.prepare.outputs.value }}' = ready
          echo artifact=compiled >> "$GITHUB_OUTPUT"
          echo LIVE_PROOF
`
	var logs bytes.Buffer
	result, err := execute(context.Background(), Request{Workflow: workflow, Job: "build", Workdir: root, Needs: map[string]JobResult{"prepare": {State: "success", Outputs: map[string]string{"value": "ready"}}}, Platforms: map[string]string{"self-hosted": "-self-hosted"}}, &logs)
	if err != nil {
		t.Fatalf("%v: %s", err, logs.String())
	}
	if result.State != "success" || result.Outputs["artifact"] != "compiled" {
		t.Fatalf("wrong job result: %#v", result)
	}
	if !strings.Contains(logs.String(), "LIVE_PROOF") {
		t.Fatal("missing live log")
	}
}

func TestCancellationStopsActualWorkerCommand(t *testing.T) {
	root := repository(t)
	marker := filepath.Join(root, "must-not-exist")
	workflow := "name: cancel\non: workflow_dispatch\njobs:\n  build:\n    runs-on: self-hosted\n    steps:\n      - run: sleep 2; touch '" + marker + "'\n"
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	var logs bytes.Buffer
	result, _ := execute(ctx, Request{Workflow: workflow, Job: "build", Workdir: root, Platforms: map[string]string{"self-hosted": "-self-hosted"}}, &logs)
	if result.State == "success" {
		t.Fatal("cancelled job reported success")
	}
	if _, err := os.Stat(marker); !os.IsNotExist(err) {
		t.Fatal("cancelled command executed")
	}
}

func TestTimeoutStopsRunningCommand(t *testing.T) {
	root := repository(t)
	marker := filepath.Join(root, "late-write")
	workflow := "name: timeout\non: workflow_dispatch\njobs:\n  build:\n    runs-on: self-hosted\n    steps:\n      - run: sleep 3; touch '" + marker + "'\n"
	var logs bytes.Buffer
	started := time.Now()
	result, err := execute(context.Background(), Request{Workflow: workflow, Job: "build", Workdir: root, TimeoutSeconds: 1, Platforms: map[string]string{"self-hosted": "-self-hosted"}}, &logs)
	if err == nil || result.State != "timed_out" {
		t.Fatalf("expected timeout: %#v %v", result, err)
	}
	if time.Since(started) > 2500*time.Millisecond {
		t.Fatal("timeout did not stop execution promptly")
	}
	time.Sleep(3 * time.Second)
	if _, err := os.Stat(marker); !os.IsNotExist(err) {
		t.Fatal("timed-out command survived and wrote a file")
	}
}

func TestSecretNeverAppearsInLogsOrOutputs(t *testing.T) {
	root := repository(t)
	workflow := `name: secret
on: workflow_dispatch
jobs:
  build:
    runs-on: self-hosted
    outputs:
      value: ${{ steps.emit.outputs.value }}
    steps:
      - id: emit
        env:
          TOKEN: ${{ secrets.TOKEN }}
        run: |
          echo "$TOKEN"
          echo "value=$TOKEN" >> "$GITHUB_OUTPUT"
`
	var logs bytes.Buffer
	secret := "secret-probe-531b9d"
	result, err := execute(context.Background(), Request{Workflow: workflow, Job: "build", Workdir: root, Secrets: map[string]string{"TOKEN": secret}, Platforms: map[string]string{"self-hosted": "-self-hosted"}}, &logs)
	if err != nil {
		t.Fatalf("execution failed: %v", err)
	}
	if strings.Contains(logs.String(), secret) {
		t.Fatalf("secret exposed in logs: %s", logs.String())
	}
	if len(result.Outputs) != 0 {
		t.Fatal("secret output retained")
	}
}

func TestMatrixAndAlwaysAfterFailedDependency(t *testing.T) {
	root := repository(t)
	workflow := `name: matrix
on: workflow_dispatch
jobs:
  prepare:
    runs-on: self-hosted
    steps:
      - run: exit 99
  build:
    needs: prepare
    if: always()
    runs-on: self-hosted
    strategy:
      matrix:
        value: [one, two]
    steps:
      - run: echo MATRIX_${{ matrix.value }}
`
	var logs bytes.Buffer
	result, err := execute(context.Background(), Request{Workflow: workflow, Job: "build", Workdir: root, Needs: map[string]JobResult{"prepare": {State: "failure"}}, Platforms: map[string]string{"self-hosted": "-self-hosted"}}, &logs)
	if err != nil || result.State != "success" {
		t.Fatalf("matrix failed: %v %s", err, logs.String())
	}
	if result.Steps["0"] != "success" {
		t.Fatalf("matrix step result missing: %#v", result.Steps)
	}
	if result.Outputs == nil {
		t.Fatal("worker must return an empty outputs object, never null")
	}
	if !strings.Contains(logs.String(), "MATRIX_one") || !strings.Contains(logs.String(), "MATRIX_two") {
		t.Fatalf("missing matrix execution: %s", logs.String())
	}
}

func TestCommandWithoutRepositoryAndWithoutGitHead(t *testing.T) {
	root := t.TempDir()
	if out, err := exec.Command("git", "-C", root, "init", "-q").CombinedOutput(); err != nil {
		t.Fatalf("git init: %v %s", err, out)
	}
	var logs bytes.Buffer
	result, err := execute(context.Background(), Request{Workflow: "jobs:\n  build:\n    runs-on: self-hosted\n    steps:\n      - run: echo NO_REPO_OK\n", Job: "build", Workdir: root, Platforms: map[string]string{"self-hosted": "-self-hosted"}}, &logs)
	if err != nil || result.State != "success" {
		t.Fatalf("command without repository failed: %v %s", err, logs.String())
	}
	if !strings.Contains(logs.String(), "NO_REPO_OK") {
		t.Fatal("command did not execute")
	}
}

func TestArtifactActionsUploadAndDownloadAcrossJobs(t *testing.T) {
	if os.Getenv("ACTIONS_TEST_REMOTE") != "1" {
		t.Skip("set ACTIONS_TEST_REMOTE=1 to fetch pinned GitHub artifact actions")
	}
	root := repository(t)
	if err := os.WriteFile(filepath.Join(root, "proof.txt"), []byte("artifact-proof"), 0600); err != nil {
		t.Fatal(err)
	}
	workflow := `name: artifacts
on: workflow_dispatch
jobs:
  upload:
    runs-on: self-hosted
    steps:
      - uses: actions/checkout@v4
      - uses: actions/upload-artifact@v4
        with:
          name: proof
          path: proof.txt
  download:
    needs: upload
    runs-on: self-hosted
    steps:
      - uses: actions/download-artifact@v4
        with:
          name: proof
          path: received
      - run: test "$(cat received/proof.txt)" = artifact-proof
        shell: sh
`
	directory := t.TempDir()
	var logs bytes.Buffer
	request := Request{Workflow: workflow, Job: "upload", Workdir: root, ArtifactDir: directory, RunID: "41", Platforms: map[string]string{"self-hosted": "-self-hosted"}}
	if image := os.Getenv("ACTIONS_TEST_DOCKER_IMAGE"); image != "" {
		request.Platforms["self-hosted"] = image
	}
	result, err := execute(context.Background(), request, &logs)
	if err != nil || result.State != "success" || len(result.Artifacts) != 1 {
		t.Fatalf("upload: %v %#v %s", err, result, logs.String())
	}
	request.Job = "download"
	request.Workdir = repository(t)
	request.Needs = map[string]JobResult{"upload": result}
	logs.Reset()
	result, err = execute(context.Background(), request, &logs)
	if err != nil || result.State != "success" {
		t.Fatalf("download: %v %#v %s", err, result, logs.String())
	}
}

func TestJobTimeBudgetHonorsExplicitTimeoutAndDefault(t *testing.T) {
	for _, test := range []struct {
		limit   int
		minutes string
		want    int
	}{{0, "", 1800}, {86400, "", 1800}, {86400, "60", 3600}, {1, "60", 1}, {86400, "1440", 86400}} {
		seconds, err := jobTimeBudget(test.limit, test.minutes)
		if err != nil || seconds != test.want {
			t.Fatalf("budget(%d,%q)=%d,%v want %d", test.limit, test.minutes, seconds, err, test.want)
		}
	}
	for _, minutes := range []string{"0", "-1", "1441", "bad"} {
		if _, err := jobTimeBudget(86400, minutes); err == nil {
			t.Fatalf("accepted invalid timeout %q", minutes)
		}
	}
}

func TestMatrixFailFastStopsLaterBranch(t *testing.T) {
	for _, failFast := range []string{"", "      fail-fast: false\n"} {
		t.Run(strings.TrimSpace(failFast), func(t *testing.T) {
			root := repository(t)
			marker := filepath.Join(root, "later-branch")
			workflow := "name: matrix\non: workflow_dispatch\njobs:\n  build:\n    runs-on: self-hosted\n    strategy:\n" + failFast + "      matrix:\n        value: [bad, good]\n    steps:\n      - id: command\n        run: |\n          if [ '${{ matrix.value }}' = bad ]; then exit 7; fi\n          touch '" + marker + "'\n"
			var logs bytes.Buffer
			result, err := execute(context.Background(), Request{Workflow: workflow, Job: "build", Workdir: root, Platforms: map[string]string{"self-hosted": "-self-hosted"}}, &logs)
			if err == nil || result.State != "failure" || result.Steps["command"] != "failure" {
				t.Fatalf("matrix failure lost: %#v %v", result, err)
			}
			_, statErr := os.Stat(marker)
			if failFast == "" && !os.IsNotExist(statErr) {
				t.Fatal("default fail-fast executed later branch")
			}
			if failFast != "" && statErr != nil {
				t.Fatalf("fail-fast false did not execute later branch: %v", statErr)
			}
		})
	}
}

func TestDynamicMaskCannotEscapeThroughJobOutputs(t *testing.T) {
	root := repository(t)
	workflow := `name: dynamic mask
on: workflow_dispatch
jobs:
  build:
    runs-on: self-hosted
    outputs:
      hidden: ${{ steps.generate.outputs.hidden }}
      public: ${{ steps.generate.outputs.public }}
    steps:
      - id: generate
        run: |
          value="generated-sensitive-$((900 + 23))"
          echo "::add-mask::$value"
          echo "hidden=$value" >> "$GITHUB_OUTPUT"
          echo 'public=visible' >> "$GITHUB_OUTPUT"
`
	var logs bytes.Buffer
	result, err := execute(context.Background(), Request{Workflow: workflow, Job: "build", Workdir: root, Platforms: map[string]string{"self-hosted": "-self-hosted"}}, &logs)
	if err != nil {
		t.Fatal(err)
	}
	if _, leaked := result.Outputs["hidden"]; leaked {
		t.Fatal("dynamically masked output escaped")
	}
	if strings.Contains(logs.String(), "generated-sensitive-923") {
		t.Fatal("dynamically masked log escaped")
	}
	if result.Outputs["public"] != "visible" {
		t.Fatal("public output lost")
	}
}

// Independent worker processes on one host must not race over action checkouts.
func TestConcurrentArtifactWorkers(t *testing.T) {
	if os.Getenv("ACTIONS_TEST_REMOTE") != "1" {
		t.Skip("set ACTIONS_TEST_REMOTE=1 for real action downloads")
	}
	executable, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 6*time.Minute)
	defer cancel()
	type completion struct {
		output []byte
		err    error
	}
	completed := make(chan completion, 2)
	for i := 0; i < 2; i++ {
		go func() {
			output, err := exec.CommandContext(ctx, executable, "-test.run=^TestArtifactActionsUploadAndDownloadAcrossJobs$", "-test.count=1").CombinedOutput()
			completed <- completion{output, err}
		}()
	}
	for i := 0; i < 2; i++ {
		result := <-completed
		if result.err != nil {
			t.Errorf("concurrent worker failed: %v\n%s", result.err, result.output)
		}
	}
}

func TestMultilineSecretLinesNeverReachLogsOrOutputs(t *testing.T) {
	root := repository(t)
	workflow := `jobs:
  build:
    runs-on: self-hosted
    outputs:
      value: ${{ steps.emit.outputs.value }}
    steps:
      - id: emit
        env:
          TOKEN: ${{ secrets.TOKEN }}
        run: |
          printf '%s\n' "$TOKEN"
          printf 'value=%s\n' "${TOKEN%%
          *}" >> "$GITHUB_OUTPUT"
`
	var logs bytes.Buffer
	secret := "synthetic-first-line-123\nsynthetic-second-line-456"
	result, err := execute(context.Background(), Request{Workflow: workflow, Job: "build", Workdir: root, Secrets: map[string]string{"TOKEN": secret}, Platforms: map[string]string{"self-hosted": "-self-hosted"}}, &logs)
	if err != nil || result.State != "success" {
		t.Fatalf("execution failed: %v %v", err, result)
	}
	for _, line := range strings.Split(secret, "\n") {
		if strings.Contains(logs.String(), line) {
			t.Fatal("multiline secret line escaped into logs")
		}
	}
	if len(result.Outputs) != 0 {
		t.Fatal("multiline secret line escaped into outputs")
	}
}

func TestTransitiveDependencyStatusWithoutRerunningAncestors(t *testing.T) {
	for _, ancestor := range []string{"success", "failure"} {
		t.Run(ancestor, func(t *testing.T) {
			condition := "success()"
			if ancestor == "failure" {
				condition = "failure()"
			}
			workflow := `jobs:
  a:
    runs-on: self-hosted
    steps:
      - run: exit 99
  b:
    needs: a
    runs-on: self-hosted
    steps:
      - run: exit 99
  c:
    needs: b
    if: ` + condition + `
    runs-on: self-hosted
    steps:
      - run: echo TRANSITIVE_STATUS_OK
`
			var logs bytes.Buffer
			result, err := execute(context.Background(), Request{Workflow: workflow, Job: "c", Workdir: repository(t), Needs: map[string]JobResult{"a": {State: ancestor}, "b": {State: "success"}}, Platforms: map[string]string{"self-hosted": "-self-hosted"}}, &logs)
			if err != nil || result.State != "success" || !strings.Contains(logs.String(), "TRANSITIVE_STATUS_OK") {
				t.Fatalf("transitive status lost: %v %v", result, err)
			}
		})
	}
}
