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

    mixed_workflow = json.loads(json.dumps(workflow))
    mixed_workflow['jobs']['right']['steps'].insert(0, {'run': 'test "$(cat payload.txt)" = local-private-repository; echo LEGACY_SOURCE_PRESERVED'})
    mixed_body = dict(body, name='Mixed legacy and node sources', repository_id=context['repo_id'], definition=json.dumps(mixed_workflow))
    mixed_pipeline = create('pipelines', mixed_body)
    mixed_run = trigger(mixed_pipeline)['id']
    context['all_runs'].append(mixed_run)
    mixed_result = done(mixed_run, 180)
    mixed_text = ''.join(row['chunk'] for row in logs(mixed_run))
    check('A bound source job does not suppress a sibling legacy repository checkout', mixed_result['state'] == 'succeeded' and 'LEGACY_SOURCE_PRESERVED' in mixed_text and 'BOTH_SOURCES_OK' in mixed_text)
    if mixed_result['state'] != 'succeeded':
        raise AssertionError(mixed_text)

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

    artifact_marker = root_dir + '/artifact-parallel'
    artifact_jobs = {}
    for own, peer in [('left', 'right'), ('right', 'left')]:
        artifact_jobs[own] = {'runs-on': 'self-hosted', 'steps': [
            {'run': f'mkdir -p {artifact_marker}; touch {artifact_marker}/{own}; n=0; until test -f {artifact_marker}/{peer}; do n=$((n+1)); test "$n" -lt 120 || exit 19; sleep .25; done; echo {own} > payload.txt'},
            {'uses': 'actions/upload-artifact@v4', 'with': {'name': 'duplicate', 'path': 'payload.txt', 'if-no-files-found': 'error'}},
        ]}
    artifact_pipeline = create('pipelines', {
        'name': 'Concurrent duplicate artifact', 'definition_format': 'github-actions',
        'definition': json.dumps({'on': 'workflow_dispatch', 'jobs': artifact_jobs}),
        'executor_bindings': [{'job_id': job, 'target_kind': 'host', 'target_ref': runner, 'image': 'host'} for job in artifact_jobs],
    })
    artifact_run = trigger(artifact_pipeline)['id']
    context['all_runs'].append(artifact_run)
    artifact_result = done(artifact_run, 180)
    check('Parallel duplicate artifact names fail explicitly without last-writer wins', artifact_result['state'] == 'failed' and len(artifact_result['artifacts']) == 1 and sorted(job['state'] for job in artifact_result['jobs']) == ['failed', 'succeeded'], str(artifact_result.get('jobs')))
    if len(artifact_result['artifacts']) != 1:
        raise AssertionError('duplicate artifact ownership was published')
    import io
    import urllib.request
    import zipfile
    archive = artifact_result['artifacts'][0]
    request = urllib.request.Request(context['base'] + '/workspaces/ops/pipeline-artifacts/' + archive['id'] + '/download')
    with context['opener'].open(request, timeout=15) as response:
        downloaded = response.read()
    with zipfile.ZipFile(io.BytesIO(downloaded)) as zipped:
        payload = zipped.read('payload.txt').decode().strip()
    winner = next(job['key'] for job in artifact_result['jobs'] if job['state'] == 'succeeded')
    check('The winning artifact bytes match the successful owning job', payload == winner)

    for label, display_names in [('same', ['Build', 'Build']), ('normalized', ['编译', '测试'])]:
        docker_jobs = {job: {'name': display_names[index], 'runs-on': 'container', 'steps': [
            {'run': f"test '${{{{ github.job }}}}' = {job}; echo CONTAINER_READY_{job}; sleep 12; echo CONTAINER_FINISHED_{job}"},
        ]} for index, job in enumerate(['left', 'right'])}
        docker_pipeline = create('pipelines', {
            'name': 'Container identity ' + label, 'definition_format': 'github-actions',
            'definition': json.dumps({'name': 'Shared display workflow', 'on': 'workflow_dispatch', 'jobs': docker_jobs}),
            'executor_bindings': [{'job_id': job, 'target_kind': 'host', 'target_ref': runner, 'image': 'pipeline-live-test'} for job in docker_jobs],
        })
        docker_run = trigger(docker_pipeline)['id']
        context['all_runs'].append(docker_run)
        docker_result = done(docker_run, 180)
        docker_text = ''.join(row['chunk'] for row in logs(docker_run))
        check(label + ' display names use isolated concurrent Docker containers', docker_result['state'] == 'succeeded' and all('CONTAINER_FINISHED_' + job in docker_text for job in docker_jobs), str(docker_result.get('jobs')))
        if docker_result['state'] != 'succeeded':
            raise AssertionError(docker_text)
