package main

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

// Applied on every build host, including when cross-compiling Windows workers.
// The generated code changes only Windows host behavior.
func patchWindowsHost(moduleDir string) error {
	for _, patch := range []struct{ file, before, after string }{
		{"container/util_windows.go", "CreationFlags: syscall.CREATE_NEW_PROCESS_GROUP", "HideWindow: true, CreationFlags: syscall.CREATE_NEW_PROCESS_GROUP | 0x08000000 /* CREATE_NO_WINDOW */"},
		{"runner/step_run.go", `script = fmt.Sprintf("%s\n%s\n%s", runPrepend, script, runAppend)`, `if runtime.GOOS == "windows" {
		if _, host := sr.RunContext.JobContainer.(*container.HostEnvironment); host {
			switch step.Shell {
			case "pwsh", "powershell":
			// Match the runner's invocation, including on stock Windows with
			// a Restricted script execution policy.
			scCmd = step.Shell + " -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Unrestricted -Command . '{0}'"
			// Windows PowerShell otherwise emits OEM bytes into the UTF-8 log
			// stream, corrupting Unicode secrets before they can be masked.
			runPrepend += "\n[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)\n$OutputEncoding = [Console]::OutputEncoding"
			// PowerShell 5.1 requires a BOM to recognize UTF-8 script literals.
			runPrepend = "\ufeff" + runPrepend
			case "bash", "sh":
				// Windows passes the raw command line, so quote the script path
				// just as the runner does (profiles/workspaces can contain spaces).
				scCmd = strings.Replace(scCmd, "{0}", "\"{0}\"", 1)
			}
		}
	}
	script = fmt.Sprintf("%s\n%s\n%s", runPrepend, script, runAppend)`},
		{"container/parse_env_file.go", "s := bufio.NewScanner(reader)", `decoded, err := commandFileReader(reader)
		if err != nil { return err }
		s := bufio.NewScanner(decoded)`},
	} {
		source := filepath.Join(moduleDir, "pkg", filepath.FromSlash(patch.file))
		data, err := os.ReadFile(source)
		if err != nil {
			return err
		}
		if strings.Count(string(data), patch.before) != 1 {
			return fmt.Errorf("act Windows host patch no longer applies: %s: %q", patch.file, patch.before)
		}
		if err := os.WriteFile(source, []byte(strings.Replace(string(data), patch.before, patch.after, 1)), 0600); err != nil {
			return err
		}
	}
	return os.WriteFile(filepath.Join(moduleDir, "pkg", "container", "ferry_command_file.go"), []byte(commandFileSource), 0600)
}

const commandFileSource = `package container

import (
	"bufio"
	"encoding/binary"
	"fmt"
	"io"
	"runtime"
	"strings"
	"unicode/utf16"
)

// PowerShell 5.1 redirection writes UTF-16LE with a BOM. Decode before
// parsing names, multiline delimiters and values (and before secret checks).
// UTF-8, UTF-8 BOM and CRLF remain supported. Unix readers are untouched.
func commandFileReader(reader io.Reader) (io.Reader, error) {
	if runtime.GOOS != "windows" {
		return reader, nil
	}
	buffered := bufio.NewReader(reader)
	bom, err := buffered.Peek(2)
	if err != nil || !(bom[0] == 0xff && bom[1] == 0xfe || bom[0] == 0xfe && bom[1] == 0xff) {
		return buffered, nil
	}
	var order binary.ByteOrder = binary.LittleEndian
	if bom[0] == 0xfe { order = binary.BigEndian }
	_, _ = buffered.Discard(2)
	data, err := io.ReadAll(buffered)
	if err != nil { return nil, err }
	if len(data)%2 != 0 {
		return nil, fmt.Errorf("invalid UTF-16 command file: odd byte count")
	}
	units := make([]uint16, len(data)/2)
	for i := range units { units[i] = order.Uint16(data[i*2:]) }
	return strings.NewReader(string(utf16.Decode(units))), nil
}
`
