package main

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
)

func TestRewriteBuiltinNotification(t *testing.T) {
	step := previewStep(t, "ferry/notify@v1", map[string]string{"channel-id": "12", "body": `${{ steps.build.outputs.report }}`, "variable-name": "$(touch unsafe)"})
	if err := rewriteBuiltinNotificationStep(step, "nonce", "/private/bridge"); err != nil {
		t.Fatal(err)
	}
	if step.Uses != "" || strings.Contains(step.Run, "touch") || !strings.Contains(step.Run, "notify") {
		t.Fatalf("step: %+v", step)
	}
	env := step.Environment()
	if env["KEEP"] != "kept" || env["FERRY_NOTIFICATION_NONCE"] != "nonce" || env["FERRY_NOTIFICATION_INPUT_COUNT"] != "3" {
		t.Fatalf("env: %+v", env)
	}
	if env["INPUT_FERRY_NOTIFICATION_0"] != `${{ steps.build.outputs.report }}` {
		t.Fatalf("expressions must be evaluated as environment values: %+v", env)
	}
	for _, input := range []string{`{}`, `{"channel-id":"12"}`, `{"channel-id":"12","body":"ok","format":"xml"}`, `{"channel-id":"12","body":"ok","typo":"oops"}`} {
		if _, err := notificationInputs(input); err == nil {
			t.Fatalf("invalid input accepted: %s", input)
		}
	}
	request, err := notificationInputs(`{"channel-id":"12","template-id":"34","to":"one@example.invalid;two@example.invalid","variable-name":"Quoted \"value\""}`)
	if err != nil || len(request.Message.To) != 2 || request.Message.Variables["name"] != `Quoted "value"` {
		t.Fatalf("request=%+v error=%v", request, err)
	}
}

type notificationService struct {
	mutex          sync.Mutex
	buffer         bytes.Buffer
	requests       []notificationRequest
	nonce, failure string
}

func (s *notificationService) Write(data []byte) (int, error) {
	s.mutex.Lock()
	defer s.mutex.Unlock()
	s.buffer.Write(data)
	// Worker JSON log records arrive as complete lines.
	for _, line := range bytes.Split(data, []byte("\n")) {
		var entry map[string]any
		if json.Unmarshal(line, &entry) != nil {
			continue
		}
		text, _ := entry["msg"].(string)
		if raw, ok := entry["raw"].(string); ok {
			text = raw
		}
		prefix := "::ferry-notify::" + s.nonce + "::"
		if !strings.HasPrefix(text, prefix) {
			continue
		}
		var request notificationRequest
		if err := json.Unmarshal([]byte(strings.TrimSpace(strings.TrimPrefix(text, prefix))), &request); err != nil {
			return 0, err
		}
		s.requests = append(s.requests, request)
		answer, _ := json.Marshal(notificationResponse{DeliveryID: "456", State: "succeeded", Error: s.failure})
		if err := os.WriteFile(filepath.FromSlash(request.Response), answer, 0600); err != nil {
			return 0, err
		}
	}
	return len(data), nil
}

func TestNotificationStepRunsThroughWorker(t *testing.T) {
	root := repository(t)
	path := artifactNodeFreePath(t)
	workflow := "name: notification\non:\n  workflow_dispatch:\n    inputs:\n      report:\n        type: string\njobs:\n  build:\n    runs-on: self-hosted\n    steps:\n      - id: notify\n        uses: ferry/notify@v1\n        with:\n          channel-id: '12'\n          body: ${{ inputs.report }}\n"
	for _, failure := range []string{"", "permission denied"} {
		t.Run("failure="+failure, func(t *testing.T) {
			service := &notificationService{nonce: "fixture-nonce", failure: failure}
			r := Request{Workflow: workflow, Job: "build", EventName: "workflow_dispatch", Workdir: root, RunID: "900", CacheDir: t.TempDir(), Platforms: map[string]string{"self-hosted": "-self-hosted"}, Env: map[string]string{"PATH": path, "Path": path}, Inputs: map[string]string{"report": "Quoted \"report\"\nSecond line"}, TimeoutSeconds: 60, PreviewNonce: service.nonce, PreviewDir: filepath.ToSlash(t.TempDir())}
			result, err := execute(t.Context(), r, service)
			service.mutex.Lock()
			defer service.mutex.Unlock()
			if len(service.requests) != 1 {
				t.Fatalf("requests=%+v result=%+v err=%v logs=%s", service.requests, result, err, service.buffer.String())
			}
			if service.requests[0].Message.Body != r.Inputs["report"] {
				t.Fatalf("expression input changed: %+v", service.requests[0])
			}
			if failure == "" && (err != nil || result.State != "success") {
				t.Fatalf("result=%+v err=%v logs=%s", result, err, service.buffer.String())
			}
			if failure != "" && (err == nil || result.State != "failure") {
				t.Fatalf("failed delivery must fail step: %+v err=%v", result, err)
			}
		})
	}
}
