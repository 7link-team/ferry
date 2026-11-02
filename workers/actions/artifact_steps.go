package main

import (
	"maps"
	"os"
	"runtime"
	"strings"

	"github.com/nektos/act/pkg/model"
	"gopkg.in/yaml.v3"
)

// act calls this for host steps before selecting their executor, including
// steps inside composites and reusable workflows. Each leg owns its model.
func rewriteBuiltinArtifactStep(step *model.Step) error {
	operation := ""
	for _, candidate := range []string{"upload", "download"} {
		prefix := "actions/" + candidate + "-artifact@v4"
		if step.Uses == prefix || strings.HasPrefix(step.Uses, prefix+".") {
			operation = candidate
			break
		}
	}
	if operation == "" {
		return nil
	}
	executable, err := os.Executable()
	if err != nil {
		return err
	}
	// Keep user values out of shell source: act evaluates these with the same
	// step expression evaluator as the original action's with inputs.
	env := maps.Clone(step.Environment())
	if env == nil {
		env = map[string]string{}
	}
	for key, value := range step.With {
		env["INPUT_FERRY_ARTIFACT_"+strings.ToUpper(strings.ReplaceAll(key, "-", "_"))] = value
	}
	env["FERRY_ARTIFACT_EXECUTABLE"] = executable
	var node yaml.Node
	if err := node.Encode(env); err != nil {
		return err
	}
	step.Env = node
	if step.Name == "" {
		step.Name = step.Uses
	}
	step.Uses, step.With = "", nil
	// Actions run in the workspace, independent of defaults.run settings.
	if step.WorkingDirectory == "" {
		step.WorkingDirectory = "${{ github.workspace }}"
	}
	step.Shell = "sh"
	step.Run = `"$FERRY_ARTIFACT_EXECUTABLE" artifact ` + operation
	if runtime.GOOS == "windows" {
		step.Shell = "cmd"
		step.Run = `@"%FERRY_ARTIFACT_EXECUTABLE%" artifact ` + operation + "\r\n@exit /b %errorlevel%"
	}
	return nil
}
