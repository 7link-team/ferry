package main

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/nektos/act/pkg/model"
)

func previewStep(t *testing.T, uses string, with map[string]string) *model.Step {
	t.Helper()
	step := &model.Step{ID: "preview", Uses: uses, With: with}
	if err := step.Env.Encode(map[string]string{"KEEP": "kept"}); err != nil {
		t.Fatal(err)
	}
	return step
}

func TestRewriteBuiltinPreviewStep(t *testing.T) {
	for _, uses := range []string{"ferry/preview@v1", "ferry/preview@v1.2"} {
		step := previewStep(t, uses, map[string]string{"port": "5173", "strip-prefix": "false", "name": "$(rm -rf /)"})
		if err := rewriteBuiltinPreviewStep(step, "secret-nonce", "/tmp/actions-x/ferry-preview"); err != nil {
			t.Fatal(err)
		}
		env := map[string]string{}
		if err := step.Env.Decode(&env); err != nil {
			t.Fatal(err)
		}
		want := map[string]string{"KEEP": "kept", "INPUT_FERRY_PREVIEW_PORT": "5173", "INPUT_FERRY_PREVIEW_STRIP_PREFIX": "false",
			"INPUT_FERRY_PREVIEW_NAME": "$(rm -rf /)", "FERRY_PREVIEW_NONCE": "secret-nonce", "FERRY_PREVIEW_DIR": "/tmp/actions-x/ferry-preview", "FERRY_PREVIEW_STEP_NAME": ""}
		for key, value := range want {
			if env[key] != value {
				t.Errorf("%s: env[%s]=%q want %q", uses, key, env[key], value)
			}
		}
		if env["FERRY_PREVIEW_EXECUTABLE"] == "" || step.Uses != "" || step.With != nil || step.Name != uses {
			t.Fatalf("%s: step not rewritten: %+v", uses, step)
		}
		// User values never enter the shell source.
		if strings.Contains(step.Run, "rm -rf") || strings.Contains(step.Run, "5173") || !strings.Contains(step.Run, " preview") {
			t.Fatalf("run=%q", step.Run)
		}
	}
	for _, uses := range []string{"ferry/preview@v2", "ferry/preview@v10", "other/preview@v1", "actions/checkout@v4"} {
		step := previewStep(t, uses, map[string]string{"port": "1"})
		if err := rewriteBuiltinPreviewStep(step, "n", "d"); err != nil || step.Uses != uses || step.Run != "" {
			t.Fatalf("%s must stay untouched: %+v %v", uses, step, err)
		}
	}
}

func TestPreviewInputs(t *testing.T) {
	inputs := func(values map[string]string) func(string) string {
		return func(key string) string { return values[key] }
	}
	request, err := previewInputs(inputs(map[string]string{"INPUT_FERRY_PREVIEW_PORT": "5173", "FERRY_PREVIEW_STEP_NAME": "Frontend"}))
	if err != nil || request.Port != 5173 || !request.StripPrefix || request.Hold != "none" || request.ReadyTimeoutSeconds != 60 || request.TimeoutMinutes != 60 || request.Name != "Frontend" {
		t.Fatalf("defaults=%+v err=%v", request, err)
	}
	request, err = previewInputs(inputs(map[string]string{"INPUT_FERRY_PREVIEW_PORT": "65535", "INPUT_FERRY_PREVIEW_NAME": " Web ", "INPUT_FERRY_PREVIEW_STRIP_PREFIX": "False",
		"INPUT_FERRY_PREVIEW_HOLD": "approval", "INPUT_FERRY_PREVIEW_READY_TIMEOUT_SECONDS": "600", "INPUT_FERRY_PREVIEW_TIMEOUT_MINUTES": "1440"}))
	if err != nil || request.Port != 65535 || request.StripPrefix || request.Hold != "approval" || request.ReadyTimeoutSeconds != 600 || request.TimeoutMinutes != 1440 || request.Name != "Web" {
		t.Fatalf("explicit=%+v err=%v", request, err)
	}
	for _, tc := range []struct {
		name   string
		values map[string]string
	}{
		{"ready-timeout", map[string]string{"INPUT_FERRY_PREVIEW_PORT": "80", "INPUT_FERRY_PREVIEW_READY_TIMEOUT_SECONDS": "601"}},
		{"ready-timeout", map[string]string{"INPUT_FERRY_PREVIEW_PORT": "80", "INPUT_FERRY_PREVIEW_READY_TIMEOUT_SECONDS": "0"}},
		{"timeout-minutes", map[string]string{"INPUT_FERRY_PREVIEW_PORT": "80", "INPUT_FERRY_PREVIEW_TIMEOUT_MINUTES": "1441"}},
		{"strip-prefix", map[string]string{"INPUT_FERRY_PREVIEW_PORT": "80", "INPUT_FERRY_PREVIEW_STRIP_PREFIX": "yes"}},
		{"hold", map[string]string{"INPUT_FERRY_PREVIEW_PORT": "80", "INPUT_FERRY_PREVIEW_HOLD": "manual"}},
	} {
		if _, err := previewInputs(inputs(tc.values)); err == nil || !strings.Contains(err.Error(), tc.name) {
			t.Errorf("%v: err=%v", tc.values, err)
		}
	}
	for _, port := range []string{"", "0", "65536", "-1", "80a"} {
		if _, err := previewInputs(inputs(map[string]string{"INPUT_FERRY_PREVIEW_PORT": port})); err == nil || !strings.Contains(err.Error(), "port") {
			t.Errorf("port %q accepted", port)
		}
	}
}

func shortPreviewPolling(t *testing.T, grace time.Duration) {
	t.Helper()
	poll, previous := previewPoll, previewGrace
	previewPoll, previewGrace = 10*time.Millisecond, grace
	t.Cleanup(func() { previewPoll, previewGrace = poll, previous })
}

func writePreviewLater(t *testing.T, path string, answers ...string) {
	t.Helper()
	done := make(chan struct{})
	// Registered after t.TempDir, so it runs before the directory is removed.
	t.Cleanup(func() { <-done })
	go func() {
		defer close(done)
		for _, answer := range answers {
			time.Sleep(50 * time.Millisecond)
			_ = os.WriteFile(path, []byte(answer), 0600)
		}
	}()
}

func TestAwaitPreviewOutcomes(t *testing.T) {
	shortPreviewPolling(t, 2*time.Second)
	for _, tc := range []struct {
		name, hold string
		answers    []string
		failure    string
		decision   bool
	}{
		{"url", "none", []string{`{"url":"/holes/abc/"}`}, "", false},
		// A truncated write reads as "not yet", never as an answer.
		{"partial-then-url", "none", []string{`{"url":"/ho`, `{"url":"/holes/abc/"}`}, "", false},
		{"error", "none", []string{`{"error":"The local service did not become reachable in time"}`}, "did not become reachable", false},
		{"approved", "approval", []string{`{"url":"/holes/abc/"}`, `{"url":"/holes/abc/","decision":"approved"}`}, "", true},
		{"rejected", "approval", []string{`{"url":"/holes/abc/"}`, `{"url":"/holes/abc/","decision":"rejected"}`}, "rejected", true},
		{"timed-out-review", "approval", []string{`{"url":"/holes/abc/"}`, `{"url":"/holes/abc/","decision":"timed_out"}`}, "timed_out", true},
	} {
		t.Run(tc.name, func(t *testing.T) {
			dir := t.TempDir()
			outputFile := filepath.Join(dir, "output")
			if err := os.WriteFile(outputFile, nil, 0600); err != nil {
				t.Fatal(err)
			}
			request := previewRequest{ID: "preview-1", Port: 80, Hold: tc.hold, ReadyTimeoutSeconds: 1, TimeoutMinutes: 1, Response: filepath.ToSlash(dir) + "/preview-1.json"}
			writePreviewLater(t, filepath.Join(dir, "preview-1.json"), tc.answers...)
			var log bytes.Buffer
			err := awaitPreview(&log, "nonce", request, outputFile)
			if (tc.failure == "") != (err == nil) || (err != nil && !strings.Contains(err.Error(), tc.failure)) {
				t.Fatalf("err=%v log=%s", err, log.String())
			}
			marker := strings.Split(log.String(), "\n")[1]
			var sent previewRequest
			if !strings.HasPrefix(marker, "::ferry-preview::nonce::") || json.Unmarshal([]byte(strings.TrimPrefix(marker, "::ferry-preview::nonce::")), &sent) != nil || sent != request {
				t.Fatalf("marker=%q", marker)
			}
			output, _ := os.ReadFile(outputFile)
			if tc.name != "error" && (string(output) != "url=/holes/abc/\n" || !strings.Contains(log.String(), "Preview: /holes/abc/")) {
				t.Fatalf("output=%q log=%s", output, log.String())
			}
			if tc.decision != strings.Contains(log.String(), "Review: ") {
				t.Fatalf("log=%s", log.String())
			}
		})
	}
}

func TestAwaitPreviewTimesOutWithoutAnswer(t *testing.T) {
	shortPreviewPolling(t, 100*time.Millisecond)
	dir := t.TempDir()
	request := previewRequest{ID: "preview-2", Port: 80, Hold: "none", ReadyTimeoutSeconds: 1, TimeoutMinutes: 1, Response: filepath.ToSlash(dir) + "/preview-2.json"}
	started := time.Now()
	err := awaitPreview(&bytes.Buffer{}, "nonce", request, "")
	if err == nil || !strings.Contains(err.Error(), "no answer") {
		t.Fatalf("err=%v", err)
	}
	// Ready timeout (1 s) plus the grace (0.1 s), not more.
	if elapsed := time.Since(started); elapsed < time.Second || elapsed > 5*time.Second {
		t.Fatalf("elapsed=%s", elapsed)
	}
	// A URL without the awaited decision also ends at the review deadline.
	previewGrace = -59*time.Second - 900*time.Millisecond
	request.Hold = "approval"
	if err := os.WriteFile(filepath.FromSlash(request.Response), []byte(`{"url":"/holes/x/"}`), 0600); err != nil {
		t.Fatal(err)
	}
	if err := awaitPreview(&bytes.Buffer{}, "nonce", request, ""); err == nil || !strings.Contains(err.Error(), "no answer") {
		t.Fatalf("review err=%v", err)
	}
}

// previewService plays the server: it answers each marker that carries its
// nonce by writing the response file the request names.
type previewService struct {
	mutex    sync.Mutex
	buffer   bytes.Buffer
	pending  []byte
	nonce    string
	decision string
	requests []previewRequest
}

func (p *previewService) Write(data []byte) (int, error) {
	p.mutex.Lock()
	defer p.mutex.Unlock()
	p.buffer.Write(data)
	p.pending = append(p.pending, data...)
	for {
		end := bytes.IndexByte(p.pending, '\n')
		if end < 0 {
			return len(data), nil
		}
		line := p.pending[:end]
		p.pending = p.pending[end+1:]
		var record map[string]any
		if json.Unmarshal(line, &record) != nil {
			continue
		}
		text, _ := record["raw"].(string)
		start := strings.Index(text, previewMarker+p.nonce+"::")
		if start < 0 {
			continue
		}
		var request previewRequest
		if err := json.Unmarshal([]byte(strings.TrimSpace(text[start+len(previewMarker+p.nonce+"::"):])), &request); err != nil {
			return 0, err
		}
		p.requests = append(p.requests, request)
		answer, _ := json.Marshal(map[string]string{"url": "/holes/" + request.ID + "/", "decision": p.decision})
		if err := os.WriteFile(filepath.FromSlash(request.Response), answer, 0600); err != nil {
			return 0, err
		}
	}
}

func TestPreviewStepRunsThroughTheWorker(t *testing.T) {
	shortPreviewPolling(t, 5*time.Second)
	root := repository(t)
	path := artifactNodeFreePath(t)
	script := platformScript(`echo "seen=${{ steps.preview.outputs.url }}" >> "$GITHUB_OUTPUT"`, `echo seen=${{ steps.preview.outputs.url }}>> "%GITHUB_OUTPUT%"`)
	shell := platformScript("sh", "cmd")
	workflow := "name: preview\non: workflow_dispatch\njobs:\n  build:\n    runs-on: self-hosted\n    outputs:\n      url: ${{ steps.after.outputs.seen }}\n    steps:\n      - id: preview\n        name: Frontend\n        uses: ferry/preview@v1\n        with:\n          port: 5173\n          hold: approval\n      - id: after\n        shell: " + shell + "\n        run: " + script + "\n"
	for _, decision := range []string{"approved", "rejected"} {
		t.Run(decision, func(t *testing.T) {
			dir := filepath.ToSlash(t.TempDir()) + "/ferry-preview"
			service := &previewService{nonce: "nonce-" + decision, decision: decision}
			r := Request{Workflow: workflow, Job: "build", Workdir: root, RunID: "900", CacheDir: t.TempDir(), Platforms: map[string]string{"self-hosted": "-self-hosted"},
				Env: map[string]string{"PATH": path, "Path": path}, TimeoutSeconds: 60, PreviewNonce: service.nonce, PreviewDir: dir}
			result, err := execute(t.Context(), r, service)
			service.mutex.Lock()
			defer service.mutex.Unlock()
			logs := service.buffer.String()
			if len(service.requests) != 1 {
				t.Fatalf("requests=%+v logs=%s", service.requests, logs)
			}
			request := service.requests[0]
			if request.Port != 5173 || request.Hold != "approval" || request.Name != "Frontend" || request.Response != dir+"/"+request.ID+".json" || strings.Trim(request.ID, "abcdefghijklmnopqrstuvwxyz0123456789-") != "" {
				t.Fatalf("request=%+v", request)
			}
			if decision == "approved" {
				if err != nil || result.State != "success" || result.Outputs["url"] != "/holes/"+request.ID+"/" {
					t.Fatalf("result=%+v err=%v logs=%s", result, err, logs)
				}
			} else if err == nil || result.State != "failure" || result.Steps["after"] == "success" {
				t.Fatalf("rejected result=%+v err=%v logs=%s", result, err, logs)
			}
		})
	}
}
