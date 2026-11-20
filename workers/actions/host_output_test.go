//go:build !windows

package main

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"runtime"
	"strings"
	"testing"
	"time"
)

func TestFailedHostCommandDrainsAllOutputBeforeReturning(t *testing.T) {
	previous := runtime.GOMAXPROCS(1)
	defer runtime.GOMAXPROCS(previous)
	root := repository(t)
	for attempt := 0; attempt < 4; attempt++ {
		t.Run(fmt.Sprint(attempt), func(t *testing.T) {
			ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
			defer cancel()
			workflow := `name: failed output
on: workflow_dispatch
jobs:
  build:
    runs-on: self-hosted
    steps:
      - run: |
          i=0
          while [ "$i" -lt 1024 ]; do
            printf 'stderr-%s\n' "$i" >&2
            i=$((i+1))
          done
          printf 'out\n'
          printf 'err\n' >&2
          exit 7
`
			var logs bytes.Buffer
			result, err := execute(ctx, Request{Workflow: workflow, Job: "build", Workdir: root,
				Platforms: map[string]string{"self-hosted": "-self-hosted"}}, &logs)
			if err == nil || result.State != "failure" {
				t.Fatalf("exit 7 must remain a failure: %#v %v", result, err)
			}
			if !strings.Contains(logs.String(), "exit status 7") {
				t.Fatalf("command must execute and exit 7: %v; %s", err, logs.String())
			}
			decoder := json.NewDecoder(bytes.NewReader(logs.Bytes()))
			seen := map[string]bool{}
			for {
				var event map[string]any
				if err := decoder.Decode(&event); err == io.EOF {
					break
				} else if err != nil {
					t.Fatal(err)
				}
				if event["raw_output"] == true {
					message, _ := event["msg"].(string)
					seen[strings.TrimSpace(message)] = true
				}
			}
			for i := 0; i < 1024; i++ {
				if !seen[fmt.Sprintf("stderr-%d", i)] {
					t.Fatalf("stderr line %d was lost before the failed command returned (%v)", i, err)
				}
			}
			if !seen["out"] || !seen["err"] {
				t.Fatalf("final stdout/stderr missing: out=%v err=%v", seen["out"], seen["err"])
			}
		})
	}
}
