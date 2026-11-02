// Actions execution worker. Ferry owns scheduling and transport; act owns workflow semantics.
package main

import (
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"maps"
	"os"
	"os/signal"
	"path/filepath"
	"slices"
	"strconv"
	"strings"
	"sync"
	"syscall"
	"time"

	"github.com/nektos/act/pkg/common"
	"github.com/nektos/act/pkg/container"
	"github.com/nektos/act/pkg/model"
	"github.com/nektos/act/pkg/runner"
	"github.com/sirupsen/logrus"
)

const engineVersion = "nektos/act v0.2.89"

type JobResult struct {
	ContinueOnError   bool              `json:"continue_on_error"`
	Instances         []JobInstance     `json:"instances,omitempty"`
	ArtifactRetention map[string]int    `json:"artifact_retention,omitempty"`
	Steps             map[string]string `json:"steps,omitempty"`
	Artifacts         []string          `json:"artifacts,omitempty"`
	State             string            `json:"state"`
	Outputs           map[string]string `json:"outputs"`
}
type Request struct {
	Actor          string               `json:"actor"`
	RunNumber      int64                `json:"run_number"`
	RunAttempt     int                  `json:"run_attempt"`
	Ref            string               `json:"ref"`
	SHA            string               `json:"sha"`
	Repository     string               `json:"repository"`
	ServerURL      string               `json:"server_url"`
	RunCancelled   bool                 `json:"run_cancelled"`
	Mode           string               `json:"mode"`
	Matrix         map[string]any       `json:"matrix"`
	CacheDir       string               `json:"cache_dir"`
	ArtifactDir    string               `json:"artifact_dir"`
	RunID          string               `json:"run_id"`
	Workflow       string               `json:"workflow"`
	Job            string               `json:"job"`
	Workdir        string               `json:"workdir"`
	EventName      string               `json:"event_name"`
	EventPath      string               `json:"event_path"`
	Needs          map[string]JobResult `json:"needs"`
	Platforms      map[string]string    `json:"platforms"`
	Env            map[string]string    `json:"env"`
	Secrets        map[string]string    `json:"secrets"`
	Vars           map[string]string    `json:"vars"`
	Inputs         map[string]string    `json:"inputs"`
	TimeoutSeconds int                  `json:"timeout_seconds"`
}

// act masks the message, but structured fields (for example set-output's arg)
// also carry user values. Sanitize every string before JSON serialization.
type safeFormatter struct{ secrets map[string]string }

func (f safeFormatter) Format(entry *logrus.Entry) ([]byte, error) {
	masks := []string{}
	for _, value := range f.secrets {
		if value != "" {
			masks = append(masks, value)
		}
	}
	if entry.Context != nil {
		if dynamic := runner.Masks(entry.Context); dynamic != nil {
			masks = append(masks, (*dynamic)...)
		}
	}
	for _, mask := range slices.Clone(masks) {
		masks = append(masks, strings.FieldsFunc(mask, func(r rune) bool { return r == '\n' || r == '\r' })...)
	}
	redact := func(value string) string {
		for _, mask := range masks {
			if mask != "" {
				value = strings.ReplaceAll(value, mask, "***")
			}
		}
		return value
	}
	data, err := (&logrus.JSONFormatter{}).Format(entry)
	if err != nil {
		return nil, err
	}
	var value any
	if err = json.Unmarshal(data, &value); err != nil {
		return nil, err
	}
	var clean func(any) any
	clean = func(value any) any {
		switch v := value.(type) {
		case string:
			return redact(v)
		case []any:
			for i, item := range v {
				v[i] = clean(item)
			}
			return v
		case map[string]any:
			out := map[string]any{}
			for key, item := range v {
				out[redact(key)] = clean(item)
			}
			return out
		default:
			return value
		}
	}
	data, err = json.Marshal(clean(value))
	return append(data, '\n'), err
}

type stepRecorder struct {
	mutex  sync.Mutex
	states map[string]string
	masks  map[string]bool
}

func (r *stepRecorder) Levels() []logrus.Level { return logrus.AllLevels }
func (r *stepRecorder) Fire(entry *logrus.Entry) error {
	r.mutex.Lock()
	defer r.mutex.Unlock()
	if entry.Context != nil {
		if masks := runner.Masks(entry.Context); masks != nil {
			for _, mask := range *masks {
				if mask != "" {
					r.masks[mask] = true
				}
			}
		}
	}
	ids, ok := entry.Data["stepID"].([]string)
	if !ok || len(ids) != 1 {
		return nil
	}
	value, ok := entry.Data["stepResult"]
	if !ok {
		return nil
	}
	state := fmt.Sprint(value)
	rank := map[string]int{"skipped": 1, "success": 2, "failure": 3, "cancelled": 4, "timed_out": 5}
	if rank[state] > rank[r.states[ids[0]]] {
		r.states[ids[0]] = state
	}
	return nil
}

type lockedWriter struct {
	mutex  sync.Mutex
	output io.Writer
}

func (w *lockedWriter) Write(data []byte) (int, error) {
	w.mutex.Lock()
	defer w.mutex.Unlock()
	return w.output.Write(data)
}

type loggerFactory struct {
	output   io.Writer
	secrets  map[string]string
	recorder *stepRecorder
}

func (f loggerFactory) WithJobLogger() *logrus.Logger {
	logger := logrus.New()
	logger.SetOutput(f.output)
	logger.SetLevel(logrus.InfoLevel)
	logger.SetFormatter(safeFormatter{secrets: f.secrets})
	logger.AddHook(f.recorder)
	return logger
}

func executeInstance(ctx context.Context, request Request, output io.Writer, index, total, parallel int, failFast bool) (JobResult, error) {
	result := JobResult{State: "failure", Outputs: map[string]string{}, Artifacts: []string{}, Steps: map[string]string{}}
	workflow, err := model.ReadWorkflow(strings.NewReader(request.Workflow), false)
	if err != nil {
		return result, fmt.Errorf("invalid workflow: %w", err)
	}
	job := workflow.GetJob(request.Job)
	if job == nil {
		return result, fmt.Errorf("unknown job %q", request.Job)
	}
	if err := hydrate(request, workflow); err != nil {
		return result, err
	}

	// Each instance reparses the workflow: act mutates steps and outputs.
	if job.Strategy != nil {
		job.Strategy.MaxParallel = parallel
		job.Strategy.FailFast = failFast
	}
	if request.EventName == "" {
		request.EventName = "workflow_dispatch"
	}

	containerJob := job.Container() != nil
	for _, platform := range request.Platforms {
		if platform != "-self-hosted" {
			containerJob = true
		}
	}
	request.Env = maps.Clone(request.Env)
	if request.Env == nil {
		request.Env = map[string]string{}
	}
	request.Secrets = maps.Clone(request.Secrets)
	if request.Secrets == nil {
		request.Secrets = map[string]string{}
	}
	recorder := &stepRecorder{states: map[string]string{}, masks: map[string]bool{}}
	factory := loggerFactory{output: &lockedWriter{output: output}, secrets: request.Secrets, recorder: recorder}
	logger := factory.WithJobLogger()
	ctx = common.WithLogger(ctx, logger.WithContext(ctx))
	ctx = runner.WithJobLoggerFactory(ctx, factory)
	config := &runner.Config{Actor: request.Actor, Token: request.Secrets["GITHUB_TOKEN"], Workdir: request.Workdir, ActionCacheDir: filepath.Join(filepath.Dir(request.Workdir), "action-cache", strconv.Itoa(index)), EventName: request.EventName, EventPath: request.EventPath, Platforms: request.Platforms,
		Env: request.Env, Secrets: request.Secrets, Vars: request.Vars, Inputs: request.Inputs, LogOutput: true, JSONLogger: true, AutoRemove: true,
		GitHubInstance: "github.com", RemoteName: "origin", ConcurrentJobs: 4}
	if request.ArtifactDir != "" {
		config.ArtifactServerPath = request.ArtifactDir
		if strings.Contains(request.Env["ACTIONS_RUNTIME_URL"], "host.docker.internal") {
			config.ContainerOptions = "--add-host=host.docker.internal:host-gateway"
		}
	}
	if containerJob {
		socket, err := container.GetSocketAndHost("")
		if err != nil {
			return result, err
		}
		config.ContainerDaemonSocket = socket.Socket
	}
	rc, err := evaluationContext(ctx, request, workflow, config)
	if err != nil {
		return result, err
	}
	rc.Matrix = request.Matrix
	rc.StrategyJobTotal = total
	rc.StrategyJobIndex = index
	rc.Cancelled.Store(request.RunCancelled)
	rc.ExprEval = rc.NewExpressionEvaluator(ctx)
	result.ContinueOnError, err = jobContinueOnError(ctx, request.Workflow, request.Job, rc)
	if err != nil {
		return result, err
	}
	minutes := rc.ExprEval.Interpolate(ctx, job.TimeoutMinutes)
	if job.TimeoutMinutes != "" && minutes == "" {
		return result, fmt.Errorf("job timeout-minutes must resolve to a positive duration")
	}
	seconds, err := jobTimeBudget(request.TimeoutSeconds, minutes)
	if err != nil {
		return result, err
	}
	jobCtx, cancel := context.WithTimeout(ctx, time.Duration(seconds)*time.Second)
	defer cancel()
	execCtx, finish := cleanupContext(jobCtx, request.RunCancelled)
	defer finish()
	execCtx = common.WithJobErrorContainer(runner.WithJobLogger(execCtx, request.Job, request.Job, config, &rc.Masks, request.Matrix))
	rc.Name = rc.ExprEval.Interpolate(execCtx, rc.Run.String())
	rc.JobName = rc.Name
	executor, err := rc.Executor()
	if err != nil {
		return result, err
	}
	err = executor(execCtx)
	result.Steps = maps.Clone(recorder.states)
	result.State = job.Result
	if result.State == "" {
		result.State = "failure"
	}
	result.Outputs = job.Outputs
	if result.Outputs == nil || result.State == "skipped" {
		result.Outputs = map[string]string{}
	}
	if result.State == "failure" && err == nil {
		err = fmt.Errorf("job %q failed", request.Job)
	}
	if errors.Is(jobCtx.Err(), context.DeadlineExceeded) {
		result.State = "timed_out"
		err = jobCtx.Err()
	}
	if errors.Is(jobCtx.Err(), context.Canceled) {
		result.State = "cancelled"
		err = jobCtx.Err()
	}
	if jobCtx.Err() == nil && execCtx.Err() != nil {
		result.State = "cancelled"
		err = execCtx.Err()
	}
	outputMasks := maps.Clone(recorder.masks)
	for _, secret := range request.Secrets {
		outputMasks[secret] = true
	}
	for mask := range maps.Clone(outputMasks) {
		for _, line := range strings.FieldsFunc(mask, func(r rune) bool { return r == '\n' || r == '\r' }) {
			outputMasks[line] = true
		}
	}
	for key, value := range result.Outputs {
		for secret := range outputMasks {
			if secret != "" && strings.Contains(value, secret) {
				delete(result.Outputs, key)
				break
			}
		}
	}
	return result, err
}

func jobTimeBudget(limit int, minutes string) (int, error) {
	seconds := 360 * 60
	if minutes != "" {
		value, err := strconv.Atoi(minutes)
		if err != nil || value < 1 || value > int((1<<63-1)/int64(time.Minute)) {
			return 0, fmt.Errorf("job timeout-minutes must resolve to a positive duration")
		}
		seconds = value * 60
	}
	if limit > 0 && seconds > limit {
		seconds = limit
	}
	return seconds, nil
}

func main() {
	path := flag.String("request", "", "private execution request JSON file")
	version := flag.Bool("version", false, "print pinned engine version")
	flag.Parse()
	if *version {
		fmt.Println(engineVersion)
		return
	}
	data, err := os.ReadFile(*path)
	var request Request
	if err == nil {
		err = json.Unmarshal(data, &request)
	}
	if err != nil {
		fmt.Fprintln(os.Stderr, "Cannot read execution request")
		os.Exit(2)
	}
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	result, err := execute(ctx, request, os.Stdout)
	_ = json.NewEncoder(os.Stdout).Encode(struct {
		Kind string `json:"kind"`
		JobResult
	}{"result", result})
	if err != nil || result.State == "failure" || result.State == "cancelled" || result.State == "timed_out" {
		os.Exit(1)
	}
}
