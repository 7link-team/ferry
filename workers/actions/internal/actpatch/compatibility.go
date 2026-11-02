package main

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

// Small, fail-closed seams for semantics owned by the worker. The worker drives
// isolated RunContexts, leaving step/action/reusable-workflow execution in act.
func patchCompatibility(moduleDir string) error {
	for _, patch := range []struct{ file, before, after string }{
		{"exprparser/functions.go", "func (impl *interperterImpl) jobSuccess() (bool, error) {", "func (impl *interperterImpl) jobSuccess() (bool, error) {\n\tif impl.env.Job.Status == \"cancelled\" { return false, nil }"},
		{"model/workflow.go", "matrixProduct := common.CartesianProduct(m)", "matrixProduct := orderedMatrixProduct(m, j.Strategy.RawMatrix)"},
		{"model/workflow.go", "\t\"github.com/nektos/act/pkg/common\"\n", ""},
		{"runner/run_context.go", "\"strings\"\n", "\"strings\"\n\t\"sync/atomic\"\n"},
		{"runner/run_context.go", "Cancelled           bool", "Cancelled           atomic.Bool\n\tStrategyJobTotal int\n\tStrategyJobIndex int"},
		{"runner/run_context.go", "if rc.Cancelled {", "if rc.Cancelled.Load() {"},
		{"runner/run_context.go", "for k, v := range rc.Run.Job().Outputs {\n\t\t\tinterpolated := ee.Interpolate(ctx, v)", "for k, v := range rc.Run.Job().Outputs {\n\t\t\tif ferryMissingOutput(v, rc.StepResults) { delete(rc.Run.Job().Outputs,k); continue }\n\t\t\tinterpolated := ee.Interpolate(ctx, v)"},
		{"runner/expression.go", "using := make(map[string]exprparser.Needs)\n\tstrategy := make(map[string]interface{})", "using := make(map[string]exprparser.Needs)\n\tstrategy := map[string]interface{}{\"job-total\": rc.StrategyJobTotal, \"job-index\": rc.StrategyJobIndex}"},
		{"runner/expression.go", "job := rc.Run.Job()\n\tstrategy := make(map[string]interface{})", "job := rc.Run.Job()\n\tstrategy := map[string]interface{}{\"job-total\": rc.StrategyJobTotal, \"job-index\": rc.StrategyJobIndex}"},
		{"runner/expression.go", `name := "workflow/hashfiles/index.js"`, `name := "workflow/hashfiles/index.cjs"`},
		{"runner/step.go", `.Debugf("Skipping step '%s' due to '%s'", stepModel, ifExpression)`, `.Infof("Skipping step '%s' due to '%s'", stepModel, ifExpression)`},
		{"runner/step.go", "rc.Cancelled = cctx != nil && cctx.Err() != nil", "rc.Cancelled.Store(rc.Cancelled.Load() || (cctx != nil && cctx.Err() != nil))"},
		{"runner/step.go", "if !rc.Cancelled && jobCancellationCtx != nil {", "if !rc.Cancelled.Load() && jobCancellationCtx != nil {"},
		{"runner/step.go", "rc.Cancelled = true", "rc.Cancelled.Store(true)"},
		{"runner/run_context.go", "func (rc *RunContext) startContainer() common.Executor {\n\treturn func(ctx context.Context) error {\n\t\tctx, cancel := common.EarlyCancelContext(ctx)", "func (rc *RunContext) startContainer() common.Executor {\n\treturn func(ctx context.Context) error {\n\t\tctx, cancel := context.WithCancel(ctx)"},
		// Runtime endpoints belong to this invocation, never process globals.
		{"runner/run_context.go", `actionsRuntimeURL := os.Getenv("ACTIONS_RUNTIME_URL")`, `actionsRuntimeURL := rc.Config.Env["ACTIONS_RUNTIME_URL"]`},
		{"runner/run_context.go", `actionsRuntimeToken := os.Getenv("ACTIONS_RUNTIME_TOKEN")`, `actionsRuntimeToken := rc.Config.Env["ACTIONS_RUNTIME_TOKEN"]`},
		// Bound post hooks and stop/remove to the same worker grace context.
		{"runner/job_executor.go", "context.WithTimeout(common.WithLogger(context.Background(), common.Logger(ctx)), time.Minute)", "context.WithTimeout(ctx, 30*time.Second)"},
		{"runner/job_executor.go", "context.WithTimeout(common.WithLogger(context.Background(), common.Logger(ctx)), 5*time.Minute)", "context.WithTimeout(ctx, 30*time.Second)"},
		{"runner/run_context.go", "s := bufio.NewScanner(reader)", "decoded, err := container.CommandFileReader(reader)\n\tif err != nil { return err }\n\ts := bufio.NewScanner(decoded)"},
		{"runner/step.go", "summary, err := io.ReadAll(reader)", "decoded, err := container.CommandFileReader(reader)\n\tif err != nil { return err }\n\tsummary, err := io.ReadAll(decoded)"},
		{"container/ferry_command_file.go", "func commandFileReader(", "func CommandFileReader("},
		{"container/parse_env_file.go", "commandFileReader(reader)", "CommandFileReader(reader)"},
		// GetEnv wrote rc.Env on every call, racing the cancellation monitor's
		// condition re-evaluation; after creation it is now read-only.
		{"runner/run_context.go", "\trc.Env[\"ACT\"] = \"true\"\n\treturn rc.Env", "\tif rc.Env[\"ACT\"] != \"true\" {\n\t\trc.Env[\"ACT\"] = \"true\"\n\t}\n\treturn rc.Env"},
	} {
		if err := replaceExact(moduleDir, patch.file, patch.before, patch.after); err != nil {
			return err
		}
	}
	if err := os.WriteFile(filepath.Join(moduleDir, "pkg", "model", "ferry_matrix.go"), []byte(orderedMatrixSource), 0600); err != nil {
		return err
	}
	return patchReusableMatrices(moduleDir)
}

// Preserve the declared axis order; ranging over Go maps randomizes plans.
const orderedMatrixSource = `package model

import "gopkg.in/yaml.v3"

func orderedMatrixProduct(values map[string][]interface{}, node yaml.Node) []map[string]interface{} {
    result := []map[string]interface{}{{}}
    for i := 0; i+1 < len(node.Content); i += 2 {
        key := node.Content[i].Value
        if key == "include" || key == "exclude" { continue }
        next := []map[string]interface{}{}
        for _, row := range result {
            for _, value := range values[key] {
                clone := map[string]interface{}{}
                for k,v := range row { clone[k] = v }
                clone[key] = value
                next = append(next,clone)
            }
        }
        result = next
    }
    return result
}
`

func replaceExact(moduleDir, file, before, after string) error {
	path := filepath.Join(moduleDir, "pkg", filepath.FromSlash(file))
	data, err := os.ReadFile(path)
	if err != nil {
		return err
	}
	if strings.Count(string(data), before) != 1 {
		return fmt.Errorf("act compatibility patch no longer applies: %s: %q", file, before)
	}
	return os.WriteFile(path, []byte(strings.Replace(string(data), before, after, 1)), 0600)
}
