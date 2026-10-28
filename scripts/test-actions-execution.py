#!/usr/bin/env python3
"""Focused real Actions acceptance, called by the existing owned Git/SSH fixture."""
import json
import shutil


def accept(context):
    api, create, trigger, done, logs, check = (context[key] for key in ('api', 'create', 'trigger', 'done', 'logs', 'check'))
    command, work, name = (context[key] for key in ('command', 'work', 'name'))
    runner, ssh, credential, repo_url, root_dir = (context[key] for key in ('runner', 'ssh', 'cred', 'repo_url', 'root_dir'))
    second = work / 'second-source'
    shutil.copytree(work / 'source', second)
    (second / 'payload.txt').write_text('independent-second-repository\n')
    command(['git', '-C', str(second), 'add', 'payload.txt'])
    command(['git', '-C', str(second), '-c', 'user.name=fixture', '-c', 'user.email=fixture@example.invalid', 'commit', '-qm', 'second repository'])
    command(['git', 'clone', '--bare', str(second), str(work / 'second.git')])
    command(['docker', 'cp', str(work / 'second.git'), name + ':/srv/git/second.git'])
    command(['docker', 'exec', name, 'git', 'config', '--global', '--add', 'safe.directory', '/srv/git/second.git'])
    connection = create('code-connections', {'name': 'Node source acceptance', 'provider': 'git', 'credential_id': credential, 'url': repo_url})
    marker = root_dir + '/actions-parallel'
    # Both peers must reach the barrier before either can finish. A serial
    # scheduler fails with a real command exit, rather than merely running slow.
    def barrier(own, peer):
        return f'mkdir -p {marker}; touch {marker}/{own}; n=0; until test -f {marker}/{peer}; do n=$((n+1)); test "$n" -lt 120 || exit 19; sleep .25; done'
    workflow = {
        'name': 'Node scoped sources and real dependency scheduling', 'on': 'workflow_dispatch',
        'jobs': {
            'left': {'runs-on': 'self-hosted', 'outputs': {'proof': '${{ steps.barrier.outputs.proof }}'}, 'steps': [
                {'id': 'before', 'run': 'test ! -e first; test ! -e second; echo BEFORE_SOURCES'},
                {'id': 'first', 'uses': 'actions/checkout@v4'},
                {'id': 'between', 'run': "test ! -e second; test \"$(cat first/payload.txt)\" = local-private-repository; echo BETWEEN_SOURCES"},
                {'id': 'second', 'uses': 'actions/checkout@v4'},
                {'id': 'after', 'run': "test \"$(cat second/payload.txt)\" = independent-second-repository; test '${{ steps.first.outputs.commit }}' != '${{ steps.second.outputs.commit }}'; echo BOTH_SOURCES_OK"},
                {'id': 'barrier', 'run': barrier('left', 'right') + '; echo proof=left >> "$GITHUB_OUTPUT"'},
            ]},
            'right': {'runs-on': 'self-hosted', 'outputs': {'proof': '${{ steps.barrier.outputs.proof }}'}, 'steps': [
                {'id': 'barrier', 'run': barrier('right', 'left') + '; echo proof=right >> "$GITHUB_OUTPUT"'},
            ]},
            'join': {'runs-on': 'self-hosted', 'needs': ['left', 'right'], 'steps': [
                {'run': "test '${{ needs.left.outputs.proof }}' = left; test '${{ needs.right.outputs.proof }}' = right; echo PARALLEL_AND_SERIAL_OK"},
            ]},
        },
    }
    bindings = [{'job_id': 'left', 'step_id': step, 'connection_id': connection, 'repository_url': url, 'ref': 'main', 'path': step}
                for step, url in [('first', repo_url), ('second', repo_url.replace('/private.git', '/second.git'))]]
    body = {'name': 'Only node configuration', 'definition_format': 'github-actions', 'definition': json.dumps(workflow),
            'checkout_bindings': bindings, 'executor_bindings': [
                {'job_id': job, 'target_kind': 'host', 'target_ref': target, 'image': 'host'}
                for job, target in [('left', runner), ('right', runner), ('join', ssh)]], 'graph_layout': {}}
    pipeline = create('pipelines', body)
    saved = api('/workspaces/ops/pipelines/' + pipeline)
    check('Actions saves with node executors and no global host or repository', saved['target_ref'] == '0' and saved['repository_id'] == '0')
    check('Independent checkout bindings round trip without credentials', saved['checkout_bindings'] == bindings and context['secret'] not in json.dumps(saved))
    run = trigger(pipeline)['id']
    context['all_runs'].append(run)
    result = done(run, 180)
    rows = logs(run)
    text = ''.join(row['chunk'] for row in rows)
    check('Actual independent jobs pass mutual barrier and dependent SSH job waits for both', result['state'] == 'succeeded' and 'PARALLEL_AND_SERIAL_OK' in text, str(result.get('jobs')))
    if result['state'] != 'succeeded':
        raise AssertionError(text)
    check('Private independent repositories execute at their own ordered checkout nodes', all(marker in text for marker in ['BEFORE_SOURCES', 'BETWEEN_SOURCES', 'BOTH_SOURCES_OK']))
    sequence = [row['seq'] for row in rows]
    check('Parallel log sequences remain unique and ordered', sequence == sorted(set(sequence)))
    check('Node credential secret is absent from logs and run metadata', context['secret'] not in text + json.dumps(result))
    jobs = {job['key']: job for job in result['jobs']}
    check('Dependent job starts only after both source jobs finish', jobs['join']['started_at'] >= max(jobs['left']['finished_at'], jobs['right']['finished_at']))

    # Cancel two concurrently running host transports and verify descendants,
    # not just database state, are gone before the run reaches terminal state.
    import os
    import re
    import subprocess
    holds = {job: {'runs-on': 'self-hosted', 'steps': [{'run': 'sleep 120 & child=$!; echo PARALLEL_CHILD_PID=$child; wait'}]}
             for job in ['runner_hold', 'ssh_hold']}
    cancel_body = {'name': 'Cancel both running executors', 'definition_format': 'github-actions',
                   'definition': json.dumps({'on': 'workflow_dispatch', 'jobs': holds}),
                   'executor_bindings': [{'job_id': job, 'target_kind': 'host', 'target_ref': target, 'image': 'host'}
                                         for job, target in [('runner_hold', runner), ('ssh_hold', ssh)]]}
    cancel_pipeline = create('pipelines', cancel_body)
    cancelled_run = trigger(cancel_pipeline)['id']
    context['all_runs'].append(cancelled_run)

    def children():
        text = ''.join(row['chunk'] for row in logs(cancelled_run))
        matches = dict(re.findall(r'\[(runner_hold|ssh_hold)\] PARALLEL_CHILD_PID=(\d+)', text))
        return matches if len(matches) == 2 else None

    pids = context['eventually'](children, 60)
    api('/workspaces/ops/pipeline-runs/' + cancelled_run + '/actions/cancel', method='POST')
    cancelled = done(cancelled_run, 60)
    check('Cancellation waits for both concurrently running executors', cancelled['state'] == 'cancelled' and all(job['state'] not in ['running', 'queued'] for job in cancelled['jobs']))
    try:
        os.kill(int(pids['runner_hold']), 0)
        runner_alive = True
    except ProcessLookupError:
        runner_alive = False
    ssh_alive = subprocess.run(['docker', 'exec', name, 'sh', '-c', f"kill -0 {pids['ssh_hold']} 2>/dev/null"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0
    check('Cancellation stops descendants on both runner and SSH', not runner_alive and not ssh_alive)
