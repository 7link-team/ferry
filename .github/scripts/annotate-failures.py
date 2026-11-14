"""Turn a failed harness log into error annotations: each distinct FAIL line or
traceback with the lines before it. Annotations are readable through the public
API, unlike job logs, so failures can be diagnosed without signing in."""
import sys
import subprocess
from pathlib import Path


exit_code = 0
if len(sys.argv) > 1 and sys.argv[1] == '--run':
    command = sys.argv[2:]
    if not command:
        raise SystemExit('usage: annotate-failures.py --run COMMAND [ARG ...]')
    # Browser steps run before the repeated harness loop. Keep their diagnostics
    # in the same artifact directory and expose failures through public annotations.
    name = Path(command[1] if len(command) > 1 else command[0]).stem
    log_path = Path('harness-logs') / f'{name}.log'
    log_path.parent.mkdir(parents=True, exist_ok=True)
    with log_path.open('w', encoding='utf-8') as log:
        process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                   text=True, encoding='utf-8', errors='replace')
        for line in process.stdout:
            print(line, end='', flush=True)
            log.write(line)
        exit_code = process.wait()
    if exit_code < 0:
        exit_code = 128 - exit_code
    if exit_code == 0:
        raise SystemExit(0)
    sys.argv[1] = str(log_path)


def keep(line):
    # Drop the engine's repository-less warnings, but never a FAIL line: those
    # quote command output that may contain them.
    failing = line.lstrip().startswith('FAIL') or 'AssertionError' in line
    return failing or 'unable to get git' not in line


lines = [line for line in open(sys.argv[1], errors='replace').read().splitlines() if keep(line)]
seen, blocks = set(), []
for index, line in enumerate(lines):
    if line.lstrip().startswith('FAIL') or line.startswith('Traceback') or 'AssertionError' in line or 'panicked at' in line:
        key = line.strip()[:80]
        if key in seen:
            continue
        seen.add(key)
        blocks.append('\n'.join(lines[max(0, index - 20):index + 4]))
if not blocks:
    blocks.append('\n'.join(lines[-60:]))
for number, block in enumerate(blocks[:9]):
    text = block[-6000:].replace('%', '%25').replace('\r', '').replace('\n', '%0A')
    print(f'::error title=failure {number + 1}::{text}')
raise SystemExit(exit_code)
