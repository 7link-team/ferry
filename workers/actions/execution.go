package main

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"maps"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/nektos/act/pkg/artifactcache"
	"github.com/nektos/act/pkg/common"
	"github.com/nektos/act/pkg/container"
	"github.com/nektos/act/pkg/model"
	"github.com/nektos/act/pkg/runner"
	"gopkg.in/yaml.v3"
)

type JobInstance struct {
	Name   string         `json:"name"`
	Matrix map[string]any `json:"matrix"`
	index  int
}

// Detach execution cancellation from the status signal so act can evaluate
// cancelled()/always() on remaining steps. All cleanup shares one 30s grace.
const cancellationGrace = 30 * time.Second

func cleanupContext(parent context.Context, alreadyCancelled bool) (context.Context, context.CancelFunc) {
	ctx, stop := context.WithCancel(context.WithoutCancel(parent))
	signal, cancelSignal := context.WithCancel(parent)
	if alreadyCancelled {
		cancelSignal()
	}
	ctx = common.WithJobCancelContext(ctx, signal)
	done := make(chan struct{})
	go func() {
		select {
		case <-signal.Done():
			timer := time.NewTimer(cancellationGrace)
			defer timer.Stop()
			select {
			case <-timer.C:
				stop()
			case <-done:
			}
		case <-done:
		}
	}()
	return ctx, func() { close(done); cancelSignal(); stop() }
}

func evaluationContext(ctx context.Context, request Request, workflow *model.Workflow, config *runner.Config) (*runner.RunContext, error) {
	event := "{}"
	if request.EventPath != "" {
		data, err := os.ReadFile(request.EventPath)
		if err != nil {
			return nil, err
		}
		event = string(data)
	} else if len(request.Inputs) > 0 {
		data, err := json.Marshal(map[string]any{"inputs": request.Inputs})
		if err != nil {
			return nil, err
		}
		event = string(data)
	}
	rc := &runner.RunContext{Config: config, Run: &model.Run{Workflow: workflow, JobID: request.Job}, Matrix: request.Matrix, EventJSON: event, StepResults: map[string]*model.StepResult{}}
	rc.Cancelled.Store(request.RunCancelled)
	rc.ExprEval = rc.NewExpressionEvaluator(ctx)
	return rc, nil
}

func hydrate(request Request, workflow *model.Workflow) error {
	job := workflow.GetJob(request.Job)
	if job == nil {
		return fmt.Errorf("unknown job %q", request.Job)
	}
	pending := append([]string{}, job.Needs()...)
	seen := map[string]bool{}
	for len(pending) > 0 {
		id := pending[0]
		pending = pending[1:]
		if seen[id] {
			continue
		}
		seen[id] = true
		dependency := workflow.GetJob(id)
		value, ok := request.Needs[id]
		if dependency == nil || !ok {
			return fmt.Errorf("missing completed dependency %q", id)
		}
		dependency.Result, dependency.Outputs = value.State, maps.Clone(value.Outputs)
		if value.ContinueOnError && value.State == "failure" {
			dependency.Result = "success"
		}
		pending = append(pending, dependency.Needs()...)
	}
	return nil
}

func evalBool(ctx context.Context, rc *runner.RunContext, raw string, fallback bool) (bool, error) {
	if raw == "" {
		return fallback, nil
	}
	value := rc.NewExpressionEvaluator(ctx).Interpolate(ctx, raw)
	b, err := strconv.ParseBool(value)
	if err != nil {
		return false, fmt.Errorf("expected boolean, got %q", value)
	}
	return b, nil
}

func jobContinueOnError(ctx context.Context, source, id string, rc *runner.RunContext) (bool, error) {
	var document struct {
		Jobs map[string]struct {
			Continue string `yaml:"continue-on-error"`
		} `yaml:"jobs"`
	}
	if err := yaml.Unmarshal([]byte(source), &document); err != nil {
		return false, err
	}
	return evalBool(ctx, rc, document.Jobs[id].Continue, false)
}

func execute(ctx context.Context, request Request, output io.Writer) (JobResult, error) {
	result := JobResult{State: "failure", Outputs: map[string]string{}, Steps: map[string]string{}}
	if request.Mode != "" && request.Mode != "plan" {
		return result, fmt.Errorf("unknown worker mode %q", request.Mode)
	}
	if request.TimeoutSeconds > 0 {
		if int64(request.TimeoutSeconds) > int64((1<<63-1)/time.Second) {
			return result, fmt.Errorf("timeout_seconds exceeds the representable duration")
		}
		var cancel context.CancelFunc
		ctx, cancel = context.WithTimeout(ctx, time.Duration(request.TimeoutSeconds)*time.Second)
		defer cancel()
	}
	request.Env = maps.Clone(request.Env)
	if request.Env == nil {
		request.Env = map[string]string{}
	}
	request.Secrets = maps.Clone(request.Secrets)
	if request.Secrets == nil {
		request.Secrets = map[string]string{}
	}
	if request.EventName == "" {
		request.EventName = "workflow_dispatch"
	}
	for key, value := range map[string]string{"GITHUB_RUN_ID": request.RunID, "GITHUB_REF": request.Ref, "SHA_REF": request.SHA, "GITHUB_REPOSITORY": request.Repository, "GITHUB_SERVER_URL": request.ServerURL} {
		if value != "" {
			request.Env[key] = value
		}
	}
	if request.RunNumber != 0 {
		request.Env["GITHUB_RUN_NUMBER"] = strconv.FormatInt(request.RunNumber, 10)
	}
	if request.RunAttempt != 0 {
		request.Env["GITHUB_RUN_ATTEMPT"] = strconv.Itoa(request.RunAttempt)
	}
	workflow, err := model.ReadWorkflow(strings.NewReader(request.Workflow), false)
	if err != nil {
		return result, err
	}
	if err = hydrate(request, workflow); err != nil {
		return result, err
	}
	job := workflow.GetJob(request.Job)
	config := &runner.Config{Workdir: request.Workdir, Env: request.Env, Secrets: request.Secrets, Vars: request.Vars, Inputs: request.Inputs, Actor: request.Actor, Token: request.Secrets["GITHUB_TOKEN"], EventName: request.EventName, GitHubInstance: "github.com", RemoteName: "origin"}
	rc, err := evaluationContext(ctx, request, workflow, config)
	if err != nil {
		return result, err
	}
	parallel, failFast := 1, true
	if job.Strategy != nil {
		if err = runner.EvaluateMatrix(ctx, rc.ExprEval, &job.Strategy.RawMatrix); err != nil {
			return result, fmt.Errorf("matrix: %w", err)
		}
		failFast, err = evalBool(ctx, rc, job.Strategy.FailFastString, true)
		if err != nil {
			return result, err
		}
	}
	matrices, err := job.GetMatrixes()
	if err != nil {
		return result, err
	}
	parallel = len(matrices)
	if job.Strategy != nil && job.Strategy.MaxParallelString != "" {
		parallel, err = strconv.Atoi(rc.ExprEval.Interpolate(ctx, job.Strategy.MaxParallelString))
		if err != nil || parallel < 1 {
			return result, fmt.Errorf("strategy.max-parallel must resolve to a positive integer")
		}
	}
	if job.Strategy != nil {
		job.Strategy.MaxParallel, job.Strategy.FailFast = parallel, failFast
	}
	instances := make([]JobInstance, 0, len(matrices))
	for index, matrix := range matrices {
		rc.Matrix = matrix
		rc.StrategyJobTotal = len(matrices)
		rc.StrategyJobIndex = index
		name := rc.NewExpressionEvaluator(ctx).Interpolate(ctx, rc.Run.String())
		if job.Name == "" && len(matrices) > 1 {
			name = fmt.Sprintf("%s (%d)", name, index+1)
		}
		instances = append(instances, JobInstance{Name: name, Matrix: matrix, index: index})
	}
	if request.Mode == "plan" {
		result.State = "success"
		result.Instances = instances
		result.FailFast = &failFast
		if job.Strategy != nil && job.Strategy.MaxParallelString != "" {
			result.MaxParallel = parallel
		}
		return result, nil
	}
	total := len(instances)
	if request.Matrix != nil {
		selected := JobInstance{Matrix: request.Matrix}
		encoded, _ := json.Marshal(request.Matrix)
		for _, candidate := range instances {
			value, _ := json.Marshal(candidate.Matrix)
			if string(value) == string(encoded) {
				selected.index = candidate.index
				break
			}
		}
		instances = []JobInstance{selected}
	}
	if len(instances) == 0 {
		result.State = "skipped"
		return result, nil
	}
	strategyParallel := parallel
	if parallel > len(instances) {
		parallel = len(instances)
	}
	if parallel < 1 {
		parallel = 1
	}
	// Services start only after the plan-only return. No process environment is
	// mutated: parallel legs receive their own config and runtime variables.
	cacheDir := request.CacheDir
	if cacheDir == "" {
		cacheDir = defaultCacheDir()
	}
	cache, err := artifactcache.StartHandler(cacheDir, "", "127.0.0.1", 0, nil)
	if err != nil {
		return result, fmt.Errorf("actions/cache: %w", err)
	}
	defer cache.Close()
	request.Env["ACTIONS_CACHE_URL"] = cache.ExternalURL() + "/"
	request.Env["ACTIONS_CACHE_SERVICE_V2"] = "false" // act serves the v1 cache API used by cache@v4's fallback.
	request.Secrets["_CACHE_CAPABILITY"] = strings.TrimPrefix(cache.ExternalURL(), "http://127.0.0.1:"+strconv.Itoa(cache.GetActualPort())+"/")
	// act uses the cache URL capability for authorization. Supply a runtime token
	// even when artifacts are disabled, as the cache toolkit requires one.
	request.Env["ACTIONS_RUNTIME_TOKEN"] = request.Secrets["_CACHE_CAPABILITY"]
	var artifact *artifactRuntime
	if request.ArtifactDir != "" {
		if request.RunID == "" {
			request.RunID = "1"
		}
		artifact, err = startArtifacts(ctx, request.ArtifactDir, request.RunID, containerJob(job, request.Platforms))
		if err != nil {
			return result, err
		}
		defer artifact.Close()
		request.Env["ACTIONS_RUNTIME_TOKEN"] = artifact.Token
		request.Env["ACTIONS_RUNTIME_URL"] = artifact.URL + "/"
		request.Secrets["_RUNTIME_TOKEN"] = artifact.Token
		request.Secrets["_ARTIFACT_CAPABILITY"] = artifact.capability
	}
	if containerJob(job, request.Platforms) {
		// The engine's Docker client reads DOCKER_HOST; every leg shares one value,
		// so it is set once for the process and restored afterwards.
		socket, e := container.GetSocketAndHost("")
		if e != nil {
			return result, e
		}
		previous, exists := os.LookupEnv("DOCKER_HOST")
		if err = os.Setenv("DOCKER_HOST", socket.Host); err != nil {
			return result, err
		}
		defer func() {
			if exists {
				_ = os.Setenv("DOCKER_HOST", previous)
			} else {
				_ = os.Unsetenv("DOCKER_HOST")
			}
		}()
	}
	output = &lockedWriter{output: output}
	legsCtx, cancelLegs := context.WithCancel(ctx)
	defer cancelLegs()
	var mutex sync.Mutex
	next := 0
	result.State = "success"
	var firstErr error
	failed, tolerated := false, true
	var workers sync.WaitGroup
	for slot := 0; slot < parallel; slot++ {
		workers.Add(1)
		go func() {
			defer workers.Done()
			for {
				mutex.Lock()
				if next == len(instances) || legsCtx.Err() != nil {
					mutex.Unlock()
					return
				}
				index := next
				next++
				mutex.Unlock()
				leg := request
				leg.Matrix = instances[index].Matrix
				value, e := executeInstance(legsCtx, leg, output, instances[index].index, total, strategyParallel, failFast)
				mutex.Lock()
				for key, v := range value.Outputs {
					result.Outputs[key] = v
				}
				for key, v := range value.Steps {
					rank := map[string]int{"skipped": 1, "success": 2, "failure": 3, "cancelled": 4, "timed_out": 5}
					if rank[v] > rank[result.Steps[key]] {
						result.Steps[key] = v
					}
				}
				if value.State == "failure" || value.State == "timed_out" {
					failed = true
					tolerated = tolerated && value.ContinueOnError
					result.State = value.State
					if firstErr == nil {
						firstErr = e
					}
					if failFast && !value.ContinueOnError {
						cancelLegs()
					}
				} else if value.State == "skipped" && len(instances) == 1 {
					result.State = "skipped"
				} else if value.State == "cancelled" && !failed {
					result.State = "cancelled"
					if firstErr == nil {
						firstErr = e
					}
				}
				mutex.Unlock()
			}
		}()
	}
	workers.Wait()
	result.ContinueOnError = failed && tolerated
	if len(instances) == 1 && !failed {
		rc.Matrix = instances[0].Matrix
		result.ContinueOnError, err = jobContinueOnError(ctx, request.Workflow, request.Job, rc)
		if err != nil {
			return result, err
		}
	}
	if ctx.Err() != nil {
		result.State = "cancelled"
		if ctx.Err() == context.DeadlineExceeded {
			result.State = "timed_out"
		}
		firstErr = ctx.Err()
	}
	if artifact != nil {
		result.ArtifactRetention = artifact.Retention()
		err = filepath.WalkDir(request.ArtifactDir, func(path string, entry os.DirEntry, e error) error {
			if e != nil {
				return e
			}
			if entry.Type().IsRegular() && strings.HasSuffix(entry.Name(), ".zip") {
				rel, e := filepath.Rel(request.ArtifactDir, path)
				if e != nil {
					return e
				}
				result.Artifacts = append(result.Artifacts, filepath.ToSlash(rel))
			}
			return nil
		})
		if err != nil {
			return result, err
		}
	}
	return result, firstErr
}

func containerJob(job *model.Job, platforms map[string]string) bool {
	if job.Container() != nil {
		return true
	}
	for _, platform := range platforms {
		if platform != "-self-hosted" {
			return true
		}
	}
	return false
}

// defaultCacheDir keeps actions/cache data across jobs. A service account may
// have no HOME (and so no user cache directory); the temp directory then holds it.
func defaultCacheDir() string {
	base, err := os.UserCacheDir()
	if err != nil {
		base = os.TempDir()
	}
	return filepath.Join(base, "ferry", "actions-cache")
}
