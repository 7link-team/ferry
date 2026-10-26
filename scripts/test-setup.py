#!/usr/bin/env python3
"""Initialize a distinct fixture superadmin; tenant test users stay ordinary.
Usage: test-setup.py API_BASE DATA_DIR [--legacy]
"""
import json
from pathlib import Path
import sys
import urllib.error
import urllib.request

base, directory = sys.argv[1:3]
client = urllib.request.build_opener(urllib.request.ProxyHandler({}))
try:
    with client.open(base + '/setup', timeout=10) as response:
        if '--legacy' in sys.argv and 'application/json' not in response.headers.get('Content-Type', ''):
            # Published versions serve the SPA fallback for unknown GET paths.
            assert response.headers.get('Content-Type', '').startswith('text/html')
            sys.exit(0)
        status = json.load(response)
except urllib.error.HTTPError as error:
    if error.code == 404 and '--legacy' in sys.argv:
        sys.exit(0)
    raise
if status['required']:
    body = {'token': (Path(directory) / 'setup-token').read_text().strip(),
            'name': 'Platform fixture', 'email': 'platform-fixture@example.invalid',
            'password': 'temporary-platform-password'}
    req = urllib.request.Request(base + '/setup', data=json.dumps(body).encode(),
        headers={'Content-Type': 'application/json', 'X-Ferry-Request': '1'})
    with client.open(req, timeout=30) as response:
        assert response.status == 200
