// Prepare isolated sources with pinned act PTY and matrix fixes. The module
// cache stays immutable; exact replacements fail closed when upstream changes.
package main

import (
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

func main() {
	if err := generate(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

func generate() error {
	if len(os.Args) != 2 {
		return fmt.Errorf("usage: actpatch temporary-directory")
	}
	goTool := os.Getenv("ACTIONS_GO")
	if goTool == "" {
		goTool = "go"
	}
	cmd := exec.Command(goTool, "mod", "download", "-json", "github.com/nektos/act")
	output, err := cmd.Output()
	if err != nil {
		return fmt.Errorf("locate pinned act: %w", err)
	}
	var module struct{ Dir string }
	if err := json.Unmarshal(output, &module); err != nil {
		return err
	}
	if module.Dir == "" {
		return fmt.Errorf("downloaded act module has no source directory")
	}
	workspace := filepath.Join(os.Args[1], "worker")
	if err := os.CopyFS(workspace, os.DirFS(".")); err != nil {
		return err
	}
	moduleDir := filepath.Join(workspace, "act")
	if err := os.CopyFS(moduleDir, os.DirFS(module.Dir)); err != nil {
		return err
	}
	source := filepath.Join(moduleDir, "pkg", "container", "host_environment.go")
	data, err := os.ReadFile(source)
	if err != nil {
		return err
	}
	patched := string(data)
	for _, change := range [][2]string{
		{"\"strings\"\n", "\"strings\"\n\t\"sync/atomic\"\n"},
		{"AutoStop  bool", "AutoStop  atomic.Bool"},
		{"if w.AutoStop &&", "if w.AutoStop.Load() &&"},
		{"writer.AutoStop = true", "writer.AutoStop.Store(true)"},
	} {
		if strings.Count(patched, change[0]) != 1 {
			return fmt.Errorf("act PTY patch no longer applies: %q", change[0])
		}
		patched = strings.Replace(patched, change[0], change[1], 1)
	}
	if err := os.WriteFile(source, []byte(patched), 0600); err != nil {
		return err
	}
	// act expands matrices but ignores fail-fast. Ferry uses one matrix slot;
	// the existing sequential executor stops before scheduling later branches.
	runnerSource := filepath.Join(moduleDir, "pkg", "runner", "runner.go")
	runnerData, err := os.ReadFile(runnerSource)
	if err != nil {
		return err
	}
	original := "pipeline = append(pipeline, common.NewParallelExecutor(maxParallel, stageExecutor...))"
	replacement := `if maxParallel == 1 && job.Strategy != nil && job.Strategy.FailFast {
                    for i, executor := range stageExecutor {
                        stageExecutor[i] = executor.Then(handleFailure(plan))
                    }
                    pipeline = append(pipeline, common.NewPipelineExecutor(stageExecutor...))
                } else {
                    pipeline = append(pipeline, common.NewParallelExecutor(maxParallel, stageExecutor...))
                }`
	if strings.Count(string(runnerData), original) != 1 {
		return fmt.Errorf("act matrix fail-fast patch no longer applies")
	}
	if err := os.WriteFile(runnerSource, []byte(strings.Replace(string(runnerData), original, replacement, 1)), 0600); err != nil {
		return err
	}
	// Display names are not execution identities. Every server job invocation
	// has a private action-cache directory, so use it to namespace Docker
	// containers, volumes and service networks without changing user contexts.
	contextSource := filepath.Join(moduleDir, "pkg", "runner", "run_context.go")
	contextData, err := os.ReadFile(contextSource)
	if err != nil {
		return err
	}
	original = `return createContainerName("act", rc.String())`
	replacement = `return createContainerName("act", rc.Config.ActionCacheDir, rc.Run.JobID, rc.String())`
	if strings.Count(string(contextData), original) != 1 {
		return fmt.Errorf("act invocation namespace patch no longer applies")
	}
	if err := os.WriteFile(contextSource, []byte(strings.Replace(string(contextData), original, replacement, 1)), 0600); err != nil {
		return err
	}
	mod, err := os.ReadFile("go.mod")
	if err != nil {
		return err
	}
	mod = append(mod, []byte("\nreplace github.com/nektos/act => ./act\n")...)
	if err := os.WriteFile(filepath.Join(workspace, "go.mod"), mod, 0600); err != nil {
		return err
	}
	sum, err := os.ReadFile("go.sum")
	if err != nil {
		return err
	}
	return os.WriteFile(filepath.Join(workspace, "go.sum"), sum, 0600)
}
