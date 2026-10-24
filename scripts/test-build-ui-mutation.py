#!/usr/bin/env python3
"""Prove the component tests reject polling after terminal state; always restore."""
from pathlib import Path
import subprocess

source = Path('src/components/PipelineRunDetail.tsx')
original = source.read_bytes()
needle = b'finished = !isActive(next.state) && page.items.length < 200;'
assert original.count(needle) == 1
command = ['npm', 'test', '--', 'src/components/PipelineRunDetail.test.tsx']

def run(label):
    result = subprocess.run(command, check=False)
    print(f'{label}: exit={result.returncode}', flush=True)
    return result.returncode

assert run('baseline') == 0
try:
    source.write_bytes(original.replace(needle, b'finished = false; // MUTATION: terminal polling continues'))
    assert run('mutant') != 0, 'Mutation survived: missing terminal polling coverage'
finally:
    source.write_bytes(original)
assert source.read_bytes() == original
assert run('restored') == 0
print('PASS: mutation detected, original bytes restored, restored tests pass')
