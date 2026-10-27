package main

import (
	"context"
	"io"
	"net/http"
	"strings"
	"testing"
)

func TestArtifactRuntimeRequiresCapabilityAndRejectsWrongRun(t *testing.T) {
	runtime, err := startArtifacts(context.Background(), t.TempDir(), "41", false)
	if err != nil {
		t.Fatal(err)
	}
	defer runtime.Close()
	path := "/twirp/github.actions.results.api.v1.ArtifactService/CreateArtifact"
	response, err := http.Post(runtime.URL+path, "application/json", strings.NewReader(`{"workflowRunBackendId":"41","workflowJobRunBackendId":"41","name":"proof","version":4}`))
	if err != nil {
		t.Fatal(err)
	}
	response.Body.Close()
	if response.StatusCode != http.StatusUnauthorized {
		t.Fatalf("unauthorized status %d", response.StatusCode)
	}
	req, _ := http.NewRequest("POST", runtime.URL+path, strings.NewReader(`{"workflowRunBackendId":"42","workflowJobRunBackendId":"42","name":"proof","version":4}`))
	req.Header.Set("Authorization", "Bearer "+runtime.Token)
	req.Header.Set("Content-Type", "application/json")
	response, err = http.DefaultClient.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	response.Body.Close()
	if response.StatusCode != http.StatusForbidden {
		t.Fatalf("wrong-run status %d", response.StatusCode)
	}
	req, _ = http.NewRequest("POST", runtime.URL+path, strings.NewReader(`{"workflowRunBackendId":"41","workflowJobRunBackendId":"41","name":"proof","version":4}`))
	req.Header.Set("Authorization", "Bearer "+runtime.Token)
	req.Header.Set("Content-Type", "application/json")
	response, err = http.DefaultClient.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	defer response.Body.Close()
	body, _ := io.ReadAll(response.Body)
	if response.StatusCode != 200 || !strings.Contains(string(body), "access_token=") {
		t.Fatalf("capability missing from upload: %d %s", response.StatusCode, body)
	}
}

func TestArtifactRuntimePreservesExistingArchive(t *testing.T) {
	runtime, err := startArtifacts(context.Background(), t.TempDir(), "41", false)
	if err != nil {
		t.Fatal(err)
	}
	defer runtime.Close()
	for index, want := range []int{http.StatusOK, http.StatusConflict} {
		req, _ := http.NewRequest("POST", runtime.URL+"/twirp/github.actions.results.api.v1.ArtifactService/CreateArtifact", strings.NewReader(`{"workflowRunBackendId":"41","workflowJobRunBackendId":"41","name":"proof","version":4}`))
		req.Header.Set("Authorization", "Bearer "+runtime.Token)
		req.Header.Set("Content-Type", "application/json")
		response, err := http.DefaultClient.Do(req)
		if err != nil {
			t.Fatal(err)
		}
		response.Body.Close()
		if response.StatusCode != want {
			t.Fatalf("create %d: got %d want %d", index, response.StatusCode, want)
		}
	}
}
