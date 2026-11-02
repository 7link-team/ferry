package main

import (
	"bytes"
	"context"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"strings"
	"sync/atomic"
	"testing"

	"github.com/nektos/act/pkg/runner"
)

// A local authenticated Git HTTP fixture exercises the actual go-git fetch
// paths used by both remote action kinds, without public network access.
func TestRemoteActionsAndReusableWorkflowsAuthenticate(t *testing.T) {
	remote := repository(t)
	compatWrite(t, remote, "action.yml", "name: private fixture\ndescription: local authenticated git fixture\nruns:\n  using: node20\n  main: main.cjs\n")
	compatWrite(t, remote, "main.cjs", `require('fs').appendFileSync(process.env.GITHUB_OUTPUT,'value=remote-action-ok\n');`)
	compatWrite(t, remote, ".github/workflows/called.yml", `name: called
on:
  workflow_call:
    outputs:
      value:
        value: ${{ jobs.child.outputs.value }}
jobs:
  child:
    runs-on: self-hosted
    outputs:
      value: ${{ steps.probe.outputs.value }}
    steps:
      - id: probe
        run: |
`+platformScript("          echo 'value=remote-workflow-ok' >> \"$GITHUB_OUTPUT\"\n", "          'value=remote-workflow-ok' >> $env:GITHUB_OUTPUT\n"))
	for _, args := range [][]string{{"add", "."}, {"-c", "user.name=fixture", "-c", "user.email=fixture@example.invalid", "commit", "-qm", "remote fixtures"}, {"branch", "-M", "main"}, {"update-server-info"}} {
		command := testCommandContext(context.Background(), "git", append([]string{"-C", remote}, args...)...)
		if out, err := command.CombinedOutput(); err != nil {
			t.Fatalf("git fixture: %v %s", err, out)
		}
	}
	const token = "local-fixture-git-token"
	var authenticated atomic.Int64
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, password, ok := r.BasicAuth()
		if !ok || password != token {
			w.Header().Set("WWW-Authenticate", `Basic realm="fixture"`)
			w.WriteHeader(http.StatusUnauthorized)
			return
		}
		authenticated.Add(1)
		if !strings.HasPrefix(r.URL.Path, "/owner/repo/") {
			http.NotFound(w, r)
			return
		}
		args := []string{"upload-pack", "--stateless-rpc"}
		advertise := strings.HasSuffix(r.URL.Path, "/info/refs")
		if advertise {
			args = append(args, "--advertise-refs")
		}
		args = append(args, filepath.Join(remote, ".git"))
		command := testCommandContext(r.Context(), "git", args...)
		if !advertise {
			command.Stdin = r.Body
		}
		data, err := command.Output()
		if err != nil {
			http.Error(w, "git fixture failed", 500)
			return
		}
		if advertise {
			w.Header().Set("Content-Type", "application/x-git-upload-pack-advertisement")
			_, _ = w.Write([]byte("001e# service=git-upload-pack\n0000"))
		} else {
			w.Header().Set("Content-Type", "application/x-git-upload-pack-result")
		}
		_, _ = w.Write(data)
	}))
	defer server.Close()
	for _, kind := range []string{"action", "workflow"} {
		t.Run(kind, func(t *testing.T) {
			workflow := "name: remote\non: workflow_dispatch\njobs:\n  build:\n"
			want := "remote-workflow-ok"
			if kind == "workflow" {
				workflow += "    uses: owner/repo/.github/workflows/called.yml@main\n"
			} else {
				want = "remote-action-ok"
				workflow += "    runs-on: self-hosted\n    outputs:\n      value: ${{ steps.probe.outputs.value }}\n    steps:\n      - id: probe\n        uses: owner/repo@main\n"
			}
			// Actions come from the action source even when the repository
			// lives on another server (e.g. Codeup), whose URL is ServerURL.
			previous := runner.FerryActionsURL
			runner.FerryActionsURL = server.URL
			defer func() { runner.FerryActionsURL = previous }()
			r := Request{Workflow: workflow, Job: "build", Workdir: repository(t), CacheDir: t.TempDir(), ServerURL: "https://codeup.example.invalid", Secrets: map[string]string{"GITHUB_TOKEN": token}, Platforms: map[string]string{"self-hosted": "-self-hosted"}, TimeoutSeconds: 20}
			var logs bytes.Buffer
			before := authenticated.Load()
			result, err := execute(context.Background(), r, &logs)
			if err != nil || result.State != "success" || result.Outputs["value"] != want || authenticated.Load() == before {
				t.Fatalf("remote %s=%+v err=%v authenticated=%d logs=%s", kind, result, err, authenticated.Load()-before, logs.String())
			}
			if strings.Contains(logs.String(), token) {
				t.Fatal("remote token leaked")
			}
		})
	}
}
