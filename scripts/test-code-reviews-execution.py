#!/usr/bin/env python3
"""Actual private Git checkout, Actions/JUnit and HTTP model orchestration.

Reuses the owned SSH/HostAgent fixture in test-pipelines.py. The model is a
local contract server; this does not claim acceptance with a commercial model.
"""
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import threading
import time


def accept(context):
    api, create, check = (context[k] for k in ('api', 'create', 'check'))
    root = '/workspaces/' + context['workspace_ops']
    secret = 'review-model-fixture-key'
    model_calls = []
    filename = context.get('repository_file', 'payload.txt')
    content = context.get('repository_content', 'local-private-repository')

    class Model(BaseHTTPRequestHandler):
        def do_POST(self):
            assert self.headers.get('Authorization') == 'Bearer ' + secret
            body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
            captured = json.loads(body['messages'][1]['content'])
            assert captured['head_sha'] == context['commit']
            assert any(f['file'] == filename for f in captured['files'])
            model_calls.append(captured)
            finding = dict(severity='info', category='testing', file=filename, line=1,
                           message='Fixture coverage observation', suggestion='Add a fixture test',
                           evidence=content.splitlines()[0])
            raw = json.dumps({'choices': [{'finish_reason': 'stop', 'message': {
                'content': json.dumps({'findings': [finding]})}}]}).encode()
            self.send_response(200)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(raw)))
            self.end_headers()
            self.wfile.write(raw)

        def log_message(self, *args):
            pass

    receiver = ThreadingHTTPServer(('127.0.0.1', 0), Model)
    threading.Thread(target=receiver.serve_forever, daemon=True).start()
    try:
        credential = create('credentials', {'name': 'Review model acceptance', 'kind': 'token', 'secret': secret})
        connection = create('code-connections', {'name': 'Review private Git', 'provider': 'git',
                            'credential_id': context['cred'], 'url': context['repo_url']})
        targets = [('HostAgent', context['runner'], True, False)]
        if context.get('ssh'):
            targets.append(('SSH', context['ssh'], False, False))
        targets.append(('failed tests with AI', context['runner'], True, True))
        for label, target, use_ai, failure in targets:
            junit = '<testsuite><testcase name="real-checkout">' + ('<failure message="fixture"/>' if failure else '') + '</testcase></testsuite>'
            test_command = (
                'test "$(git rev-parse HEAD)" = ' + context['commit'] + '; '
                'test "$(cat '+filename+')" = "'+content+'"; '
                "printf '%s' '"+junit+"' > junit.xml; "
                + ('exit 1' if failure else 'echo REVIEW_ACTUAL_CHECKOUT_OK'))
            if context.get('windows'):
                test_command = (
                    "if ((git rev-parse HEAD).Trim() -ne '"+context['commit']+"') { throw 'wrong immutable head' }; "
                    "if (-not (Test-Path '"+filename+"')) { throw 'checkout missing' }; "
                    "[IO.File]::WriteAllText((Join-Path $PWD 'junit.xml'), '"+junit+"'); "
                    + ('exit 1' if failure else 'Write-Output REVIEW_ACTUAL_CHECKOUT_OK'))
            body = dict(name='Review acceptance ' + label, description='', enabled=True, trigger='manual',
                        branch='main', repositories=[{'connection_id': connection, 'repository': context['repo_url']}],
                        tests_enabled=True, ai_enabled=use_ai, test_command=test_command, junit_path='junit.xml',
                        runner_kind='host', runner_ref=target, runner_labels=['self-hosted'],
                        ai_endpoint=f'http://127.0.0.1:{receiver.server_port}/v1/chat/completions',
                        ai_model='local-contract-fixture', ai_credential_id=credential,
                        max_bytes=24000, ai_max_tokens=32768)
            review = create('code-reviews', body)
            kickoff = api(f'{root}/code-reviews/{review}/run', {
                'base_sha': context['commit'], 'head_sha': context['commit'], 'full_scan': True}, status=202)
            run_id = kickoff['items'][0]['id']
            deadline = time.monotonic() + 240
            while time.monotonic() < deadline:
                row = api(f'{root}/code-reviews/{review}/runs/{run_id}')
                if row['state'] in ('completed', 'failed', 'cancelled'):
                    break
                time.sleep(.5)
            else:
                raise AssertionError('review actual execution timed out')
            expected = 'failed' if failure else 'completed'
            check(label + ' review reaches expected terminal state', row['state'] == expected, json.dumps(row))
            check(label + ' actual test/JUnit result imported', row['test_state'] == ('failed' if failure else 'passed'))
            pipeline_run = api(f"{root}/pipeline-runs/{row['pipeline_run_id']}")
            logs = ''.join(v['chunk'] for v in api(f"{root}/pipeline-runs/{row['pipeline_run_id']}/logs?after=0")['items'])
            context['all_runs'].append(row['pipeline_run_id'])
            check(label + ' immutable head linked to real pipeline', pipeline_run['commit'] == context['commit'])
            check(label + ' model credential absent from repository logs and run metadata', secret not in logs + json.dumps(pipeline_run))
            if use_ai:
                findings = api(f'{root}/code-reviews/{review}/findings')['items']
                check(label + ' HTTP model findings imported independently of tests', row['ai_state'] == 'completed_findings' and len(findings) == 1)
        check('two actual model calls used captured immutable Git objects', len(model_calls) == 2)
        cancellation = dict(body, name='Review running cancellation acceptance',
                            ai_enabled=False, junit_path='', runner_ref=context['runner'],
                            test_command=('Write-Output REVIEW_CANCEL_STARTED; Start-Sleep -Seconds 90'
                                          if context.get('windows') else
                                          'echo REVIEW_CANCEL_STARTED; sleep 90'))
        review = create('code-reviews', cancellation)
        kickoff = api(f'{root}/code-reviews/{review}/run', {
            'head_sha': context['commit'], 'full_scan': True}, status=202)
        job = kickoff['items'][0]['id']
        deadline = time.monotonic() + 120
        while time.monotonic() < deadline:
            row = api(f'{root}/code-reviews/{review}/runs/{job}')
            pipeline = row.get('pipeline_run_id')
            if pipeline and str(pipeline) != '0':
                logs = ''.join(v['chunk'] for v in api(
                    f'{root}/pipeline-runs/{pipeline}/logs?after=0')['items'])
                if 'REVIEW_CANCEL_STARTED' in logs:
                    break
            time.sleep(.5)
        else:
            raise AssertionError('actual review test step did not start before cancellation')
        context['all_runs'].append(pipeline)
        api(f'{root}/code-reviews/{review}/runs/{job}/cancel', {})
        row = api(f'{root}/code-reviews/{review}/runs/{job}')
        check('running review acknowledges cancellation without claiming completion',
              row['state'] in ('cancelling', 'cancelled'))
        deadline = time.monotonic() + 150
        while time.monotonic() < deadline:
            row = api(f'{root}/code-reviews/{review}/runs/{job}')
            run = api(f'{root}/pipeline-runs/{pipeline}')
            if row['state'] == 'cancelled' and run['state'] == 'cancelled':
                break
            time.sleep(.5)
        else:
            raise AssertionError('review cancellation did not finish real pipeline cleanup')
        check('actual running review and linked pipeline finish cancelled',
              row['state'] == run['state'] == 'cancelled')
        check('cancellation does not invoke the model', len(model_calls) == 2)
    finally:
        receiver.shutdown()
        receiver.server_close()
