"""Turn a failed harness log into error annotations: each distinct FAIL line or
traceback with the lines before it. Annotations are readable through the public
API, unlike job logs, so failures can be diagnosed without signing in."""
import sys


def keep(line):
    # Drop the engine's repository-less warnings, but never a FAIL line: those
    # quote command output that may contain them.
    failing = line.lstrip().startswith('FAIL') or 'AssertionError' in line
    return failing or 'unable to get git' not in line


lines = [line for line in open(sys.argv[1], errors='replace').read().splitlines() if keep(line)]
seen, blocks = set(), []
for index, line in enumerate(lines):
    if line.lstrip().startswith('FAIL') or line.startswith('Traceback') or 'AssertionError' in line:
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
