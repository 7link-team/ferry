#!/usr/bin/env python3
"""Real standalone insight HTTP acceptance with an isolated local provider.

Run: python scripts/test-code-insights.py SERVER_BINARY
Provider response shapes come from the existing contribution HTTP fixture.
No live provider accounts, Runner, act, or external messages are involved.
"""
from contextlib import closing
import http.cookiejar
from http.server import ThreadingHTTPServer
import importlib.util
import json
import os
from pathlib import Path
import socket
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request
from urllib.parse import parse_qs, urlencode, urlparse


def main():
    spec = importlib.util.spec_from_file_location(
        "contribution_fixture", Path(__file__).with_name("test-contribution-analysis.py"))
    fixture = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(fixture)
    available_repositories = ["owner/repository"]

    class Provider(fixture.Provider):
        def respond(self):
            parsed = urlparse(self.path)
            if parsed.path == "/user/repos":
                assert self.headers.get("Authorization") == "Bearer valid-personal-token-github"
                fixture.calls[("github", "request")] += 1
                page = parse_qs(parsed.query).get("page", ["1"])[0]
                return self.json([
                    {"id": 101 + index, "name": path.rsplit("/", 1)[1],
                     "default_branch": "main", "full_name": path,
                     "clone_url": fixture.api_base + "/" + path + ".git"}
                    for index, path in enumerate(available_repositories)
                ] if page == "1" else [])
            return super().respond()

    receiver = ThreadingHTTPServer(("127.0.0.1", 0), Provider)
    fixture.api_base = f"http://127.0.0.1:{receiver.server_port}"
    threading.Thread(target=receiver.serve_forever, daemon=True).start()
    flags = {"creationflags": subprocess.CREATE_NO_WINDOW} if os.name == "nt" else {}
    server = str(Path(sys.argv[1]).resolve())
    process = None
    try:
        with tempfile.TemporaryDirectory(prefix="code-insights-http-") as directory:
            work = Path(directory)
            with socket.socket() as probe:
                probe.bind(("127.0.0.1", 0))
                port = probe.getsockname()[1]
            origin = f"http://127.0.0.1:{port}/api"
            environment = dict(os.environ, FERRY_DATA_DIR=directory,
                               FERRY_BIND=f"127.0.0.1:{port}")
            environment.pop("FERRY_DATABASE_URL", None)
            client = urllib.request.build_opener(
                urllib.request.ProxyHandler({}),
                urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))

            def request(path, body=None, method=None, expected=200, opener=None):
                req = urllib.request.Request(
                    origin + path, data=json.dumps(body).encode() if body is not None else None,
                    method=method, headers={"Content-Type": "application/json", "X-Ferry-Request": "1", 'Accept-Language': 'zh-CN'})
                try:
                    response = (opener or client).open(req, timeout=30)
                except urllib.error.HTTPError as error:
                    response = error
                with response:
                    raw = response.read()
                    try:
                        value = json.loads(raw or b"null")
                    except json.JSONDecodeError:
                        value = raw.decode("utf8", errors="replace")
                    assert response.status == expected, (path, response.status, value)
                    return value

            def wait(probe, label, timeout=50):
                deadline = time.monotonic() + timeout
                while time.monotonic() < deadline:
                    assert process.poll() is None, "server exited"
                    try:
                        value = probe()
                        if value:
                            return value
                    except OSError:
                        pass
                    time.sleep(.15)
                raise AssertionError("timed out: " + label)

            def sql(query, params=()):
                with closing(sqlite3.connect(work / "ferry.sqlite", timeout=15)) as db:
                    return db.execute(query, params).fetchall()

            with (work / "server.log").open("wb") as log:
                try:
                    def start():
                        nonlocal process
                        process = subprocess.Popen([server], env=environment, stdout=log, stderr=log, **flags)
                        wait(lambda: request("/status")["ok"], "server startup")

                    start()
                    request("/setup", {"name": "Admin", "email": "insights@example.invalid",
                        "password": "fixture-password", "token": (work / "setup-token").read_text().strip()})
                    workspace = request("/workspaces", {"name": "Insights", "slug": "insights"})["id"]
                    root = "/workspaces/" + workspace
                    endpoint = root + "/code-insights"
                    credential = request(root + "/credentials", {"name": "Fixture PAT", "kind": "token",
                        "username": "fixture", "secret": "valid-personal-token-github"}, expected=201)["id"]
                    connection = request(root + "/code-connections", {"name": "Fixture GitHub", "provider": "github",
                        "credential_id": credential, "base_url": fixture.api_base,
                        "api_url": fixture.api_base}, expected=201)["id"]

                    def create(name, full=True):
                        return request(endpoint, {"name": name, "enabled": True, "update_mode": "event", "full_history": full,
                            "initial_days": 7, "repositories": [{"connection_id": connection,
                            "repository": "owner/repository"}]}, expected=201)["id"]

                    def detail(identity):
                        return request(endpoint + "/" + identity)

                    def report(identity, kind="commits", **params):
                        query = urlencode(dict(isMerge="0,1", **params))
                        return request(endpoint + "/" + identity + "/reports/" + kind + "?" + query)

                    def collected(identity):
                        value = detail(identity)
                        rows = value["sources"]
                        return value if rows and all(r["state"] == "complete" and
                            r["job"] and r["job"]["state"] == "complete" for r in rows) else None

                    def persisted():
                        return tuple(sql("SELECT * FROM " + table + " ORDER BY id") for table in (
                            "repository_analysis_configs", "repository_analysis_sources",
                            "repository_analysis_jobs", "repository_analysis_commits"))

                    first = create("Entire history")
                    wait(lambda: collected(first), "first standalone insight collection")
                    assert report(first)["total"] == 3
                    assert report(first)["coverage"]["complete"]
                    state = persisted()
                    calls = fixture.calls[("github", "request")]
                    recent = create("Recent history", full=False)
                    independent = create("Independent authors")
                    time.sleep(2.2)
                    assert persisted() == state, "overlapping scopes mutated the shared collector or facts"
                    assert fixture.calls[("github", "request")] == calls, "overlapping scopes re-fetched the provider"
                    assert report(recent)["total"] == 0, "scope coverage must filter older shared facts"
                    assert report(independent)["total"] == 3
                    for table in ("repository_analysis_configs", "repository_analysis_sources", "repository_analysis_jobs"):
                        assert sql("SELECT COUNT(*) FROM " + table)[0][0] == 1
                    assert request(endpoint)["total"] == 3
                    assert not sql("SELECT id FROM pipelines")
                    assert not sql("SELECT id FROM pipeline_runs")
                    print("PASS real insight create/list/collection: three scopes, one collector, independent coverage, no act", flush=True)

                    current = detail(first)
                    stale = dict(current)
                    current["aliases"] = [
                        {"name": "", "email": "first@example.invalid", "nickname": "Combined"},
                        {"name": "", "email": "second@example.invalid", "nickname": "Combined"}]
                    request(endpoint + "/" + first, current, method="PATCH")
                    request(endpoint + "/" + first, stale, method="PATCH", expected=409)
                    time.sleep(2.2)
                    assert persisted() == state, "mapping edits changed collection or immutable facts"
                    assert fixture.calls[("github", "request")] == calls
                    mapped = report(first, "contributors")["items"]
                    assert len(mapped) == 2
                    assert next(row for row in mapped if row["nickname"] == "Combined")["commits"] == 2
                    assert len(report(independent, "contributors")["items"]) == 3
                    assert detail(independent)["aliases"] == []
                    print("PASS mapping and stale-version HTTP updates: local identities, unchanged provider/jobs/facts", flush=True)

                    other = request("/workspaces", {"name": "Other", "slug": "other-insights"})["id"]
                    request("/workspaces/" + other + "/code-insights/" + first, expected=404)
                    request("/workspaces/" + other + "/code-insights", {"name": "Foreign connection", "enabled": True,
                        "repositories": [{"connection_id": connection, "repository": "owner/repository"}]}, expected=404)
                    anonymous = urllib.request.build_opener(urllib.request.ProxyHandler({}))
                    request(endpoint, expected=401, opener=anonymous)
                    assert request(endpoint)["total"] == 3
                    assert persisted() == state
                    process.terminate()
                    process.wait(30)
                    start()
                    assert request(endpoint)["total"] == 3
                    restarted = persisted()
                    # New demand rows can still carry the legacy identity schema
                    # marker 0 until startup recovery. That migration must not
                    # change any collector setting, schedule, lease, or fact.
                    assert restarted[1:] == state[1:]
                    assert len(restarted[0]) == len(state[0])
                    for before, after in zip(state[0], restarted[0]):
                        assert before[:2] == after[:2] and before[3:] == after[3:]
                        previous, current = json.loads(before[2]), json.loads(after[2])
                        previous.pop("identity_version")
                        assert current.pop("identity_version") == 1
                        assert previous == current
                    state = restarted
                    assert len(report(first, "contributors")["items"]) == 2
                    print("PASS HTTP workspace/anonymous isolation, atomic forbidden creation, persisted restart", flush=True)

                    paused = detail(first)
                    paused["enabled"] = False
                    request(endpoint + "/" + first, paused, method="PATCH")
                    assert persisted() == state, "pausing one scope disturbed another enabled demand"
                    request(endpoint + "/" + first, method="DELETE")
                    assert persisted() == state
                    request(endpoint + "/" + independent, method="DELETE")
                    assert json.loads(sql("SELECT config FROM repository_analysis_configs")[0][0])["enabled"]
                    request(endpoint + "/" + recent, method="DELETE")
                    assert not json.loads(sql("SELECT config FROM repository_analysis_configs")[0][0])["enabled"]
                    assert sql("SELECT COUNT(*) FROM repository_analysis_commits")[0][0] == 3
                    time.sleep(2.2)
                    assert fixture.calls[("github", "request")] == calls
                    assert request(endpoint)["total"] == 0
                    assert not fixture.fixture_errors, fixture.fixture_errors
                    print("PASS pause/delete demand aggregation: other scopes continue; last removal stops collection and retains facts", flush=True)

                    # Exercise the real background scheduler. SQL only advances
                    # its persisted test clock; configuration/sync use the API.
                    scheduled = request(endpoint, {"name": "Default schedule", "enabled": True,
                        "full_history": True, "repositories": [{"connection_id": connection,
                        "repository": "owner/repository"}]}, expected=201)["id"]
                    wait(lambda: collected(scheduled), "scheduled insight initialization")
                    schedule = detail(scheduled)
                    assert schedule["update_mode"] == "scheduled"
                    assert schedule["schedule"] == "0 */4 * * *"
                    assert schedule["next_run_at"] > int(time.time() * 1000)
                    generation = schedule["sources"][0]["job"]["generation"]
                    requests_before = fixture.calls[("github", "request")]
                    time.sleep(2.2)
                    assert detail(scheduled)["sources"][0]["job"]["generation"] == generation
                    assert fixture.calls[("github", "request")] == requests_before
                    with closing(sqlite3.connect(work / "ferry.sqlite", timeout=15)) as db, db:
                        db.execute("UPDATE code_insights SET next_run_at=1 WHERE id=?", (scheduled,))
                    def scheduled_run(after):
                        result = collected(scheduled)
                        return result if result and result["sources"][0]["job"]["generation"] > after else None
                    ran = wait(lambda: scheduled_run(generation), "real due scheduled collection")
                    assert ran["sources"][0]["job"]["generation"] == generation + 1
                    next_run = ran["next_run_at"]
                    assert next_run > int(time.time() * 1000)
                    time.sleep(2.2)
                    assert detail(scheduled)["sources"][0]["job"]["generation"] == generation + 1
                    assert detail(scheduled)["next_run_at"] == next_run
                    request(endpoint + "/" + scheduled + "/sync", {}, method="POST")
                    manual = wait(lambda: scheduled_run(generation + 1), "manual sync of scheduled insight")
                    assert manual["sources"][0]["job"]["generation"] == generation + 2
                    assert manual["next_run_at"] == next_run
                    assert sql("SELECT COUNT(*) FROM repository_analysis_commits")[0][0] == 3
                    assert not fixture.fixture_errors, fixture.fixture_errors
                    print("PASS default four-hour schedule: future stays idle, overdue ticks coalesce once, manual sync preserves next run", flush=True)

                    union_connection = request(root + "/code-connections", {
                        "name": "Wildcard coverage union", "provider": "github",
                        "credential_id": credential, "base_url": fixture.api_base,
                        "api_url": fixture.api_base}, expected=201)["id"]
                    union_recent = request(endpoint, {
                        "name": "Explicit recent coverage", "enabled": True,
                        "update_mode": "event", "full_history": False, "initial_days": 7,
                        "repositories": [{"connection_id": union_connection,
                                          "repository": "owner/repository"}]}, expected=201)["id"]
                    wait(lambda: collected(union_recent), "recent explicit source")
                    assert report(union_recent)["total"] == 0
                    assert sql("SELECT coverage_start FROM repository_analysis_sources WHERE connection_id=?",
                               (union_connection,))[0][0] > 0
                    union_full = request(endpoint, {
                        "name": "Wildcard complete coverage", "enabled": True,
                        "update_mode": "event", "full_history": True,
                        "repositories": [{"connection_id": union_connection,
                                          "all_repositories": True}]}, expected=201)["id"]
                    wait(lambda: collected(union_full), "wildcard history backfill")
                    assert sql("SELECT coverage_start FROM repository_analysis_sources WHERE connection_id=?",
                               (union_connection,))[0][0] == 0
                    assert report(union_full)["total"] == 3
                    assert report(union_full)["coverage"]["complete"]
                    assert report(union_recent)["total"] == 0
                    assert not fixture.fixture_errors, fixture.fixture_errors
                    print("PASS wildcard full history widens an existing recent explicit source without widening its insight", flush=True)

                    dynamic_connection = request(root + "/code-connections", {
                        "name": "Dynamic repository admission", "provider": "github",
                        "credential_id": credential, "base_url": fixture.api_base,
                        "api_url": fixture.api_base}, expected=201)["id"]
                    dynamic = request(endpoint, {
                        "name": "Owner descendants", "enabled": True,
                        "update_mode": "event", "full_history": True,
                        "repositories": [{"connection_id": dynamic_connection,
                                          "repository": "owner/*"}]}, expected=201)["id"]
                    wait(lambda: collected(dynamic), "dynamic group initialization")
                    assert report(dynamic)["total"] == 3
                    version = detail(dynamic)["version"]
                    available_repositories.extend(["owner/future/new", "outsider/repository"])
                    # Advance only discovery's durable clock. The real scheduler
                    # discovers and queues the newly authorized matching repository.
                    with closing(sqlite3.connect(work / "ferry.sqlite", timeout=15)) as db, db:
                        db.execute("UPDATE repository_analysis_configs SET next_reconcile_at=0 WHERE id=?",
                                   (dynamic_connection,))
                    def admitted():
                        value = collected(dynamic)
                        return value if value and len(value["sources"]) == 2 else None
                    admitted_group = wait(admitted, "future descendant automatic admission")
                    assert detail(dynamic)["version"] == version
                    assert {row["repository"] for row in admitted_group["sources"]} == {
                        "owner/repository", "owner/future/new"}
                    assert report(dynamic)["total"] == 6, "identical SHAs in distinct repositories must remain distinct"
                    assert sql("SELECT COUNT(*) FROM repository_analysis_sources WHERE connection_id=?",
                               (dynamic_connection,))[0][0] == 2
                    all_dynamic = request(endpoint, {
                        "name": "All available repositories", "enabled": True,
                        "update_mode": "event", "full_history": True,
                        "repositories": [{"connection_id": dynamic_connection,
                                          "all_repositories": True}]}, expected=201)["id"]
                    wait(lambda: collected(all_dynamic) if len(detail(all_dynamic)["sources"]) == 3 else None,
                         "all repositories admits outside namespace")
                    assert report(all_dynamic)["total"] == 9
                    assert report(dynamic)["total"] == 6
                    available_repositories.append("outsider/future")
                    with closing(sqlite3.connect(work / "ferry.sqlite", timeout=15)) as db, db:
                        db.execute("UPDATE repository_analysis_configs SET next_reconcile_at=0 WHERE id=?",
                                   (dynamic_connection,))
                    wait(lambda: collected(all_dynamic) if len(detail(all_dynamic)["sources"]) == 4 else None,
                         "all repositories automatically admits future repository")
                    assert report(all_dynamic)["total"] == 12
                    assert report(dynamic)["total"] == 6
                    assert sql("SELECT COUNT(*) FROM repository_analysis_configs WHERE id=?",
                               (dynamic_connection,))[0][0] == 1
                    assert not sql("SELECT id FROM pipelines")
                    assert not sql("SELECT id FROM pipeline_runs")
                    assert not fixture.fixture_errors, fixture.fixture_errors
                    print("PASS automatic nested-group admission without insight edits; outside namespace excluded; all-repository scope shares facts and expands independently", flush=True)
                except Exception:
                    log.flush()
                    print((work / "server.log").read_text(encoding="utf8", errors="replace")[-6000:], file=sys.stderr)
                    raise
                finally:
                    if process and process.poll() is None:
                        process.terminate()
                        process.wait(30)
    finally:
        receiver.shutdown()
        receiver.server_close()


if __name__ == "__main__":
    main()
