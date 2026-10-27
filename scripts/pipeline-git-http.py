#!/usr/bin/env python3
"""Local authenticated smart HTTP Git fixture; no external repository required."""
import base64
import http.server
import json
import os
import subprocess
import sys
from urllib.parse import urlsplit, parse_qs

root, config, port = sys.argv[1:]
credentials = json.load(open(config))
expected = 'Basic ' + base64.b64encode(
    (credentials['username'] + ':' + credentials['secret']).encode()).decode()


provider_expected = 'Basic ' + base64.b64encode(
    ('connected-git-user:' + credentials['secret']).encode()).decode()


class Git(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass  # Never log Authorization.

    def do_GET(self):
        self.serve()

    def do_POST(self):
        self.serve()

    def serve(self):
        url = urlsplit(self.path)
        if url.path in ['/user','/user/repos','/projects'] or url.path.startswith('/oapi/v1/codeup/'):
            token = parse_qs(url.query).get('access_token',[''])[0] or self.headers.get('PRIVATE-TOKEN') or self.headers.get('x-yunxiao-token') or self.headers.get('Authorization','').removeprefix('Bearer ').removeprefix('token ')
            if token != credentials['secret']:
                self.send_response(401);self.end_headers();return
            if url.path=='/user': value={'login':'connected-git-user','username':'connected-git-user'}
            else:value=[{'name':'private','full_name':'team/private','clone_url':f'http://127.0.0.1:{port}/private.git','default_branch':'main'}]
            payload=json.dumps(value).encode();self.send_response(200);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(payload)));self.end_headers();self.wfile.write(payload);return
        if self.headers.get('Authorization') not in [expected, provider_expected]:
            self.send_response(401)
            self.send_header('WWW-Authenticate', 'Basic realm="local-git"')
            self.end_headers()
            return
        url = urlsplit(self.path)
        env = dict(os.environ, GIT_PROJECT_ROOT=root, GIT_HTTP_EXPORT_ALL='1',
                   PATH_INFO=url.path, QUERY_STRING=url.query,
                   REQUEST_METHOD=self.command, REMOTE_USER=credentials['username'],
                   CONTENT_TYPE=self.headers.get('Content-Type', ''),
                   CONTENT_LENGTH=self.headers.get('Content-Length', '0'))
        data = self.rfile.read(int(env['CONTENT_LENGTH']))
        proc = subprocess.run(['git', 'http-backend'], input=data, env=env,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True)
        header, body = proc.stdout.split(b'\r\n\r\n', 1)
        headers = [line.decode().split(':', 1) for line in header.split(b'\r\n')]
        status = next((int(v.strip().split()[0]) for k, v in headers if k == 'Status'), 200)
        self.send_response(status)
        for k, v in headers:
            if k != 'Status':
                self.send_header(k, v.strip())
        self.end_headers()
        self.wfile.write(body)


http.server.ThreadingHTTPServer(('0.0.0.0', int(port)), Git).serve_forever()
