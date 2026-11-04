#!/usr/bin/env python3
"""Start an isolated real server and provider, then run Chromium without API mocks."""
from http.server import ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlparse
import http.cookiejar
import importlib.util
import json
import os
import socket
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request


def main():
    root = Path(__file__).resolve().parent.parent
    output = root / "data/maintenance/reports-reviews-live"
    output.mkdir(parents=True, exist_ok=True)
    spec = importlib.util.spec_from_file_location("contribution_fixture", root / "scripts/test-contribution-analysis.py")
    fixture = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(fixture)

    class Provider(fixture.Provider):
        def respond(self):
            parsed = urlparse(self.path)
            if parsed.path == "/user/repos":
                return self.json([{"id": 101, "name": "repository", "default_branch": "main", "full_name": "owner/repository", "clone_url": fixture.api_base + "/owner/repository.git"}])
            return super().respond()

    provider = ThreadingHTTPServer(("127.0.0.1", 0), Provider)
    fixture.api_base = f"http://127.0.0.1:{provider.server_port}"
    threading.Thread(target=provider.serve_forever, daemon=True).start()
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        port = probe.getsockname()[1]
    origin = f"http://127.0.0.1:{port}"
    state = Path(tempfile.mkdtemp(prefix="fixture-", dir=output))
    client = urllib.request.build_opener(urllib.request.ProxyHandler({}), urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))

    def request(path, body=None, method=None):
        req = urllib.request.Request(origin + "/api" + path, data=json.dumps(body).encode() if body is not None else None, method=method, headers={"Content-Type": "application/json", "X-Ferry-Request": "1"})
        with client.open(req, timeout=30) as response:
            return json.load(response)

    binary = str(Path(sys.argv[1] if len(sys.argv) > 1 else root / "target/debug/ferry-server.exe").resolve())
    environment = dict(os.environ, FERRY_DATA_DIR=str(state), FERRY_BIND=f"127.0.0.1:{port}")
    environment.pop("FERRY_DATABASE_URL", None)
    flags = {"creationflags": subprocess.CREATE_NO_WINDOW} if os.name == "nt" else {}
    with (state / "server.log").open("wb") as log:
        process = subprocess.Popen([binary], cwd=root, env=environment, stdout=log, stderr=log, **flags)
        try:
            deadline = time.monotonic() + 45
            while True:
                try:
                    if request("/status")["ok"]:
                        break
                except OSError:
                    assert process.poll() is None, "server exited; inspect server.log"
                assert time.monotonic() < deadline, "server startup timed out"
                time.sleep(.2)
            email, password = "browser@example.invalid", "fixture-password"
            request("/setup", {"name": "Browser acceptance", "email": email, "password": password, "token": (state / "setup-token").read_text().strip()})
            workspace = request("/workspaces", {"name": "Browser acceptance", "slug": "browser-acceptance"})["id"]
            base = "/workspaces/" + workspace
            credential = request(base + "/credentials", {"name": "Provider fixture token", "kind": "token", "username": "fixture", "secret": "valid-personal-token-github"})["id"]
            connection = request(base + "/code-connections", {"name": "Provider fixture", "provider": "github", "credential_id": credential, "base_url": fixture.api_base, "api_url": fixture.api_base})["id"]
            insight = request(base + "/code-insights", {"name": "Source insight", "enabled": True, "update_mode": "event", "full_history": True, "initial_days": 7, "repositories": [{"connection_id": connection, "repository": "owner/repository"}]})["id"]
            deadline = time.monotonic() + 60
            while True:
                detail = request(base + "/code-insights/" + insight)
                if detail["sources"] and all(source["state"] == "complete" for source in detail["sources"]):
                    break
                assert time.monotonic() < deadline, detail
                time.sleep(.2)
            detail["aliases"] = [{"name": "", "email": email, "nickname": "Combined"} for email in ["first@example.invalid", "second@example.invalid"]]
            request(base + "/code-insights/" + insight, detail, "PATCH")
            host = request(base + "/nodes", {"name": "Report fixture host", "connection": "runner"})["id"]
            manifest = json.dumps({"apiVersion": "ferry/v1", "kind": "Application", "metadata": {"name": "fixture"}, "services": [{"name": "web", "image": "nginx:alpine"}]})
            request(base + "/deployments", {"name": "Fixture application", "mode": "compose", "target_kind": "host", "target_ref": host, "workdir": "/tmp/report-fixture", "manifest": manifest})
            browser_env = dict(os.environ, LIVE_BASE_URL=origin, LIVE_WORKSPACE=workspace, LIVE_INSIGHT_ID=insight, LIVE_CONNECTION_ID=connection, LIVE_EMAIL=email, LIVE_PASSWORD=password, LIVE_OUTPUT=str(state))
            result = subprocess.run(["node", "scripts/test-reports-reviews-live-ui.mjs"], cwd=root, env=browser_env, capture_output=True, text=True, encoding="utf-8", errors="replace", **flags)
            (state / "browser.log").write_text(result.stdout + result.stderr, encoding="utf-8")
            print(result.stdout + result.stderr, flush=True)
            assert result.returncode == 0, "live browser failed; inspect browser.log"
            assert not fixture.fixture_errors, fixture.fixture_errors
            print("PASS real product API + Chromium report CRUD/generation/aliases and retired review endpoints; fixture state: " + str(state), flush=True)
        finally:
            (state / "provider-evidence.json").write_text(json.dumps({"fixture_errors": fixture.fixture_errors, "calls": {"/".join(key): value for key, value in fixture.calls.items()}}, indent=2), encoding="utf-8")
            process.terminate()
            process.wait(30)
            provider.shutdown()


if __name__ == "__main__":
    main()
