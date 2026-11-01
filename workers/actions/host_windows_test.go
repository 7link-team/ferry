package main

import (
	"bytes"
	"context"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
)

func windowsHostJob(t *testing.T, script, shell string, env, secrets map[string]string, extraSteps ...string) (JobResult, string, error) {
	t.Helper()
	workflow := `jobs:
  build:
    runs-on: self-hosted
    outputs:
      public: ${{ steps.emit.outputs.public }}
      hidden: ${{ steps.emit.outputs.hidden }}
      configured: ${{ steps.emit.outputs.configured }}
      multiline: ${{ steps.emit.outputs.multiline }}
    steps:
      - id: emit
` + shell + "        run: |\n          " + strings.ReplaceAll(script, "\n", "\n          ") + "\n" + strings.Join(extraSteps, "\n")
	var logs bytes.Buffer
	// Exercise both the repository and the sibling action cache under spaces.
	root := filepath.Join(repository(t), "parent with spaces", "work tree")
	if err := os.MkdirAll(root, 0700); err != nil {
		t.Fatal(err)
	}
	result, err := execute(context.Background(), Request{Workflow: workflow, Job: "build", Workdir: root, Env: env, Secrets: secrets, Platforms: map[string]string{"self-hosted": "-self-hosted"}}, &logs)
	return result, logs.String(), err
}

func TestWindowsDefaultShellWithoutPwsh(t *testing.T) {
	powershell, err := exec.LookPath("powershell")
	if err != nil {
		t.Fatal(err)
	}
	// Supply a PATH that contains Windows PowerShell 5.1, but cannot find pwsh.
	env := map[string]string{"PATH": filepath.Dir(powershell)}
	result, logs, err := windowsHostJob(t, `if ($PSVersionTable.PSVersion.Major -ne 5) { throw 'expected Windows PowerShell 5.1' }
if ((Get-ExecutionPolicy -Scope Process) -ne 'Unrestricted') { throw 'wrong script execution policy' }
'public=desktop-桌面' >> $env:GITHUB_OUTPUT`, "", env, nil)
	if err != nil || result.State != "success" || result.Outputs["public"] != "desktop-桌面" {
		t.Fatalf("default fallback failed: %#v %v %s", result, err, logs)
	}
	// An explicitly requested pwsh must fail rather than silently change editions.
	result, logs, err = windowsHostJob(t, "echo must-not-run", "        shell: pwsh\n", env, nil)
	if err == nil || result.State != "failure" || !strings.Contains(logs, "Cannot find: pwsh") {
		t.Fatalf("explicit missing pwsh did not fail: %#v %v %s", result, err, logs)
	}
}

func TestWindowsCommandFileEncodingsAndMasking(t *testing.T) {
	for _, encoding := range []string{"Unicode", "BigEndianUnicode", "UTF8", "UTF8NoBOM"} {
		t.Run(encoding, func(t *testing.T) {
			// Exercise CRLF, BOMs, multiline delimiters, env propagation and
			// masking together. The token is assembled at runtime, not in logs.
			script := `$token = 'runtime-sensitive-' + (900 + 23)
echo "::add-mask::$token"
echo $token
echo $env:TOKEN
$output = "public=visible` + "`r`n" + `hidden=$token` + "`r`n" + `configured=$env:TOKEN` + "`r`n" + `multiline<<END` + "`r`n" + `first` + "`r`n" + `second` + "`r`n" + `END` + "`r`n" + `"
$environment = "PUBLIC_ENV=propagated` + "`r`n" + `PRIVATE_ENV=$env:TOKEN` + "`r`n" + `"
`
			if encoding == "UTF8NoBOM" {
				script += `[IO.File]::WriteAllText($env:GITHUB_OUTPUT, $output, (New-Object Text.UTF8Encoding($false)))
[IO.File]::WriteAllText($env:GITHUB_ENV, $environment, (New-Object Text.UTF8Encoding($false)))`
			} else {
				script += fmt.Sprintf("$output | Out-File -NoNewline -Encoding %s $env:GITHUB_OUTPUT\n$environment | Out-File -NoNewline -Encoding %s $env:GITHUB_ENV", encoding, encoding)
			}
			extraSteps := `
      - run: |
          if ($env:PUBLIC_ENV -ne 'propagated') { throw 'env missing or corrupt' }
          if ($env:PRIVATE_ENV -ne $env:TOKEN) { throw 'secret env missing or corrupt' }
          echo $env:PRIVATE_ENV`
			secret := "秘密-configured-sensitive-531b9d"
			result, logs, err := windowsHostJob(t, script, "        shell: powershell\n", map[string]string{"TOKEN": secret}, map[string]string{"TOKEN": secret}, extraSteps)
			if err != nil || result.State != "success" || result.Outputs["public"] != "visible" || result.Outputs["multiline"] != "first\nsecond" {
				t.Fatalf("command file parsing failed: %#v %v %s", result, err, logs)
			}
			if _, retained := result.Outputs["hidden"]; retained || strings.Contains(logs, "runtime-sensitive-923") || strings.Contains(logs, secret) {
				t.Fatalf("secret escaped: %#v %s", result, logs)
			}
			if _, retained := result.Outputs["configured"]; retained || strings.Contains(logs, "configured-sensitive-531b9d") {
				t.Fatalf("configured Unicode secret escaped: %#v %s", result, logs)
			}
		})
	}
}

func TestWindowsHostProcessesHaveNoConsole(t *testing.T) {
	// Query the real child process, rather than merely inspecting a struct.
	script := `Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices; public class ConsoleProbe { [DllImport("kernel32.dll")] public static extern IntPtr GetConsoleWindow(); }'
if ([ConsoleProbe]::GetConsoleWindow() -ne [IntPtr]::Zero) { throw 'visible console allocated' }
'public=no-console' >> $env:GITHUB_OUTPUT`
	result, logs, err := windowsHostJob(t, script, "        shell: powershell\n", nil, nil)
	if err != nil || result.State != "success" || result.Outputs["public"] != "no-console" {
		t.Fatalf("host child console: %#v %v %s", result, err, logs)
	}
}

func TestWindowsMultilineSecretCRLF(t *testing.T) {
	secret := "秘密-first-sensitive-893af\r\nsecond-sensitive-778be"
	result, logs, err := windowsHostJob(t, `echo $env:TOKEN
'configured<<END_SECRET' >> $env:GITHUB_OUTPUT
$env:TOKEN >> $env:GITHUB_OUTPUT
'END_SECRET' >> $env:GITHUB_OUTPUT
'public=visible' >> $env:GITHUB_OUTPUT`, "", map[string]string{"TOKEN": secret}, map[string]string{"TOKEN": secret})
	if err != nil || result.State != "success" || result.Outputs["public"] != "visible" {
		t.Fatalf("multiline execution failed: %#v %v %s", result, err, logs)
	}
	if _, retained := result.Outputs["configured"]; retained {
		t.Fatal("multiline secret output retained")
	}
	for _, line := range []string{"first-sensitive-893af", "second-sensitive-778be"} {
		if strings.Contains(logs, line) {
			t.Fatal("multiline secret line escaped into logs")
		}
	}
}

func TestWindowsExplicitShells(t *testing.T) {
	for _, test := range []struct{ shell, script string }{
		{"powershell", `'public=visible' >> $env:GITHUB_OUTPUT`},
		{"cmd", `echo public=visible>> "%GITHUB_OUTPUT%"`},
		{"bash", `echo public=visible >> "$GITHUB_OUTPUT"`},
	} {
		t.Run(test.shell, func(t *testing.T) {
			result, logs, err := windowsHostJob(t, test.script, "        shell: "+test.shell+"\n", nil, nil)
			if err != nil || result.State != "success" || result.Outputs["public"] != "visible" {
				t.Fatalf("explicit shell: %#v %v %s", result, err, logs)
			}
		})
	}
}
