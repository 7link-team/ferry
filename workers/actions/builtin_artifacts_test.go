package main

import (
	"archive/zip"
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
)

// The built-in action invokes os.Executable, including when execute is exercised
// by the test binary. Use precisely the production helper entry point.
func TestMain(m *testing.M) {
	if (len(os.Args) == 3 && os.Args[1] == "artifact") || (len(os.Args) == 2 && (os.Args[1] == "preview" || os.Args[1] == "notify")) {
		main()
		os.Exit(0)
	}
	os.Exit(m.Run())
}

func artifactNodeFreePath(t *testing.T) string {
	t.Helper()
	var path string
	if runtime.GOOS == "windows" {
		system := filepath.Join(os.Getenv("SystemRoot"), "System32")
		path = system + ";" + filepath.Join(system, "WindowsPowerShell", "v1.0")
	} else {
		path = t.TempDir()
		sh, err := exec.LookPath("sh")
		if err != nil {
			t.Fatal(err)
		}
		if err = os.Symlink(sh, filepath.Join(path, "sh")); err != nil {
			t.Fatal(err)
		}
	}
	t.Setenv("PATH", path)
	if _, err := exec.LookPath("node"); err == nil {
		t.Fatal("Node remains available in test PATH")
	}
	return path
}

func artifactFixtureFile(t *testing.T, root, name, value string, mode os.FileMode) {
	t.Helper()
	p := filepath.Join(root, filepath.FromSlash(name))
	if err := os.MkdirAll(filepath.Dir(p), 0700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(p, []byte(value), mode); err != nil {
		t.Fatal(err)
	}
}

func artifactExecute(t *testing.T, r Request) (JobResult, string, error) {
	t.Helper()
	var logs bytes.Buffer
	result, err := execute(context.Background(), r, &logs)
	return result, logs.String(), err
}

func TestBuiltinArtifactsRoundTripWithoutNode(t *testing.T) {
	root, second := repository(t), repository(t)
	source, destination, store := t.TempDir(), t.TempDir(), t.TempDir()
	for name, value := range map[string]string{
		"payload/readme.txt": "readme", "payload/nested/deep.txt": "nested",
		"payload/nested/exclude.txt": "excluded", "payload/.hidden": "hidden",
		"payload/.private/key.txt": "private", "lone.txt": "single",
		"old/change.txt": "old", "new/change.txt": "replacement",
	} {
		artifactFixtureFile(t, source, name, value, 0600)
	}
	artifactFixtureFile(t, source, "payload/tool", "#!/bin/sh\nexit 0\n", 0755)
	path := artifactNodeFreePath(t)
	workflow := `name: built-in artifacts
on: workflow_dispatch
jobs:
  upload:
    runs-on: self-hosted
    outputs:
      id: ${{ steps.directory.outputs.artifact-id }}
      digest: ${{ steps.directory.outputs.artifact-digest }}
      later: ${{ steps.later.outputs.value }}
    steps:
      - id: skipped
        if: ${{ false }}
        uses: actions/upload-artifact@v4
        with:
          path: impossible-path
          if-no-files-found: error
      - id: directory
        name: Directory without Node
        uses: actions/upload-artifact@v4
        timeout-minutes: 1
        env:
          SELECT_PATH: payload
        working-directory: ${{ env.SOURCE }}
        with:
          name: directory
          path: ${{ env.SELECT_PATH }}
          retention-days: 7
          compression-level: 0
      - id: later
        shell: sh
        run: echo 'value=${{ steps.directory.outputs.artifact-id }}/${{ steps.directory.outputs.artifact-digest }}' >> "$GITHUB_OUTPUT"
      - uses: actions/upload-artifact@v4.6.2
        with:
          name: wildcard
          path: |
            ${{ env.SOURCE }}/payload/**/*.txt
            !${{ env.SOURCE }}/payload/**/exclude.txt
      - uses: actions/upload-artifact@v4
        with:
          name: hidden
          path: ${{ env.SOURCE }}/payload
          include-hidden-files: true
          compression-level: 9
      - uses: actions/upload-artifact@v4
        with:
          name: explicit-hidden
          path: ${{ env.SOURCE }}/payload/.hidden
      - uses: actions/upload-artifact@v4
        with:
          name: multi
          path: |
            ${{ env.SOURCE }}/payload/readme.txt
            ${{ env.SOURCE }}/new/change.txt
      - uses: actions/upload-artifact@v4
        with:
          name: single
          path: ${{ env.SOURCE }}/lone.txt
      - uses: actions/upload-artifact@v4
        with:
          name: replaced
          path: ${{ env.SOURCE }}/old/change.txt
      - uses: actions/upload-artifact@v4
        with:
          name: replaced
          path: ${{ env.SOURCE }}/new/change.txt
          overwrite: true
          retention-days: 9
  download:
    needs: upload
    runs-on: self-hosted
    outputs:
      downloaded: ${{ steps.all.outputs.download-path }}
      merged: ${{ steps.merge.outputs.download-path }}
      later: ${{ steps.later.outputs.value }}
    steps:
      - id: all
        uses: actions/download-artifact@v4
        with:
          path: ${{ env.DESTINATION }}/all
      - id: merge
        uses: actions/download-artifact@v4.3.0
        with:
          path: ${{ env.DESTINATION }}/merged
          merge-multiple: true
      - uses: actions/download-artifact@v4
        with:
          pattern: wild*
          path: ${{ env.DESTINATION }}/pattern
      - uses: actions/download-artifact@v4
        with:
          name: single
          path: ${{ env.DESTINATION }}/named
      - id: later
        shell: sh
        run: echo 'value=${{ steps.all.outputs.download-path }}' >> "$GITHUB_OUTPUT"
`
	if runtime.GOOS == "windows" {
		workflow = strings.ReplaceAll(workflow, "shell: sh", "shell: cmd")
		workflow = strings.ReplaceAll(workflow, `echo 'value=${{ steps.directory.outputs.artifact-id }}/${{ steps.directory.outputs.artifact-digest }}' >> "$GITHUB_OUTPUT"`, `echo value=${{ steps.directory.outputs.artifact-id }}/${{ steps.directory.outputs.artifact-digest }}>> "%GITHUB_OUTPUT%"`)
		workflow = strings.ReplaceAll(workflow, `echo 'value=${{ steps.all.outputs.download-path }}' >> "$GITHUB_OUTPUT"`, `echo value=${{ steps.all.outputs.download-path }}>> "%GITHUB_OUTPUT%"`)
	}
	r := Request{Workflow: workflow, Job: "upload", Workdir: root, ArtifactDir: store, RunID: "671", CacheDir: t.TempDir(), Platforms: map[string]string{"self-hosted": "-self-hosted"}, Env: map[string]string{"SOURCE": filepath.ToSlash(source), "DESTINATION": filepath.ToSlash(destination), "PATH": path, "Path": path}, TimeoutSeconds: 60}
	uploaded, logs, err := artifactExecute(t, r)
	if err != nil || uploaded.State != "success" {
		t.Fatalf("upload=%+v error=%v logs=%s", uploaded, err, logs)
	}
	if uploaded.Steps["skipped"] != "skipped" {
		t.Fatalf("if lost: %+v", uploaded.Steps)
	}
	if len(uploaded.Artifacts) != 7 {
		t.Fatalf("artifacts=%v", uploaded.Artifacts)
	}
	if uploaded.ArtifactRetention["671/directory/directory.zip"] != 7 || uploaded.ArtifactRetention["671/replaced/replaced.zip"] != 9 {
		t.Fatalf("retention=%v", uploaded.ArtifactRetention)
	}
	data, err := os.ReadFile(filepath.Join(store, "671", "directory", "directory.zip"))
	if err != nil {
		t.Fatal(err)
	}
	digest := sha256.Sum256(data)
	wantDigest := hex.EncodeToString(digest[:])
	if uploaded.Outputs["id"] == "" || uploaded.Outputs["digest"] != wantDigest || uploaded.Outputs["later"] != uploaded.Outputs["id"]+"/"+wantDigest {
		t.Fatalf("outputs=%v zip digest=%s", uploaded.Outputs, wantDigest)
	}
	archive, err := zip.OpenReader(filepath.Join(store, "671", "directory", "directory.zip"))
	if err != nil {
		t.Fatal(err)
	}
	for _, file := range archive.File {
		if strings.HasPrefix(file.Name, "payload/") || strings.HasPrefix(file.Name, "/") {
			t.Errorf("incorrect archive root %q", file.Name)
		}
	}
	archive.Close()
	r.Job, r.Workdir = "download", second
	r.Needs = map[string]JobResult{"upload": uploaded}
	downloaded, logs, err := artifactExecute(t, r)
	if err != nil || downloaded.State != "success" {
		t.Fatalf("download=%+v error=%v logs=%s", downloaded, err, logs)
	}
	if filepath.Clean(downloaded.Outputs["downloaded"]) != filepath.Join(destination, "all") || filepath.Clean(downloaded.Outputs["merged"]) != filepath.Join(destination, "merged") || downloaded.Outputs["later"] != downloaded.Outputs["downloaded"] {
		t.Fatalf("download outputs=%v", downloaded.Outputs)
	}
	for name, want := range map[string]string{
		"all/directory/readme.txt": "readme", "all/directory/nested/deep.txt": "nested",
		"all/wildcard/readme.txt": "readme", "all/wildcard/nested/deep.txt": "nested",
		"all/hidden/.hidden": "hidden", "all/hidden/.private/key.txt": "private",
		"all/explicit-hidden/.hidden": "hidden", "all/multi/payload/readme.txt": "readme", "all/multi/new/change.txt": "replacement", "all/single/lone.txt": "single",
		"all/replaced/change.txt": "replacement", "merged/change.txt": "replacement",
		"merged/lone.txt": "single", "named/lone.txt": "single",
		"pattern/wildcard/nested/deep.txt": "nested",
	} {
		b, e := os.ReadFile(filepath.Join(destination, filepath.FromSlash(name)))
		if e != nil || string(b) != want {
			t.Errorf("%s=%q want=%q error=%v", name, b, want, e)
		}
	}
	for _, name := range []string{"all/directory/.hidden", "all/directory/.private/key.txt", "all/wildcard/nested/exclude.txt", "all/wildcard/.private/key.txt", "pattern/single/lone.txt", "merged/directory/readme.txt"} {
		if _, e := os.Stat(filepath.Join(destination, filepath.FromSlash(name))); !os.IsNotExist(e) {
			t.Errorf("unexpected %s error=%v", name, e)
		}
	}
	if runtime.GOOS != "windows" {
		info, err := os.Stat(filepath.Join(destination, "all", "directory", "tool"))
		if err != nil || info.Mode().Perm()&0111 != 0111 {
			t.Errorf("executable mode lost: %v %v", info, err)
		}
	}
}

func TestBuiltinArtifactInputFailuresAndTolerance(t *testing.T) {
	root := repository(t)
	source := t.TempDir()
	artifactFixtureFile(t, source, "file.txt", "proof", 0600)
	path := artifactNodeFreePath(t)
	for _, tc := range []struct{ name, action, inputs, state, diagnostic string }{
		{"missing-error", "upload", "path: missing-file\n          if-no-files-found: error", "failure", "No files"},
		{"missing-warn", "upload", "path: missing-file", "success", "No files found"},
		{"missing-ignore", "upload", "path: missing-file\n          if-no-files-found: ignore", "success", ""},
		{"bad-compression", "upload", "path: ${{ env.SOURCE }}/file.txt\n          compression-level: 10", "failure", "compression-level"},
		{"other-token", "download", "github-token: ${{ secrets.OTHER_TOKEN }}", "failure", "github-token"},
		{"other-empty-token", "download", "github-token: ''", "failure", "github-token"},
		{"other-repository", "download", "repository: owner/other", "failure", "repository"},
		{"other-run", "download", "run-id: 98", "failure", "run-id"},
		{"missing-artifact", "download", "name: missing-artifact", "failure", ""},
	} {
		t.Run(tc.name, func(t *testing.T) {
			workflow := "name: input checks\non: workflow_dispatch\njobs:\n  build:\n    runs-on: self-hosted\n    steps:\n      - id: probe\n        uses: actions/" + tc.action + "-artifact@v4\n        with:\n          " + tc.inputs + "\n"
			r := Request{Workflow: workflow, Job: "build", Workdir: root, ArtifactDir: t.TempDir(), RunID: "672", CacheDir: t.TempDir(), Platforms: map[string]string{"self-hosted": "-self-hosted"}, Env: map[string]string{"SOURCE": filepath.ToSlash(source), "PATH": path, "Path": path}, Secrets: map[string]string{"OTHER_TOKEN": "do-not-print-this-token"}, TimeoutSeconds: 30}
			result, logs, err := artifactExecute(t, r)
			if result.State != tc.state || (tc.state == "failure") != (err != nil) {
				t.Fatalf("result=%+v error=%v logs=%s", result, err, logs)
			}
			if tc.diagnostic != "" && !strings.Contains(strings.ToLower(logs), strings.ToLower(tc.diagnostic)) {
				t.Errorf("missing diagnostic %q: %s", tc.diagnostic, logs)
			}
			if strings.Contains(logs, "do-not-print-this-token") {
				t.Fatal("secret leaked")
			}
			if strings.HasPrefix(tc.name, "missing-") && tc.action == "upload" && len(result.Artifacts) != 0 {
				t.Fatalf("missing files created artifacts %v", result.Artifacts)
			}
		})
	}
	// An unsuccessful built-in action remains a real step: tolerance permits later
	// success(), while outcome/conclusion retain normal act semantics.
	script := platformScript(`echo 'value=${{ steps.probe.outcome }}/${{ steps.probe.conclusion }}' >> "$GITHUB_OUTPUT"`, `echo value=${{ steps.probe.outcome }}/${{ steps.probe.conclusion }}>> "%GITHUB_OUTPUT%"`)
	shell := platformScript("sh", "cmd")
	workflow := fmt.Sprintf("name: tolerated artifact\non: workflow_dispatch\njobs:\n  build:\n    runs-on: self-hosted\n    outputs:\n      result: ${{ steps.after.outputs.value }}\n    steps:\n      - id: probe\n        continue-on-error: true\n        uses: actions/upload-artifact@v4\n        with:\n          path: missing-file\n          if-no-files-found: error\n      - id: after\n        if: success()\n        shell: %s\n        run: %s\n", shell, script)
	r := Request{Workflow: workflow, Job: "build", Workdir: root, ArtifactDir: t.TempDir(), RunID: "673", CacheDir: t.TempDir(), Platforms: map[string]string{"self-hosted": "-self-hosted"}, Env: map[string]string{"PATH": path, "Path": path}, TimeoutSeconds: 30}
	result, logs, err := artifactExecute(t, r)
	if err != nil || result.State != "success" || result.Outputs["result"] != "failure/success" {
		t.Fatalf("tolerated=%+v error=%v logs=%s", result, err, logs)
	}
}

func TestBuiltinArtifactDefaultsAndTilde(t *testing.T) {
	root := repository(t)
	source, home, store := t.TempDir(), t.TempDir(), t.TempDir()
	artifactFixtureFile(t, source, "literal ' & file.txt", "literal input", 0600)
	path := artifactNodeFreePath(t)
	workflow := `name: defaults
on: workflow_dispatch
jobs:
  build:
    runs-on: self-hosted
    defaults:
      run:
        working-directory: nonexistent-run-default
    outputs:
      workspace: ${{ github.workspace }}
      default-path: ${{ steps.default.outputs.download-path }}
      home-path: ${{ steps.home.outputs.download-path }}
    steps:
      - uses: actions/upload-artifact@v4
        with:
          path: ${{ env.SOURCE }}/literal ' & file.txt
      - id: default
        uses: actions/download-artifact@v4
        with:
          name: artifact
      - id: home
        uses: actions/download-artifact@v4
        with:
          name: artifact
          path: ~/downloaded
`
	r := Request{Workflow: workflow, Job: "build", Workdir: root, ArtifactDir: store, RunID: "674", CacheDir: t.TempDir(), Platforms: map[string]string{"self-hosted": "-self-hosted"}, Env: map[string]string{"SOURCE": filepath.ToSlash(source), "HOME": home, "USERPROFILE": home, "PATH": path, "Path": path}, TimeoutSeconds: 30}
	result, logs, err := artifactExecute(t, r)
	if err != nil || result.State != "success" {
		t.Fatalf("defaults=%+v error=%v logs=%s", result, err, logs)
	}
	if len(result.Artifacts) != 1 || result.Artifacts[0] != "674/artifact/artifact.zip" {
		t.Fatalf("default artifact name=%v", result.Artifacts)
	}
	if result.Outputs["workspace"] == "" || filepath.Clean(result.Outputs["default-path"]) != filepath.Clean(result.Outputs["workspace"]) || filepath.Clean(result.Outputs["home-path"]) != filepath.Join(home, "downloaded") {
		t.Fatalf("default paths=%v", result.Outputs)
	}
	value, err := os.ReadFile(filepath.Join(home, "downloaded", "literal ' & file.txt"))
	if err != nil || string(value) != "literal input" {
		t.Fatalf("literal path contents=%q error=%v", value, err)
	}
}

func TestBuiltinArtifactRejectsUnsafeArchive(t *testing.T) {
	root, store, destination := repository(t), t.TempDir(), t.TempDir()
	archivePath := filepath.Join(store, "675", "unsafe", "unsafe.zip")
	if err := os.MkdirAll(filepath.Dir(archivePath), 0700); err != nil {
		t.Fatal(err)
	}
	f, err := os.Create(archivePath)
	if err != nil {
		t.Fatal(err)
	}
	archive := zip.NewWriter(f)
	entry, err := archive.Create("../escaped.txt")
	if err != nil {
		t.Fatal(err)
	}
	if _, err = entry.Write([]byte("must not extract")); err != nil {
		t.Fatal(err)
	}
	if err = archive.Close(); err != nil {
		t.Fatal(err)
	}
	if err = f.Close(); err != nil {
		t.Fatal(err)
	}
	path := artifactNodeFreePath(t)
	workflow := `name: unsafe archive
on: workflow_dispatch
jobs:
  build:
    runs-on: self-hosted
    steps:
      - uses: actions/download-artifact@v4
        with:
          name: unsafe
          path: ${{ env.DESTINATION }}
`
	r := Request{Workflow: workflow, Job: "build", Workdir: root, ArtifactDir: store, RunID: "675", CacheDir: t.TempDir(), Platforms: map[string]string{"self-hosted": "-self-hosted"}, Env: map[string]string{"DESTINATION": filepath.ToSlash(destination), "PATH": path, "Path": path}, TimeoutSeconds: 30}
	result, logs, err := artifactExecute(t, r)
	if err == nil || result.State != "failure" || !strings.Contains(logs, "unsafe path") {
		t.Fatalf("unsafe archive=%+v error=%v logs=%s", result, err, logs)
	}
	if _, err := os.Stat(filepath.Join(filepath.Dir(destination), "escaped.txt")); !os.IsNotExist(err) {
		t.Fatalf("ZIP path escaped destination: %v", err)
	}
}

func TestBuiltinArtifactsInsideCompositeAndReusableWorkflow(t *testing.T) {
	root, source, destination, store := repository(t), t.TempDir(), t.TempDir(), t.TempDir()
	artifactFixtureFile(t, source, "proof.txt", "nested built-in action", 0600)
	artifactFixtureFile(t, root, "local/action.yml", `name: composite artifact
description: artifact operation inside a composite
outputs:
  digest:
    description: uploaded digest
    value: ${{ steps.upload.outputs.artifact-digest }}
runs:
  using: composite
  steps:
    - id: upload
      uses: actions/upload-artifact@v4
      with:
        name: nested
        path: ${{ env.SOURCE }}/proof.txt
`, 0600)
	artifactFixtureFile(t, root, ".github/workflows/called.yml", `name: called artifact workflow
on:
  workflow_call:
    outputs:
      digest:
        value: ${{ jobs.child.outputs.digest }}
jobs:
  child:
    runs-on: self-hosted
    outputs:
      digest: ${{ steps.composite.outputs.digest }}
    steps:
      - uses: actions/checkout@v4
      - id: composite
        uses: ./local
      - uses: actions/download-artifact@v4
        with:
          name: nested
          path: ${{ env.DESTINATION }}
`, 0600)
	path := artifactNodeFreePath(t)
	r := Request{Workflow: "on: workflow_dispatch\njobs:\n  build:\n    uses: ./.github/workflows/called.yml\n", Job: "build", Workdir: root, ArtifactDir: store, RunID: "676", CacheDir: t.TempDir(), Platforms: map[string]string{"self-hosted": "-self-hosted"}, Env: map[string]string{"SOURCE": filepath.ToSlash(source), "DESTINATION": filepath.ToSlash(destination), "PATH": path, "Path": path}, TimeoutSeconds: 30}
	result, logs, err := artifactExecute(t, r)
	if err != nil || result.State != "success" || len(result.Outputs["digest"]) != 64 {
		t.Fatalf("nested built-ins=%+v error=%v logs=%s", result, err, logs)
	}
	value, err := os.ReadFile(filepath.Join(destination, "proof.txt"))
	if err != nil || string(value) != "nested built-in action" {
		t.Fatalf("nested download=%q error=%v", value, err)
	}
}
