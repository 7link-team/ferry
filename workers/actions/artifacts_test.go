package main

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
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

func TestArtifactOverwriteDeleteAndRetention(t *testing.T) {
	directory := t.TempDir()
	runtime, err := startArtifacts(context.Background(), directory, "41", false)
	if err != nil {
		t.Fatal(err)
	}
	defer runtime.Close()
	post := func(method, payload string, want int) map[string]any {
		t.Helper()
		req, err := http.NewRequest("POST", runtime.URL+"/twirp/github.actions.results.api.v1.ArtifactService/"+method, strings.NewReader(payload))
		if err != nil {
			t.Fatal(err)
		}
		req.Header.Set("Authorization", "Bearer "+runtime.Token)
		req.Header.Set("Content-Type", "application/json")
		response, err := http.DefaultClient.Do(req)
		if err != nil {
			t.Fatal(err)
		}
		defer response.Body.Close()
		data, err := io.ReadAll(response.Body)
		if err != nil {
			t.Fatal(err)
		}
		if response.StatusCode != want {
			t.Fatalf("%s: status=%d want=%d body=%s", method, response.StatusCode, want, data)
		}
		var value map[string]any
		_ = json.Unmarshal(data, &value)
		return value
	}
	for index, contents := range []string{"old archive", "replacement archive"} {
		if index == 1 {
			// upload-artifact overwrite:true uses DeleteArtifact before CreateArtifact.
			post("DeleteArtifact", `{"workflowRunBackendId":"41","workflowJobRunBackendId":"41","name":"proof"}`, 200)
			if _, err := os.Stat(filepath.Join(directory, "41", "proof", "proof.zip")); !os.IsNotExist(err) {
				t.Fatal("delete preserved old archive")
			}
		}
		payload := `{"workflowRunBackendId":"41","workflowJobRunBackendId":"41","name":"proof","version":4,"expiresAt":"` + time.Now().Add(time.Duration(7+index)*24*time.Hour).UTC().Format(time.RFC3339Nano) + `"}`
		reply := post("CreateArtifact", payload, 200)
		link, err := url.Parse(reply["signedUploadUrl"].(string))
		if err != nil {
			t.Fatal(err)
		}
		query := link.Query()
		query.Set("comp", "block")
		link.RawQuery = query.Encode()
		req, err := http.NewRequest("PUT", link.String(), strings.NewReader(contents))
		if err != nil {
			t.Fatal(err)
		}
		response, err := http.DefaultClient.Do(req)
		if err != nil {
			t.Fatal(err)
		}
		response.Body.Close()
		if response.StatusCode != 201 {
			t.Fatalf("upload status=%d", response.StatusCode)
		}
		data, err := os.ReadFile(filepath.Join(directory, "41", "proof", "proof.zip"))
		if err != nil || string(data) != contents {
			t.Fatalf("archive=%q err=%v", data, err)
		}
		if runtime.Retention()["41/proof/proof.zip"] != 7+index {
			t.Fatalf("retention=%v", runtime.Retention())
		}
		post("CreateArtifact", payload, http.StatusConflict)
	}
	post("DeleteArtifact", `{"workflowRunBackendId":"41","name":"../escape"}`, http.StatusBadRequest)
	post("DeleteArtifact", `{"workflowRunBackendId":"99","name":"proof"}`, http.StatusForbidden)
	post("DeleteArtifact", `{"workflowRunBackendId":"41","name":"proof"}`, 200)
	if len(runtime.Retention()) != 0 {
		t.Fatal("deleted artifact retained expiry metadata")
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
