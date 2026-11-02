#!/usr/bin/env python3
"""Real HTTP/SQLite contribution acceptance against disposable local providers.

Run after cargo build -p ferry-server, or set SERVER_BIN/FERRY_BINARY. No external
provider accounts, network destinations, notifications, or credentials are used.
Provider responses exercise documented shapes; this is not live-provider proof.
"""

from collections import Counter
from contextlib import closing
import hashlib
import hmac
import http.cookiejar
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import socket
import sqlite3
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.request
from urllib.parse import parse_qs, unquote, urlencode, urlparse


A, B, C, D = (letter * 40 for letter in "abcd")
PROVIDERS = ("github", "gitlab", "gitee", "gitea", "codeup")
SUPPORTED = PROVIDERS[:-1]
states = {provider: {"heads": {"main": C, "feature/shared": B}, "mode": "normal"}
          for provider in PROVIDERS}
states["gitea"]["heads"] = {"main": B, "feature/shared": A}
calls = Counter()
fixture_errors = []
api_base = ""


def metadata(provider, sha):
    parents = {A: [], B: [A], C: [A, B], D: [C]}[sha]
    if provider == "gitea" and sha == D:
        parents = [B]
    author = {"name": "Fixture Author", "email": "author@example.invalid", "date": "2025-01-01T12:00:00Z"}
    if provider == "github":
        author.update(name="Other Author" if sha == C else "Fixture Author",
                      email={A: "first@example.invalid", B: "second@example.invalid",
                             C: "other@example.invalid", D: "first@example.invalid"}[sha])
    committer = dict(author, date="2025-01-02T12:00:00Z")
    message = {A: "chore: initial", B: "feat(scope)!： shared\n\nfull description", C: "merge: branches", D: "fix: incremental"}[sha]
    if provider == "gitlab":
        return {"id": sha, "parent_ids": parents, "message": message,
                "author_name": author["name"], "author_email": author["email"],
                "authored_date": author["date"], "committed_date": committer["date"]}
    if provider == "codeup":
        return {"id": sha, "parentIds": parents, "message": message,
                "authorName": author["name"], "authorEmail": author["email"],
                "authoredDate": author["date"], "committedDate": committer["date"]}
    return {"sha": sha, "parents": [{"sha": p} for p in parents],
            "commit": {"message": message, "author": author, "committer": committer}}


def files(sha):
    added, deleted = {A: (1, 0), B: (3, 1), C: (3, 0), D: (2, 0)}[sha]
    return [{"filename": {A: "initial.txt", B: "shared.rs", C: "merge.py", D: "incremental.ts"}[sha],
             "additions": added, "deletions": deleted}]


class Provider(BaseHTTPRequestHandler):
    def json(self, value, more=False, status=200):
        payload = json.dumps(value).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.send_header("x-next-page", "2" if more else "")
        self.end_headers()
        self.wfile.write(payload)

    def do_GET(self):
        try:
            self.respond()
        except Exception as error:
            fixture_errors.append(repr(error))
            self.json({"error": str(error)}, status=500)

    def respond(self):
        parsed = urlparse(self.path)
        query = parse_qs(parsed.query)
        token = (query.get("access_token", [""])[0] or self.headers.get("PRIVATE-TOKEN")
                 or self.headers.get("x-yunxiao-token") or self.headers.get("Authorization", "").removeprefix("Bearer ").removeprefix("token "))
        provider = token.removeprefix("valid-personal-token-")
        assert provider in PROVIDERS, "unexpected fixture authorization"
        if provider == "codeup":
            assert self.headers.get("x-yunxiao-token") and not query.get("access_token")
        elif provider == "gitlab":
            assert self.headers.get("PRIVATE-TOKEN")
        elif provider == "gitee":
            assert query.get("access_token")
        elif provider == "gitea":
            assert self.headers.get("Authorization", "").startswith("token ")
        calls[(provider, "request")] += 1
        path = parsed.path
        page = int(query.get("page", ["1"])[0])
        state = states[provider]
        repository = "test-org/team/repository" if provider == "codeup" else "owner/repository"
        prefix = ("/oapi/v1/codeup/organizations/test-org/repositories" if provider == "codeup"
                  else "/projects" if provider == "gitlab" else "/user/repos")
        if path.endswith("/user"):
            return self.json({"login": "fixture", "username": "fixture"})
        if path == prefix:
            row = {"id": 101, "name": "repository", "default_branch": "main"}
            if provider == "codeup":
                row.update(pathWithNamespace=repository, webUrl=api_base + "/" + repository)
            elif provider == "gitlab":
                row.update(path_with_namespace=repository, http_url_to_repo=api_base + "/" + repository + ".git")
            else:
                row.update(full_name=repository, clone_url=api_base + "/" + repository + ".git")
            return self.json([row] if page == 1 else [])
        if path.endswith("/branches"):
            size = "perPage" if provider == "codeup" else "limit" if provider == "gitea" else "per_page"
            assert query.get(size) == ["50"], (provider, query)
            if provider == "codeup":
                assert unquote(path.split("/repositories/", 1)[1][:-len("/branches")]) == repository
            elif provider == "gitlab":
                assert "/projects/owner%2Frepository/repository/branches" == path
            rows = []
            for name, sha in state["heads"].items():
                if provider == "codeup":
                    rows.append({"name": name, "commitId": sha})
                else:
                    rows.append({"name": name, "commit": {"id" if provider == "gitlab" else "sha": sha}})
            # A short first page still has an explicit next page.
            return self.json(rows[:1] if page == 1 else rows[1:], more=page == 1 and len(rows) > 1)
        if "/compare" in path:
            calls[(provider, "compare")] += 1
            if provider == "gitea":
                # Gitea Compare exposes commits, not a direct parent-tree diff.
                return self.json({"total_commits": 1, "commits": [metadata(provider, C)]})
            parent = query.get("from", [""])[0] if provider == "gitlab" else unquote(path.rsplit("/", 1)[1]).split("...")[0]
            if provider == "gitlab":
                assert query.get("straight") == ["true"] and query.get("to") == [C]
                return self.json({"diffs": [{"new_path": "merge.py", "diff": "@@ -0,0 +1 @@\n" + "+line\n" * (3 if parent == A else 2)}]})
            return self.json({"files": [{"filename": "merge.py", "additions": 3 if parent == A else 2, "deletions": 0}]})
        if "/commits/" in path:
            suffix = path.split("/commits/", 1)[1].split("/")
            sha = suffix[0]
            assert sha in (A, B, C, D), path
            if provider == "gitea":
                assert "/git/commits/" in path, "Gitea requires its git commit endpoint"
            if len(suffix) > 1:
                assert provider == "gitlab" and suffix[1:] == ["diff"]
                calls[(provider, "diff")] += 1
                changed = files(sha)[0]
                return self.json([{"old_path": changed["filename"], "new_path": changed["filename"],
                                   "diff": "@@ -1 +1 @@\n" + "-old\n" * changed["deletions"] + "+new\n" * changed["additions"]}])
            calls[(provider, "commit")] += 1
            calls[(provider, sha)] += 1
            row = metadata(provider, sha)
            if provider == "codeup":
                # Official metadata alone cannot establish complete line/file
                # statistics. It must not silently become zero contribution.
                return self.json(row)
            row["stats"] = {"additions": files(sha)[0]["additions"], "deletions": files(sha)[0]["deletions"]}
            if provider != "gitlab":
                row["files"] = files(sha)
            if provider == "gitea":
                row["files"] = [{"filename": item["filename"], "status": "modified"} for item in files(sha)]
            if provider == "github" and sha == B:
                if page == 1:
                    return self.json(row, more=True)
                row["files"] = [{"filename": "second.rs", "additions": 0, "deletions": 0}]
            if sha == D and state["mode"] == "incomplete":
                row["truncated"] = True
            return self.json(row)
        raise AssertionError("unexpected provider endpoint: " + path)

    def do_POST(self):
        fixture_errors.append("unexpected outbound POST " + self.path)
        self.json({"error": "no sending is expected"}, status=500)

    def log_message(self, *args):
        pass


def main():
    global api_base
    receiver = ThreadingHTTPServer(("127.0.0.1", 0), Provider)
    threading.Thread(target=receiver.serve_forever, daemon=True).start()
    api_base = f"http://127.0.0.1:{receiver.server_port}"
    with tempfile.TemporaryDirectory(prefix="contribution-http-test-") as directory:
        data = Path(directory)
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        base = f"http://127.0.0.1:{port}/api"
        env = dict(os.environ, FERRY_DATA_DIR=directory, FERRY_BIND=f"127.0.0.1:{port}")
        env.pop("FERRY_DATABASE_URL", None)
        client = urllib.request.build_opener(urllib.request.ProxyHandler({}), urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))

        def request(path, body=None, method=None, expected=200):
            req = urllib.request.Request(base + path, data=json.dumps(body).encode() if body is not None else None,
                                         method=method, headers={"Content-Type": "application/json", "X-Ferry-Request": "1", 'Accept-Language': 'zh-CN'})
            try:
                response = client.open(req, timeout=30)
            except urllib.error.HTTPError as error:
                response = error
            with response:
                raw = response.read()
                try:
                    value = json.loads(raw) if raw else None
                except ValueError:
                    raise AssertionError((path, response.code, raw.decode("utf-8", "replace")))
                assert response.code == expected, (path, response.code, value)
                return value

        binary = os.environ.get("SERVER_BIN", os.environ.get("FERRY_BINARY", "target/debug/ferry-server"))
        log = (data / "server.log").open("wb")
        proc = subprocess.Popen([binary, "--desktop-control"], env=env, stdout=log, stderr=log, stdin=subprocess.PIPE,
                                creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
        try:
            for _ in range(300):
                assert proc.poll() is None, (data / "server.log").read_text(encoding="utf-8")
                try:
                    if request("/status")["ok"]:
                        break
                except OSError:
                    pass
                time.sleep(.1)
            else:
                raise AssertionError("server startup timed out")
            request("/setup", {"email": "admin@example.invalid", "name": "Admin", "password": "fixture8",
                               "token": (data / "setup-token").read_text(encoding="utf-8").strip()})
            root = "/workspaces/" + request("/workspaces", {"name": "Fixture", "slug": "fixture"})["id"]
            connections = {}

            def analysis(identity):
                return request(root + "/code-connections/" + identity + "/analysis")

            def wait(identity, state="complete", after=0):
                deadline = time.monotonic() + 45
                while time.monotonic() < deadline:
                    result = analysis(identity)
                    sources = result["sources"]
                    if sources and all(source["state"] == state and source["job"]["state"] == state
                                       and source["job"]["generation"] > after for source in sources):
                        return result
                    time.sleep(.15)
                raise AssertionError(("analysis timeout", result, fixture_errors, (data / "server.log").read_text(encoding="utf-8")[-6000:]))

            def retry(identity):
                previous = analysis(identity)["sources"][0]["job"]["generation"]
                request(root + "/code-connections/" + identity + "/analysis-runs", {}, method="POST")
                return previous

            def report(provider, kind="commits", **query):
                return request(root + "/code-contributions/" + kind + "?" + urlencode(dict(connection_id=connections[provider], isMerge="0,1", **query)))

            for provider in PROVIDERS:
                credential = request(root + "/credentials", {"name": provider + " PAT", "kind": "token", "username": "fixture",
                                                              "secret": "valid-personal-token-" + provider}, expected=201)["id"]
                body = {"name": provider, "provider": provider, "credential_id": credential,
                        "base_url": api_base, "api_url": api_base}
                if provider == "codeup":
                    body["organization"] = "test-org"
                identity = request(root + "/code-connections", body, expected=201)["id"]
                connections[provider] = identity
                request(root + "/code-connections/" + identity + "/analysis",
                        {"version": 0, "config": {"enabled": True, "all_repositories": True, "full_history": True}}, method="PATCH")
            for provider in SUPPORTED:
                wait(connections[provider])
                result = report(provider)
                facts = {item["commitHash"]: item for item in result["items"]}
                expected_shas = {A, B} if provider == "gitea" else {A, B, C}
                assert result["coverage"]["complete"] and result["total"] == len(expected_shas), (provider, result)
                assert set(facts) == expected_shas
                if provider != "gitea":
                    assert facts[C]["additions"] == 5 and facts[C]["languageStats"] == {".py": 1}
                assert facts[A]["additions"] == facts[A]["deletions"] == 0
                assert facts[B]["effectives"] == 2 and facts[B]["messageType"] == "feat"
                assert facts[B]["message"] == "feat(scope)!： shared\n\nfull description"
                assert set(facts[B]["branches"]) == ({"main"} if provider == "gitea" else {"main", "feature/shared"})
                assert facts[B]["date"] != facts[B]["committerDate"]
                overview = report(provider, "overview")["items"][0]
                expected_totals = (2, 3, 1, 2) if provider == "gitea" else (3, 8, 1, 7)
                assert (overview["commits"], overview["additions"], overview["deletions"], overview["effectives"]) == expected_totals, overview
                expected_branches = {"main": 2, "feature/shared": 1} if provider == "gitea" else {"main": 3, "feature/shared": 2}
                assert {row["branchName"]: row["commits"] for row in report(provider, "branches")["items"]} == expected_branches
                assert calls[(provider, "compare")] == (0 if provider == "gitea" else 2)
                print("PASS", provider, "HTTP graph, branch pagination, metadata and de-duplicated reports", flush=True)
            failed = wait(connections["codeup"], "failed")
            assert failed["sources"][0]["last_error"]
            result = report("codeup")
            assert not result["coverage"]["complete"] and result["total"] == 0
            print("PASS codeup incomplete metadata is failed, unpublished, and not represented as zero contribution", flush=True)
            assert calls[("gitlab", "diff")] == 3
            assert calls[("github", B)] == 2

            # Identity edits operate entirely on persisted facts. Same display
            # names remain distinct until explicit aliases map them to a person.
            identity = connections["github"]
            raw_rows = report("github", "contributors")["items"]
            assert len(raw_rows) == 3, raw_rows
            raw_keys = {row["email"]: row["authorKey"] for row in raw_rows}
            assert len(set(raw_keys.values())) == 3
            assert len([row for row in raw_rows if row["nickname"] == "Fixture Author"]) == 2

            def persisted_identity_state():
                with closing(sqlite3.connect(data / "ferry.sqlite")) as database:
                    source = database.execute(
                        "SELECT id,repository_id,state,synced_at,coverage_start FROM repository_analysis_sources WHERE connection_id=?",
                        (identity,)).fetchone()
                    jobs = database.execute("SELECT * FROM repository_analysis_jobs WHERE id=?", (source[0],)).fetchall()
                    facts = database.execute(
                        "SELECT sha,parents,message,author_name,author_email,author_date,committer_date,additions,deletions,effectives,language_stats,complete FROM repository_analysis_commits WHERE repository_id=? ORDER BY sha",
                        (source[1],)).fetchall()
                    reconcile = database.execute("SELECT next_reconcile_at FROM repository_analysis_configs WHERE id=?", (identity,)).fetchone()
                return source, jobs, facts, reconcile

            def save_aliases(aliases):
                before = persisted_identity_state()
                request_count = calls[("github", "request")]
                current = analysis(identity)
                config = dict(current["config"], aliases=aliases)
                request(root + "/code-connections/" + identity + "/analysis",
                        {"version": current["version"], "config": config}, method="PATCH")
                # Cross a scheduler turn: merely asserting immediately after
                # PATCH could miss accidentally enqueued discovery/history work.
                time.sleep(2.2)
                assert calls[("github", "request")] == request_count, "mapping-only edit called the provider"
                assert persisted_identity_state() == before, "mapping-only edit changed jobs, coverage, or commit facts"
                return analysis(identity)["config"]["aliases"]

            aliases = save_aliases([
                {"name": "", "email": "first@example.invalid", "nickname": "Combined"},
                {"name": "", "email": "second@example.invalid", "nickname": "Combined"},
            ])
            person = aliases[0]["contributor_id"]
            assert person and aliases[1]["contributor_id"] == person, aliases
            mapped = report("github", "contributors")["items"]
            combined = next(row for row in mapped if row["nickname"] == "Combined")
            assert len(mapped) == 2 and combined["commits"] == 2 and combined["effectives"] == 2
            assert combined["authorKey"] == person
            person_row_id = combined["id"]
            assert report("github", authors=person)["total"] == 2
            ranking = report("github", "ranking")["items"]
            assert [row["effectives"] for row in ranking] == [5, 2]
            assert ranking[1]["authorKey"] == person
            aliases = save_aliases([
                {"name": alias["name"], "email": alias["email"], "nickname": "Renamed"}
                for alias in aliases
            ])
            assert all(alias["contributor_id"] == person for alias in aliases)
            renamed = next(row for row in report("github", "contributors")["items"] if row["nickname"] == "Renamed")
            assert renamed["authorKey"] == person and renamed["id"] == person_row_id
            aliases = save_aliases(aliases + [{"name": "", "email": "other@example.invalid", "nickname": "Independent"}])
            independent = next(alias["contributor_id"] for alias in aliases if alias["email"] == "other@example.invalid")
            assert independent and independent != person
            aliases = save_aliases([dict(alias, nickname="Renamed") for alias in aliases])
            same_names = report("github", "contributors")["items"]
            assert len(same_names) == 2 and {row["authorKey"] for row in same_names} == {person, independent}
            assert all(row["nickname"] == "Renamed" for row in same_names)
            assert report("github", authors=person)["total"] == 2
            assert report("github", authors=independent)["total"] == 1
            save_aliases([])
            restored = report("github", "contributors")["items"]
            assert {row["email"]: row["authorKey"] for row in restored} == raw_keys
            print("PASS stable author mappings merge aliases, preserve IDs on rename, separate same nicknames and restore raw identities without provider calls or jobs", flush=True)

            # Exercise real report pagination and ordering through the database.
            # SQL query-count/plan assertions live in the backend tests; elapsed
            # time here is evidence only, without a machine-dependent threshold.
            query_timings = {}
            for kind in ("contributors", "ranking", "branches"):
                started = time.perf_counter()
                whole = report("github", kind, page_size=500)
                paged = [report("github", kind, page=page, page_size=1) for page in range(1, whole["total"] + 2)]
                assert all(page["total"] == whole["total"] for page in paged)
                assert [row["id"] for page in paged for row in page["items"]] == [row["id"] for row in whole["items"]]
                assert not paged[-1]["items"]
                assert report("github", kind, offset=1, limit=1)["items"] == whole["items"][1:2]
                query_timings[kind] = round((time.perf_counter() - started) * 1000, 2)
            assert [row["effectives"] for row in report("github", "ranking")["items"]] == [5, 2, 0]
            print("PASS database-backed contributor, ranking and branch pages preserve totals, order and offsets", json.dumps(query_timings), flush=True)

            for provider in SUPPORTED:
                initial_total = 2 if provider == "gitea" else 3
                count = calls[(provider, "commit")]
                wait(connections[provider], after=retry(connections[provider]))
                assert report(provider)["total"] == initial_total and calls[(provider, "commit")] == count
                states[provider]["heads"]["new-known-branch"] = B
                wait(connections[provider], after=retry(connections[provider]))
                assert calls[(provider, "commit")] == count
                assert report(provider, branches="new-known-branch")["total"] == 2
                states[provider]["heads"]["main"] = D
                if provider == "github":
                    states[provider]["mode"] = "incomplete"
                    wait(connections[provider], "failed", after=retry(connections[provider]))
                    incomplete = report(provider)
                    assert incomplete["total"] == 3 and not incomplete["coverage"]["complete"]
                    with closing(sqlite3.connect(data / "ferry.sqlite")) as database:
                        head = database.execute("SELECT head FROM repository_analysis_refs WHERE source_id=? AND name='main'",
                                                (analysis(connections[provider])["sources"][0]["id"],)).fetchone()[0]
                    assert head == C, "truncated commit must not advance the published ref"
                    states[provider]["mode"] = "normal"
                    count += 1  # Only the incomplete SHA is retried after recovery.
                    print("PASS truncated GitHub file response retains prior published head and reports incomplete coverage", flush=True)
                wait(connections[provider], after=retry(connections[provider]))
                assert report(provider)["total"] == initial_total + 1 and calls[(provider, "commit")] == count + 1
                print("PASS", provider, "retry reuses facts, known SHA gains branch membership, new SHA analyzed once", flush=True)
            # Signed duplicate events drive the same durable jobs while notification
            # filters exclude push; there is no outbound notification destination.
            secret = "fixture-webhook-private-secret"
            for provider in SUPPORTED:
                identity = connections[provider]
                request(root + "/code-connection-monitors/" + identity,
                        {"version": 0, "enabled": True, "all_repositories": True, "repositories": "", "events": ["tag.created"],
                         "secret": secret, "webhook": ""}, method="PATCH")
                previous = analysis(identity)["sources"][0]["job"]["generation"]
                count = calls[(provider, "commit")]
                payload = {"repository": {"full_name": "owner/repository"}, "ref": "refs/heads/main", "before": C, "after": D}
                if provider == "gitlab":
                    payload.update(object_kind="push", project={"path_with_namespace": "owner/repository"})
                raw = json.dumps(payload).encode()
                for delivery in ("first", "duplicate"):
                    headers = {"Content-Type": "application/json"}
                    if provider == "github":
                        headers.update({"X-GitHub-Event": "push", "X-GitHub-Delivery": delivery,
                                        "X-Hub-Signature-256": "sha256=" + hmac.new(secret.encode(), raw, hashlib.sha256).hexdigest()})
                    elif provider == "gitea":
                        headers.update({"X-Gitea-Event": "push", "X-Gitea-Delivery": delivery,
                                        "X-Gitea-Signature": hmac.new(secret.encode(), raw, hashlib.sha256).hexdigest()})
                    else:
                        name = "Gitlab" if provider == "gitlab" else "Gitee"
                        headers.update({"X-" + name + "-Event": "Push Hook", "X-" + name + "-Token": secret})
                    req = urllib.request.Request(base + "/repository-events/" + identity, data=raw, headers=headers)
                    with urllib.request.urlopen(req, timeout=20) as response:
                        assert response.code == 204
                wait(identity, after=previous)
                assert report(provider)["total"] == (3 if provider == "gitea" else 4) and calls[(provider, "commit")] == count
                assert request(root + "/code-connections/" + identity + "/events")["total"] == 0
                print("PASS", provider, "authenticated duplicate push events dirty analysis independently of notification subscriptions", flush=True)
            states["gitea"]["heads"]["main"] = C
            wait(connections["gitea"], "failed", after=retry(connections["gitea"]))
            assert report("gitea")["total"] == 3 and not report("gitea")["coverage"]["complete"]
            print("PASS Gitea merge Compare without direct diff is explicitly failed, preserving published facts", flush=True)
            with closing(sqlite3.connect(data / "ferry.sqlite")) as database:
                counts = dict(database.execute("SELECT p.provider, COUNT(*) FROM repository_analysis_commits c JOIN repository_analysis_repositories p ON p.id=c.repository_id GROUP BY p.provider"))
                assert counts == {provider: 3 if provider == "gitea" else 4 for provider in SUPPORTED}, counts
                assert database.execute("SELECT COUNT(*) FROM repository_analysis_refs WHERE present=1").fetchone()[0] == 12
            assert not fixture_errors, fixture_errors
            print(json.dumps({"result": "PASS", "scope": "real isolated Ferry HTTP + SQLite + five local provider contracts; not live-provider verification", "fact_counts": counts}))
        except Exception:
            print((data / "server.log").read_text(encoding="utf-8")[-8000:], flush=True)
            raise
        finally:
            if proc.poll() is None:
                proc.terminate()
                proc.wait(timeout=30)
            log.close()
            receiver.shutdown()
            receiver.server_close()


if __name__ == "__main__":
    main()
