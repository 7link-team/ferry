package main

import (
	"os"
	"path/filepath"
)

// Reusable workflows enter act's planner rather than the worker's selected-job
// scheduler. Use the same isolation boundary here: snapshot the complete model
// before scheduling, run private copies, and merge only completed results.
func patchReusableMatrices(moduleDir string) error {
	for _, patch := range []struct{ file, before, after string }{
		{"model/workflow.go", "TimeoutMinutes string                    `yaml:\"timeout-minutes\"`", "RawContinueOnError string `yaml:\"continue-on-error\"`\n\tTimeoutMinutes string                    `yaml:\"timeout-minutes\"`"},
		{"runner/run_context.go", "StrategyJobTotal int", "FerryIsolated bool\n\tStrategyJobTotal int"},
		{"runner/job_executor.go", "if rc.caller != nil {\n\t\t// set reusable workflow job result", "if rc.caller != nil && !rc.FerryIsolated {\n\t\t// set reusable workflow job result"},
		{"runner/job_executor.go", "if rc.caller != nil {\n\t\t// map outputs for reusable workflows", "if rc.caller != nil && !rc.FerryIsolated {\n\t\t// map outputs for reusable workflows"},
		{"runner/runner.go", "\tmaxJobNameLen := 0\n", ""},
		{"runner/runner.go", "\t\t\t\tstageExecutor := make([]common.Executor, 0)\n", ""},
		{"runner/runner.go", "strategyRc.NewExpressionEvaluator(ctx).EvaluateYamlNode(ctx, &job.Strategy.RawMatrix)", "EvaluateMatrix(ctx, strategyRc.NewExpressionEvaluator(ctx), &job.Strategy.RawMatrix)"},
		{"runner/runner.go", upstreamMatrixSchedule, "\t\t\t\tpipeline = append(pipeline, runner.ferryMatrixExecutor(run, matrixes))"},
	} {
		if err := replaceExact(moduleDir, patch.file, patch.before, patch.after); err != nil {
			return err
		}
	}
	return os.WriteFile(filepath.Join(moduleDir, "pkg", "runner", "ferry_matrix.go"), []byte(reusableMatrixSource), 0600)
}

const upstreamMatrixSchedule = `				maxParallel := 4
				if job.Strategy != nil {
					maxParallel = job.Strategy.MaxParallel
				}

				if len(matrixes) < maxParallel {
					maxParallel = len(matrixes)
				}

				for i, matrix := range matrixes {
					rc := runner.newRunContext(ctx, run, matrix)
					rc.JobName = rc.Name
					if len(matrixes) > 1 {
						rc.Name = fmt.Sprintf("%s-%d", rc.Name, i+1)
					}
					if len(rc.String()) > maxJobNameLen {
						maxJobNameLen = len(rc.String())
					}
					stageExecutor = append(stageExecutor, func(ctx context.Context) error {
						jobName := fmt.Sprintf("%-*s", maxJobNameLen, rc.String())
						executor, err := rc.Executor()

						if err != nil {
							return err
						}

						return executor(common.WithJobErrorContainer(WithJobLogger(ctx, rc.Run.JobID, jobName, rc.Config, &rc.Masks, matrix)))
					})
				}
				pipeline = append(pipeline, common.NewParallelExecutor(maxParallel, stageExecutor...))`

const reusableMatrixSource = `package runner

import (
    "context"
    "encoding/json"
    "fmt"
    "maps"
    "path/filepath"
    "regexp"
    "strconv"
    "sync"
    "time"

    "github.com/nektos/act/pkg/common"
    "github.com/nektos/act/pkg/exprparser"
    "github.com/nektos/act/pkg/model"
    "gopkg.in/yaml.v3"
)

// Protect publication to act's original workflow and reusable caller models.
// Running contexts never reference those mutable models.
var ferryModelMutex sync.Mutex

var ferryFromJSON=regexp.MustCompile(` + "`" + `(?is)^\s*\$\{\{\s*fromJSON\((.*)\)\s*\}\}\s*$` + "`" + `)

// Preserve object member order for whole-matrix fromJSON expressions. act's
// ordinary map conversion sorts keys and loses the strategy's expansion order.
func EvaluateMatrix(ctx context.Context, evaluator ExpressionEvaluator, node *yaml.Node) error {
    if node.Kind==yaml.ScalarNode {
        if match:=ferryFromJSON.FindStringSubmatch(node.Value); match!=nil {
            raw:=evaluator.Interpolate(ctx,"` + "${{" + ` "+match[1]+" ` + "}}" + `")
            if !json.Valid([]byte(raw)) { return fmt.Errorf("strategy.matrix fromJSON must resolve to valid JSON") }
            var document yaml.Node
            if err:=yaml.Unmarshal([]byte(raw),&document); err!=nil { return err }
            if len(document.Content)!=1 || document.Content[0].Kind!=yaml.MappingNode { return fmt.Errorf("strategy.matrix must resolve to an object") }
            *node=*document.Content[0]
            return evaluator.EvaluateYamlNode(ctx,node)
        }
    }
    return evaluator.EvaluateYamlNode(ctx,node)
}

func (r *runnerImpl) ferryMatrixExecutor(run *model.Run, matrices []map[string]interface{}) common.Executor {
    return func(ctx context.Context) error {
        ferryModelMutex.Lock()
        template, err := json.Marshal(run.Workflow)
        ferryModelMutex.Unlock()
        if err != nil { return err }
        strategyWorkflow := &model.Workflow{}
        if err=json.Unmarshal(template,strategyWorkflow); err!=nil { return err }
        strategyRC := r.newRunContext(ctx,&model.Run{Workflow:strategyWorkflow,JobID:run.JobID},nil)
        parallel, failFast := len(matrices), true
        if strategy := strategyRC.Run.Job().Strategy; strategy != nil {
            if strategy.MaxParallelString != "" {
                parallel, err = strconv.Atoi(strategyRC.ExprEval.Interpolate(ctx,strategy.MaxParallelString))
                if err != nil || parallel < 1 { return fmt.Errorf("strategy.max-parallel must resolve to a positive integer") }
            }
            if strategy.FailFastString != "" {
                failFast, err = EvalBool(ctx,strategyRC.ExprEval,strategy.FailFastString,exprparser.DefaultStatusCheckNone)
                if err != nil { return err }
            }
        }
        declaredParallel := parallel
        if parallel > len(matrices) { parallel = len(matrices) }
        if parallel < 1 { return nil }
        group, stop := context.WithCancel(ctx)
        defer stop()
        // Do not use group as the execution context: cancellation is a status
        // signal until the shared cleanup grace expires.
        var mutex sync.Mutex
        next := 0
        outputs := map[string]string{}
        state := "success"
        var firstErr error
        var workers sync.WaitGroup
        for slot:=0; slot<parallel; slot++ {
            workers.Add(1)
            go func() {
                defer workers.Done()
                for {
                    mutex.Lock()
                    if next == len(matrices) || group.Err()!=nil { mutex.Unlock(); return }
                    index:=next; next++; mutex.Unlock()
                    workflow := &model.Workflow{}
                    e := json.Unmarshal(template,workflow)
                    if e != nil { mutex.Lock(); firstErr=e; state="failure"; mutex.Unlock(); stop(); return }
                    isolated := &model.Run{Workflow:workflow,JobID:run.JobID}
                    config := *r.config
                    config.Env=maps.Clone(config.Env); config.Secrets=maps.Clone(config.Secrets)
                    config.ActionCacheDir=filepath.Join(config.ActionCacheDir,"reusable-"+run.JobID,strconv.Itoa(index))
                    child := *r
                    child.config=&config
                    rc := child.newRunContext(ctx,isolated,matrices[index])
                    rc.FerryIsolated=true
                    rc.StrategyJobTotal=len(matrices); rc.StrategyJobIndex=index
                    if isolated.Job().Strategy!=nil { isolated.Job().Strategy.MaxParallel=declaredParallel; isolated.Job().Strategy.FailFast=failFast }
                    rc.ExprEval=rc.NewExpressionEvaluator(ctx)
                    tolerated:=false
                    if raw:=isolated.Job().RawContinueOnError; raw!="" { tolerated,e=EvalBool(ctx,rc.ExprEval,raw,exprparser.DefaultStatusCheckNone) }
                    minutes:=360
                    if raw:=isolated.Job().TimeoutMinutes; e==nil && raw!="" {
                        minutes,e=strconv.Atoi(rc.ExprEval.Interpolate(ctx,raw))
                        if e==nil && (minutes<1 || int64(minutes)>int64((1<<63-1)/time.Minute)) { e=fmt.Errorf("job timeout-minutes must resolve to a positive duration") }
                    }
                    jobCtx,cancel:=context.WithTimeout(group,time.Duration(minutes)*time.Minute)
                    execCtx,finish:=ferryCleanupContext(ctx,jobCtx)
                    rc.Cancelled.Store(common.JobCancelContext(execCtx).Err()!=nil)
                    rc.ExprEval=rc.NewExpressionEvaluator(execCtx)
                    rc.JobName=rc.Name
                    execCtx=common.WithJobErrorContainer(WithJobLogger(execCtx,run.JobID,rc.Name,&config,&rc.Masks,matrices[index]))
                    if e==nil {
                        var executor common.Executor
                        executor,e=rc.Executor()
                        if e==nil { e=executor(execCtx) }
                    }
                    legFailed:=isolated.Job().Result=="failure" || e!=nil || jobCtx.Err()==context.DeadlineExceeded
                    finish(); cancel()
                    mutex.Lock()
                    for key,value:=range isolated.Job().Outputs { outputs[key]=value }
                    if legFailed && !tolerated {
                        state="failure"
                        if firstErr==nil { firstErr=e; if firstErr==nil { firstErr=fmt.Errorf("job %s failed",run.JobID) } }
                        if failFast { stop() }
                    }
                    mutex.Unlock()
                }
            }()
        }
        workers.Wait()
        ferryModelMutex.Lock()
        defer ferryModelMutex.Unlock()
        run.Job().Outputs=outputs; run.Job().Result=state
        aggregate:=r.newRunContext(ctx,run,nil)
        setJobOutputs(ctx,aggregate)
        if aggregate.caller!=nil && (state=="failure" || aggregate.caller.runContext.Run.Job().Result=="") { aggregate.caller.runContext.result(state) }
        return firstErr
    }
}

func ferryCleanupContext(parent, signal context.Context) (context.Context,func()) {
    ctx,cancel:=context.WithCancel(context.WithoutCancel(parent))
    joined,stop:=context.WithCancel(signal)
    stopParent:=func() bool { return false }
    if upstream:=common.JobCancelContext(parent); upstream!=nil {
        if upstream.Err()!=nil { stop() } else { stopParent=context.AfterFunc(upstream,stop) }
    }
    ctx=common.WithJobCancelContext(ctx,joined)
    done:=make(chan struct{})
    go func(){ select { case <-joined.Done():
        timer:=time.NewTimer(30*time.Second); defer timer.Stop()
        select { case <-timer.C:cancel(); case <-parent.Done():cancel(); case <-done: }
        case <-parent.Done():cancel(); case <-done: } }()
    return ctx,func(){ close(done); stopParent(); stop(); cancel() }
}

var ferryOutputReference=regexp.MustCompile(` + "`" + `^\s*\$\{\{\s*steps\.([a-zA-Z_][a-zA-Z0-9_-]*)\.outputs\.([a-zA-Z_][a-zA-Z0-9_-]*)\s*\}\}\s*$` + "`" + `)

// An undefined step output is absent. An explicitly emitted empty output is
// present and must overwrite a previous matrix leg's value on completion.
func ferryMissingOutput(expression string, steps map[string]*model.StepResult) bool {
    match:=ferryOutputReference.FindStringSubmatch(expression)
    if match==nil { return false }
    step:=steps[match[1]]
    if step==nil { return true }
    _,present:=step.Outputs[match[2]]
    return !present
}
`
