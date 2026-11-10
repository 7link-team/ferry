package main

import (
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"maps"
	"os"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"time"

	"github.com/nektos/act/pkg/model"
	"gopkg.in/yaml.v3"
)

// `ferry/preview@v1` (docs/preview-automation-design.md section 5). The step
// asks the service for a preview with a marker line on its log, then waits for
// the answer the service writes into the job's private directory.

const previewMarker = "::ferry-preview::"

// How often the response file is read; tests shorten it.
var previewPoll = 500 * time.Millisecond

// The worker gives up this long after the service's own deadline, so a lost
// response (for example a disconnected host) still ends the step.
var previewGrace = 30 * time.Second

// builtinStepRewriter rewrites the built-in actions this worker implements.
func builtinStepRewriter(nonce, dir string) func(*model.Step) error {
	return func(step *model.Step) error {
		if err := rewriteBuiltinArtifactStep(step); err != nil {
			return err
		}
		if err := rewriteBuiltinNotificationStep(step, nonce, dir); err != nil {
			return err
		}
		return rewriteBuiltinPreviewStep(step, nonce, dir)
	}
}

func rewriteBuiltinPreviewStep(step *model.Step, nonce, dir string) error {
	const prefix = "ferry/preview@v1"
	if step.Uses != prefix && !strings.HasPrefix(step.Uses, prefix+".") {
		return nil
	}
	executable, err := os.Executable()
	if err != nil {
		return err
	}
	// Inputs travel through the environment, never through shell source.
	env := maps.Clone(step.Environment())
	if env == nil {
		env = map[string]string{}
	}
	for key, value := range step.With {
		env["INPUT_FERRY_PREVIEW_"+strings.ToUpper(strings.ReplaceAll(key, "-", "_"))] = value
	}
	env["FERRY_PREVIEW_EXECUTABLE"] = executable
	env["FERRY_PREVIEW_STEP_NAME"] = step.Name
	// Only this step sees the nonce, so no other step can forge a request.
	env["FERRY_PREVIEW_NONCE"] = nonce
	env["FERRY_PREVIEW_DIR"] = dir
	var node yaml.Node
	if err := node.Encode(env); err != nil {
		return err
	}
	step.Env = node
	if step.Name == "" {
		step.Name = step.Uses
	}
	step.Uses, step.With = "", nil
	step.Shell = "sh"
	step.Run = `"$FERRY_PREVIEW_EXECUTABLE" preview`
	if runtime.GOOS == "windows" {
		step.Shell = "cmd"
		step.Run = "@\"%FERRY_PREVIEW_EXECUTABLE%\" preview\r\n@exit /b %errorlevel%"
	}
	return nil
}

// previewRequest is the marker's JSON payload.
type previewRequest struct {
	ID                  string `json:"id"`
	Port                int    `json:"port"`
	Name                string `json:"name"`
	StripPrefix         bool   `json:"strip_prefix"`
	Hold                string `json:"hold"`
	ReadyTimeoutSeconds int    `json:"ready_timeout_seconds"`
	TimeoutMinutes      int    `json:"timeout_minutes"`
	Response            string `json:"response"`
}

type previewResponse struct {
	URL      string `json:"url"`
	Error    string `json:"error"`
	Decision string `json:"decision"`
}

func previewInteger(getenv func(string) string, key string, fallback, low, high int) (int, error) {
	value := strings.TrimSpace(getenv("INPUT_FERRY_PREVIEW_" + key))
	if value == "" {
		if fallback == 0 {
			return 0, fmt.Errorf("input %s is required", strings.ToLower(strings.ReplaceAll(key, "_", "-")))
		}
		return fallback, nil
	}
	number, err := strconv.Atoi(value)
	if err != nil || number < low || number > high {
		return 0, fmt.Errorf("input %s must be an integer from %d to %d", strings.ToLower(strings.ReplaceAll(key, "_", "-")), low, high)
	}
	return number, nil
}

// previewInputs validates the step's inputs and builds its request.
func previewInputs(getenv func(string) string) (previewRequest, error) {
	request := previewRequest{StripPrefix: true, Hold: "none"}
	var err error
	if request.Port, err = previewInteger(getenv, "PORT", 0, 1, 65535); err != nil {
		return request, err
	}
	if request.ReadyTimeoutSeconds, err = previewInteger(getenv, "READY_TIMEOUT_SECONDS", 60, 1, 600); err != nil {
		return request, err
	}
	if request.TimeoutMinutes, err = previewInteger(getenv, "TIMEOUT_MINUTES", 60, 1, 1440); err != nil {
		return request, err
	}
	switch value := strings.ToLower(strings.TrimSpace(getenv("INPUT_FERRY_PREVIEW_STRIP_PREFIX"))); value {
	case "", "true":
	case "false":
		request.StripPrefix = false
	default:
		return request, fmt.Errorf("input strip-prefix must be true or false")
	}
	switch value := strings.TrimSpace(getenv("INPUT_FERRY_PREVIEW_HOLD")); value {
	case "", "none":
	case "approval":
		request.Hold = value
	default:
		return request, fmt.Errorf("input hold must be approval or none")
	}
	request.Name = strings.TrimSpace(getenv("INPUT_FERRY_PREVIEW_NAME"))
	if request.Name == "" {
		request.Name = strings.TrimSpace(getenv("FERRY_PREVIEW_STEP_NAME"))
	}
	return request, nil
}

func runPreviewHelper(output io.Writer) error {
	request, err := previewInputs(os.Getenv)
	if err != nil {
		return err
	}
	nonce, dir := os.Getenv("FERRY_PREVIEW_NONCE"), os.Getenv("FERRY_PREVIEW_DIR")
	if nonce == "" || dir == "" {
		return fmt.Errorf("ferry/preview needs a job started by the Ferry service")
	}
	var id [8]byte
	if _, err := rand.Read(id[:]); err != nil {
		return fmt.Errorf("cannot create a preview request id")
	}
	request.ID = "preview-" + hex.EncodeToString(id[:])
	if err := os.MkdirAll(dir, 0700); err != nil {
		return fmt.Errorf("cannot prepare the preview response directory")
	}
	// The service writes exactly this path; it is sent so either side can
	// check that both agree.
	request.Response = dir + "/" + request.ID + ".json"
	return awaitPreview(output, nonce, request, os.Getenv("GITHUB_OUTPUT"))
}

// readPreview returns the response once it parses; a missing or partially
// written file reads as "not yet".
func readPreview(path string) (previewResponse, bool) {
	var response previewResponse
	data, err := os.ReadFile(path)
	if err != nil || json.Unmarshal(data, &response) != nil {
		return response, false
	}
	return response, true
}

func pollPreview(path string, timeout time.Duration, done func(previewResponse) bool) (previewResponse, error) {
	deadline := time.Now().Add(timeout)
	for {
		if response, ok := readPreview(path); ok && (response.Error != "" || done(response)) {
			if response.Error != "" {
				return response, errors.New(response.Error)
			}
			return response, nil
		}
		if time.Now().After(deadline) {
			return previewResponse{}, fmt.Errorf("no answer from the Ferry service within %s", timeout)
		}
		time.Sleep(previewPoll)
	}
}

// previewOutput sets the step's `url` output. The service sends a single-line
// root-relative path; anything else is refused rather than written.
func previewOutput(path, url string) error {
	if strings.ContainsAny(url, "\r\n") {
		return fmt.Errorf("the Ferry service sent an invalid preview link")
	}
	file, err := os.OpenFile(path, os.O_WRONLY|os.O_APPEND, 0600)
	if err != nil {
		return fmt.Errorf("cannot open the step output file")
	}
	defer file.Close()
	if _, err := fmt.Fprintf(file, "url=%s\n", url); err != nil {
		return fmt.Errorf("cannot write the step output")
	}
	return nil
}

func awaitPreview(output io.Writer, nonce string, request previewRequest, githubOutput string) error {
	data, err := json.Marshal(request)
	if err != nil {
		return err
	}
	fmt.Fprintf(output, "Requesting a preview of port %d\n", request.Port)
	fmt.Fprintf(output, "%s%s::%s\n", previewMarker, nonce, data)
	path := filepath.FromSlash(request.Response)
	ready := time.Duration(request.ReadyTimeoutSeconds)*time.Second + previewGrace
	response, err := pollPreview(path, ready, func(response previewResponse) bool { return response.URL != "" })
	if err != nil {
		return err
	}
	if githubOutput != "" {
		if err := previewOutput(githubOutput, response.URL); err != nil {
			return err
		}
	}
	fmt.Fprintf(output, "Preview: %s\n", response.URL)
	if request.Hold != "approval" {
		return nil
	}
	fmt.Fprintln(output, "Waiting for the preview to be approved or rejected on the run page")
	review := time.Duration(request.TimeoutMinutes)*time.Minute + previewGrace
	response, err = pollPreview(path, review, func(response previewResponse) bool { return response.Decision != "" })
	if err != nil {
		return err
	}
	fmt.Fprintf(output, "Review: %s\n", response.Decision)
	if response.Decision != "approved" {
		return fmt.Errorf("the preview was not approved (%s)", response.Decision)
	}
	return nil
}
