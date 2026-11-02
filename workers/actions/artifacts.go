package main

import (
	"context"
	"crypto/rand"
	"crypto/subtle"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"hash/fnv"
	"io"
	"io/fs"
	"maps"
	"math"
	"net"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"

	"github.com/golang-jwt/jwt/v5"
	"github.com/julienschmidt/httprouter"
	"github.com/nektos/act/pkg/artifacts"
)

// RoutesV4 supplies the compatibility protocol. This outer capability check is
// required: act's local CLI uses publicly known signing keys and is not an
// authenticated network service on its own.
type artifactRuntime struct {
	URL, Token string
	capability string
	server     *http.Server
	listener   net.Listener
	root       *os.Root
	mutex      sync.Mutex
	retention  map[string]int
}

func (r *artifactRuntime) Retention() map[string]int {
	r.mutex.Lock()
	defer r.mutex.Unlock()
	return maps.Clone(r.retention)
}

func (r *artifactRuntime) Close() {
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	if r.server.Shutdown(ctx) != nil {
		r.server.Close()
	}
	r.root.Close()
}

type artifactFS struct {
	base string
	root *os.Root
}

func (f artifactFS) relative(name string) (string, error) {
	rel, err := filepath.Rel(f.base, name)
	if err != nil || !fs.ValidPath(filepath.ToSlash(rel)) {
		return "", fmt.Errorf("invalid artifact path")
	}
	return rel, nil
}
func (f artifactFS) Open(name string) (fs.File, error) {
	rel, err := f.relative(name)
	if err != nil {
		return nil, err
	}
	return f.root.Open(rel)
}
func (f artifactFS) open(name string, flags int) (artifacts.WritableFile, error) {
	rel, err := f.relative(name)
	if err != nil {
		return nil, err
	}
	if err = f.root.MkdirAll(filepath.Dir(rel), 0700); err != nil {
		return nil, err
	}
	return f.root.OpenFile(rel, flags, 0600)
}
func (f artifactFS) OpenWritable(name string) (artifacts.WritableFile, error) {
	return f.open(name, os.O_CREATE|os.O_WRONLY|os.O_TRUNC)
}
func (f artifactFS) OpenAppendable(name string) (artifacts.WritableFile, error) {
	return f.open(name, os.O_CREATE|os.O_WRONLY|os.O_APPEND)
}

func startArtifacts(ctx context.Context, directory, run string, container bool) (*artifactRuntime, error) {
	if err := os.MkdirAll(filepath.Join(directory, run), 0700); err != nil {
		return nil, err
	}
	root, err := os.OpenRoot(directory)
	if err != nil {
		return nil, err
	}
	address := "127.0.0.1:0"
	if container {
		address = "0.0.0.0:0"
	}
	listener, err := net.Listen("tcp", address)
	if err != nil {
		root.Close()
		return nil, err
	}
	nonce := make([]byte, 32)
	if _, err = rand.Read(nonce); err != nil {
		listener.Close()
		root.Close()
		return nil, err
	}
	capability := hex.EncodeToString(nonce)
	claims := jwt.MapClaims{"scp": "Actions.Results:" + run + ":" + run, "TaskID": json.Number(run), "RunID": json.Number(run), "JobID": json.Number(run), "exp": time.Now().Add(24 * time.Hour).Unix(), "nonce": capability}
	token, err := jwt.NewWithClaims(jwt.SigningMethodHS256, claims).SignedString([]byte{})
	if err != nil {
		listener.Close()
		root.Close()
		return nil, err
	}
	runtime := &artifactRuntime{Token: token, capability: capability, listener: listener, root: root, retention: map[string]int{}}
	advertised := "127.0.0.1"
	if container {
		advertised = "host.docker.internal"
	}
	runtime.URL = fmt.Sprintf("http://%s:%d", advertised, listener.Addr().(*net.TCPAddr).Port)
	router := httprouter.New()
	filesystem := artifactFS{base: directory, root: root}
	artifacts.RoutesV4(router, directory, filesystem, filesystem)
	handler := http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method == "POST" {
			if subtle.ConstantTimeCompare([]byte(r.Header.Get("Authorization")), []byte("Bearer "+token)) != 1 {
				w.WriteHeader(http.StatusUnauthorized)
				return
			}
			body, err := io.ReadAll(http.MaxBytesReader(w, r.Body, 1024*1024))
			if err != nil {
				w.WriteHeader(http.StatusRequestEntityTooLarge)
				return
			}
			var request map[string]any
			if json.Unmarshal(body, &request) != nil {
				w.WriteHeader(http.StatusBadRequest)
				return
			}
			for _, key := range []string{"workflowRunBackendId", "workflow_run_backend_id", "workflowJobRunBackendId", "workflow_job_run_backend_id"} {
				if value, ok := request[key]; ok && fmt.Sprint(value) != run {
					w.WriteHeader(http.StatusForbidden)
					return
				}
			}
			runtime.mutex.Lock()
			defer runtime.mutex.Unlock()
			creating := strings.HasSuffix(r.URL.Path, "/CreateArtifact")
			deleting := strings.HasSuffix(r.URL.Path, "/DeleteArtifact")
			if creating || deleting {
				name, ok := request["name"].(string)
				if !ok || name == "" || name == "." || name == ".." || strings.ContainsAny(name, "/\\") {
					http.Error(w, "Invalid artifact name", http.StatusBadRequest)
					return
				}
				archive := filepath.Join(run, name, name+".zip")
				if deleting {
					// Use the confined root, unlike act's os.RemoveAll route.
					if err := root.Remove(archive); err != nil && !os.IsNotExist(err) {
						http.Error(w, "Cannot delete artifact", http.StatusInternalServerError)
						return
					}
					_ = root.Remove(filepath.Join(run, name))
					delete(runtime.retention, filepath.ToSlash(archive))
					w.Header().Set("Content-Type", "application/json")
					id := fnv.New32a()
					_, _ = id.Write([]byte(name))
					_, _ = fmt.Fprintf(w, `{"ok":true,"artifactId":"%d"}`, id.Sum32())
					return
				}
				if _, err := root.Stat(archive); err == nil {
					http.Error(w, "Artifact already exists; use a distinct name", http.StatusConflict)
					return
				}
				days := 0
				for _, key := range []string{"expiresAt", "expires_at"} {
					if raw, ok := request[key].(string); ok && raw != "" {
						expires, err := time.Parse(time.RFC3339Nano, raw)
						if err != nil {
							http.Error(w, "Invalid artifact expiry", http.StatusBadRequest)
							return
						}
						days = max(0, int(math.Round(time.Until(expires).Hours()/24)))
					}
				}
				runtime.retention[filepath.ToSlash(archive)] = days
			}
			r.Body = io.NopCloser(strings.NewReader(string(body)))
			// The returned URLs must refer to this listener, never an arbitrary Host.
			endpoint, _ := url.Parse(runtime.URL)
			r.Host = endpoint.Host
			recorder := httptest.NewRecorder()
			router.ServeHTTP(recorder, r)
			for key, values := range recorder.Header() {
				w.Header()[key] = values
			}
			var reply map[string]any
			data := recorder.Body.Bytes()
			if json.Unmarshal(data, &reply) == nil {
				for _, key := range []string{"signedUploadUrl", "signedUrl"} {
					if value, ok := reply[key].(string); ok {
						link, err := url.Parse(value)
						if err == nil {
							query := link.Query()
							query.Set("access_token", capability)
							link.RawQuery = query.Encode()
							reply[key] = link.String()
						}
					}
				}
				data, _ = json.Marshal(reply)
			}
			w.Header().Del("Content-Length")
			w.WriteHeader(recorder.Code)
			_, _ = w.Write(data)
			return
		}
		if subtle.ConstantTimeCompare([]byte(r.URL.Query().Get("access_token")), []byte(capability)) != 1 {
			w.WriteHeader(http.StatusUnauthorized)
			return
		}
		runtime.mutex.Lock()
		defer runtime.mutex.Unlock()
		router.ServeHTTP(w, r)
	})
	runtime.server = &http.Server{Handler: handler, ReadHeaderTimeout: 5 * time.Second}
	go func() { _ = runtime.server.Serve(listener) }()
	// The job owns this lifecycle. Close is called after post steps and cleanup.
	_ = ctx
	return runtime, nil
}
