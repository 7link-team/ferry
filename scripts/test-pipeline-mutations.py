#!/usr/bin/env python3
"""Run two real-host mutation tests, always restore source, then verify restoration.
Run alone: this temporarily edits pipeline_runs.rs and rebuilds the binary.
"""
from pathlib import Path
import subprocess
import tempfile

source = Path('crates/server/src/pipeline_runs.rs')
original = source.read_text()
output = Path(tempfile.mkdtemp(prefix='pipeline-mutations-'))


def run(label, expected, evidence=None):
    log = output / (label + '.log')
    with log.open('w') as stream:
        result = subprocess.run(['./scripts/test-pipelines.sh'], stdout=stream, stderr=subprocess.STDOUT)
    text = log.read_text()
    print(f'{label}: exit={result.returncode}; log={log}', flush=True)
    for line in text.splitlines():
        if 'FAIL ' in line or ' passed, ' in line:
            print(line, flush=True)
    assert result.returncode == expected, f'{label}: unexpected exit code; see {log}'
    if evidence:
        assert evidence in text, f'{label}: failed for wrong reason; see {log}'


try:
    run('baseline', 0)
    needle = 'if state != "succeeded" {'
    assert original.count(needle) == 1
    source.write_text(original.replace(needle, 'if state == "cancelled" {'))
    run('continue-after-failure', 1, 'FAIL 4 runner failure stops next step')
    source.write_text(original)
    run('restored-after-failure', 0)
    needle = 'let command = format!("sh {}", quote(&format!("{private}/step")));'
    assert original.count(needle) == 1
    mutant = '''let command = format!("sh -c {}", quote(&format!(
        "export LEAKED_PASSWORD={}; sleep 1; sh {}",
        quote(credential.map(|c|c.2.as_str()).unwrap_or("")), quote(&format!("{private}/step")))));'''
    source.write_text(original.replace(needle, mutant))
    run('credential-in-argv', 1, 'FAIL 7 runner target ps contains no credential plaintext')
finally:
    source.write_text(original)
    # Also rebuild and verify the restored code after an unexpected test failure.
    run('final-restored', 0)
    assert source.read_text() == original
print('Both live mutations detected; source restored; restored live suite passed.', flush=True)
