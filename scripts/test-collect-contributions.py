#!/usr/bin/env python3
"""Collector tests using disposable Git histories; public network tests are opt-in.

Example: python scripts/test-collect-contributions.py --public-repository
https://github.com/octocat/Hello-World.git
"""

import argparse
import importlib.util
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
import zlib
from unittest.mock import patch


SCRIPT = Path(__file__).with_name("collect-contributions.py")
sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("collector", SCRIPT)
collector = importlib.util.module_from_spec(spec)
spec.loader.exec_module(collector)


class CollectorTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="contribution-collector-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.repo = self.root / "repo"
        self.repo.mkdir()
        self.environment = dict(os.environ)
        for key in tuple(self.environment):
            if key.startswith("GIT_"):
                self.environment.pop(key)
        self.environment.update(GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull,
                                GIT_TERMINAL_PROMPT="0")
        self.git("init", "--initial-branch=main")
        self.git("config", "user.name", "Test Author")
        self.git("config", "user.email", "author@example.invalid")

    def git(self, *args, day=None, author_day=None):
        env = dict(self.environment)
        if day:
            env.update(GIT_COMMITTER_DATE=f"2025-01-{day:02}T12:00:00Z",
                       GIT_AUTHOR_DATE=f"2025-01-{author_day or day:02}T12:00:00Z")
        return subprocess.run(
            ["git", "-c", "core.autocrlf=false", "-c", "commit.gpgsign=false", *args],
            cwd=self.repo, env=env, check=True, stdout=subprocess.PIPE,
            stderr=subprocess.PIPE, encoding="utf-8",
            creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0,
        ).stdout.strip()

    def commit(self, filename, content, message, day, author_day=None):
        (self.repo / filename).write_bytes(content if isinstance(content, bytes) else content.encode())
        self.git("add", "--", filename)
        self.git("commit", "-m", message, day=day, author_day=author_day)
        return self.git("rev-parse", "HEAD")

    def collect(self, known=(), start=0):
        with patch.dict(os.environ, self.environment, clear=True):
            return collector.collect(collector.Git(self.repo), "task", 7, "repository", start, known)

    def history(self):
        base = self.commit("base.txt", "base\n", "chore: initial", 1)
        main = self.commit("main.py", "one\ntwo\n", "feat(scope)!： main\n\nfull body", 3, 1)
        self.git("checkout", "-b", "feature", base)
        feature = self.commit("feature.py", "a\nb\nc\n", "fix: feature", 2)
        self.git("checkout", "main")
        self.git("merge", "--no-ff", "feature", "-m", "merge: feature", day=4)
        merge = self.git("rev-parse", "HEAD")
        self.git("branch", "copy", merge)
        return base, main, feature, merge

    def test_shared_sha_merge_metadata_and_first_parent_extensions(self):
        base, main, feature, merge = self.history()
        result = self.collect()
        facts = {fact["sha"]: fact for fact in result["commits"]}
        self.assertEqual(set(facts), {base, main, feature, merge})
        self.assertEqual(result["refs"], {"copy": merge, "feature": feature, "main": merge})
        self.assertEqual(set(result["ref_commits"]["feature"]), {base, feature})
        self.assertEqual(result["ref_commits"]["main"], result["ref_commits"]["copy"])
        self.assertEqual((facts[merge]["additions"], facts[merge]["deletions"]), (5, 0))
        self.assertEqual(facts[merge]["language_stats"], {".py": 1})
        self.assertEqual(facts[merge]["parents"], [main, feature])
        self.assertEqual(facts[base]["language_stats"], {".txt": 1})
        self.assertEqual((facts[base]["additions"], facts[base]["deletions"]), (0, 0))
        self.assertEqual(facts[main]["message"], "feat(scope)!： main\n\nfull body\n")
        self.assertEqual(facts[main]["committer_date"] - facts[main]["author_date"], 2 * 86400000)
        self.assertEqual(facts[main]["author_email"], "author@example.invalid")

    def test_known_sha_still_acquires_new_branch_membership_without_diff(self):
        shas = self.history()
        self.git("branch", "new-branch", shas[2])
        with patch.object(collector, "analyze", side_effect=AssertionError("recomputed known SHA")):
            result = self.collect(known=shas)
        self.assertEqual(result["commits"], [])
        self.assertEqual(set(result["ref_commits"]["new-branch"]), {shas[0], shas[2]})
        # A new SHA is the only new analysis, regardless of branch count.
        new = self.commit("new.txt", "new\n", "feat: new", 5)
        with patch.object(collector, "analyze", wraps=collector.analyze) as analyze:
            result = self.collect(known=shas)
        self.assertEqual(analyze.call_count, 1)
        self.assertEqual([fact["sha"] for fact in result["commits"]], [new])

    def test_date_skew_does_not_prune_newer_ancestor(self):
        ancestor = self.commit("base.txt", "base\n", "ancestor", 10)
        old = self.commit("old.txt", "old\n", "old date", 1)
        target = self.commit("target.txt", "target\n", "tip", 11)
        cutoff = 1736337600000  # 2025-01-08T12:00:00Z
        result = self.collect(start=cutoff)
        self.assertEqual(set(result["ref_commits"]["main"]), {ancestor, target})
        facts = {fact["sha"]: fact for fact in result["commits"]}
        self.assertEqual(set(facts), {ancestor, old, target})
        self.assertFalse(facts[old]["complete"])
        self.assertEqual(facts[old]["parents"], [ancestor])
        self.assertEqual(facts[old]["language_stats"], {})
        self.assertEqual((facts[old]["additions"], facts[old]["deletions"]), (0, 0))
        self.assertTrue(facts[ancestor]["complete"])
        self.assertNotIn(old, result["ref_commits"]["main"])
        self.assertEqual(result["coverage_start"], cutoff)

    def test_binary_rename_deletion_and_unusual_paths(self):
        self.commit("README", "base\n", "initial", 1)
        self.commit(".config", "value\n", "dotfile", 2)
        binary_path = "space file.bin" if os.name == "nt" else "space\tfile.bin"
        self.commit(binary_path, b"\x00\x01\x02", "binary", 3)
        self.git("mv", "README", "renamed.md")
        self.git("commit", "-m", "rename", day=4)
        renamed = self.git("rev-parse", "HEAD")
        self.git("rm", "--", ".config")
        self.git("commit", "-m", "delete", day=5)
        deleted = self.git("rev-parse", "HEAD")
        facts = {fact["sha"]: fact for fact in self.collect()["commits"]}
        binary = next(fact for fact in facts.values() if fact["message"].startswith("binary"))
        self.assertEqual((binary["additions"], binary["deletions"]), (0, 0))
        self.assertEqual(binary["language_stats"], {".bin": 1})
        self.assertEqual(facts[renamed]["language_stats"], {"": 1, ".md": 1})
        self.assertEqual((facts[renamed]["additions"], facts[renamed]["deletions"]), (1, 1))
        self.assertEqual(facts[deleted]["language_stats"], {".config": 1})
        self.assertEqual(facts[deleted]["deletions"], 1)

    def test_remote_branches_bare_repository_and_conflicts(self):
        base = self.commit("base.txt", "base\n", "initial", 1)
        self.git("update-ref", "refs/remotes/origin/remote-only", base)
        self.git("symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/remote-only")
        result = self.collect()
        self.assertEqual(result["refs"], {"main": base, "remote-only": base})
        bare = self.root / "bare.git"
        self.git("clone", "--bare", str(self.repo), str(bare))
        result = collector.collect(collector.Git(bare), "task", 1, "repository", 0)
        self.assertEqual(result["refs"], {"main": base})
        next_sha = self.commit("new.txt", "new\n", "new", 2)
        self.git("update-ref", "refs/remotes/origin/main", base)
        with self.assertRaisesRegex(ValueError, "conflicting"):
            self.collect()
        self.assertNotEqual(base, next_sha)

    def test_cli_atomic_output_and_shallow_failure(self):
        base = self.commit("base.txt", "base\n", "initial", 1)
        known = self.root / "known.json"
        known.write_text(json.dumps([base]), encoding="utf-8")
        output = self.root / "artifact.json"
        command = [sys.executable, str(SCRIPT), "--repository", str(self.repo),
                   "--task-id", "task", "--generation", "3", "--repository-id", "repo-id",
                   "--coverage-start", "0", "--known-complete", str(known), "--output", str(output)]
        result = subprocess.run(command, env=self.environment, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        artifact = json.loads(output.read_text(encoding="utf-8"))
        self.assertEqual(artifact["repository"], "repo-id")
        self.assertEqual(artifact["commits"], [])
        self.assertEqual(artifact["ref_commits"], {"main": [base]})
        before = output.read_bytes()
        (self.repo / ".git" / "shallow").write_text(base + "\n", encoding="ascii")
        failed = subprocess.run(command, env=self.environment, capture_output=True)
        self.assertNotEqual(failed.returncode, 0)
        self.assertIn(b"shallow", failed.stderr)
        self.assertEqual(output.read_bytes(), before)

    def test_force_push_and_deleted_branch_rebuild_membership(self):
        base, main, feature, merge = self.history()
        self.git("reset", "--hard", base)
        for branch in ("copy", "feature"):
            self.git("branch", "-D", branch)
        result = self.collect(known=(base, main, feature, merge))
        self.assertEqual(result["refs"], {"main": base})
        self.assertEqual(result["ref_commits"], {"main": [base]})
        self.assertEqual(result["commits"], [])

    def test_empty_repository_and_missing_history_fail(self):
        result = self.collect()
        self.assertEqual((result["refs"], result["commits"], result["ref_commits"]), ({}, [], {}))
        base = self.commit("base.txt", "base\n", "initial", 1)
        self.commit("new.txt", "new\n", "new", 2)
        missing = self.repo / ".git" / "objects" / base[:2] / base[2:]
        missing.chmod(0o600)
        missing.unlink()
        with self.assertRaises(ValueError):
            self.collect()

    def test_full_history_includes_pre_epoch_dates(self):
        base = self.commit("base.txt", "base\n", "initial", 1)
        # Write a valid raw commit: Git's human date parser rejects negative
        # timestamps on some versions, although its object format supports them.
        tree = self.git("rev-parse", base + "^{tree}")
        content = (f"tree {tree}\nauthor Early Author <early@example.invalid> -1 +0000\n"
                   "committer Early Author <early@example.invalid> -1 +0000\n\npre-epoch\n").encode()
        raw = f"commit {len(content)}\0".encode() + content
        sha = hashlib.sha1(raw).hexdigest()
        directory = self.repo / ".git" / "objects" / sha[:2]
        directory.mkdir(exist_ok=True)
        (directory / sha[2:]).write_bytes(zlib.compress(raw))
        self.git("update-ref", "refs/heads/early", sha)
        full = self.collect()
        self.assertEqual(full["ref_commits"]["early"], [sha])
        fact = next(fact for fact in full["commits"] if fact["sha"] == sha)
        self.assertEqual(fact["committer_date"], -1000)
        self.assertTrue(fact["complete"])
        limited = self.collect(start=1)
        self.assertEqual(limited["ref_commits"]["early"], [])
        self.assertFalse(next(fact for fact in limited["commits"] if fact["sha"] == sha)["complete"])


class PublicCollectorTests(unittest.TestCase):
    def validate_public_repository(self, endpoint):
        self.assertTrue(endpoint.startswith(("https://", "git://")), "public tests accept HTTPS or anonymous Git transport")
        environment = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
        environment.update(GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull, GIT_TERMINAL_PROMPT="0")
        flags = {"creationflags": subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0}
        started = time.monotonic()
        with tempfile.TemporaryDirectory(prefix="public-contribution-collector-") as temporary:
            root = Path(temporary)
            repository = root / "repository.git"

            def git(*arguments):
                result = subprocess.run(["git", "-c", "credential.helper=", *arguments],
                                        env=environment, capture_output=True, timeout=180, **flags)
                self.assertEqual(result.returncode, 0, result.stderr.decode("utf-8", "replace"))
                return result.stdout

            # A bare clone fetches every branch without fetching arbitrary pull
            # request refs or executing target repository code or submodules.
            git("clone", "--bare", endpoint, str(repository))
            expected_refs = {}
            expected_members = {}
            for row in git("-C", str(repository), "for-each-ref", "--format=%(refname)%09%(objectname)", "refs/heads/").splitlines():
                name, sha = row.decode("ascii").split("\t")
                name = name.removeprefix("refs/heads/")
                expected_refs[name] = sha
                expected_members[name] = set(git("-C", str(repository), "rev-list", sha).decode("ascii").splitlines())
            expected_union = set().union(*expected_members.values())
            self.assertTrue(expected_union)
            print(f"\nPUBLIC fetched {endpoint}: {len(expected_refs)} branches, {len(expected_union)} unique commits", flush=True)

            def collect(label, coverage_start=0, known=None):
                output = root / (label + ".json")
                command = [sys.executable, str(SCRIPT), "--repository", str(repository),
                           "--task-id", "public-test", "--generation", "1", "--repository-id", endpoint,
                           "--coverage-start", str(coverage_start), "--output", str(output)]
                if known is not None:
                    known_file = root / "known.json"
                    known_file.write_text(json.dumps(sorted(known)), encoding="utf-8")
                    command.extend(["--known-complete", str(known_file)])
                result = subprocess.run(command, env=environment, capture_output=True, timeout=600, **flags)
                self.assertEqual(result.returncode, 0, result.stderr.decode("utf-8", "replace"))
                return json.loads(output.read_text(encoding="utf-8"))

            full = collect("full")
            self.assertTrue(full["complete"])
            self.assertEqual(full["refs"], expected_refs)
            self.assertEqual({name: set(shas) for name, shas in full["ref_commits"].items()}, expected_members)
            facts = {fact["sha"]: fact for fact in full["commits"]}
            self.assertEqual(set(facts), expected_union)
            self.assertTrue(all(fact["complete"] for fact in facts.values()))

            # One independent Git log computes all-parent diff totals in a
            # batch. Initial commits retain upstream's zero-line asymmetry.
            totals = {sha: [0, 0] for sha in expected_union}
            log = git("-C", str(repository), "log", "--branches", "-m", "--no-renames",
                      "--no-ext-diff", "--no-textconv", "--numstat", "--format=%x1e%H")
            for record in log.split(b"\x1e")[1:]:
                rows = record.splitlines()
                sha = rows[0].decode("ascii")
                if not facts[sha]["parents"]:
                    continue
                for row in rows[1:]:
                    columns = row.split(b"\t", 2)
                    if len(columns) == 3 and columns[0] != b"-":
                        totals[sha][0] += int(columns[0])
                        totals[sha][1] += int(columns[1])
            for sha, fact in facts.items():
                self.assertEqual([fact["additions"], fact["deletions"]], totals[sha], sha)

            repeated = collect("known-repeat", known=expected_union)
            self.assertEqual(repeated["commits"], [])
            self.assertEqual(repeated["refs"], full["refs"])
            self.assertEqual(repeated["ref_commits"], full["ref_commits"])
            dates = sorted({fact["committer_date"] for fact in facts.values()})
            cutoff = dates[len(dates) // 2]
            self.assertNotEqual(cutoff, 0, "choose a non-sentinel coverage boundary")
            limited = collect("coverage", coverage_start=cutoff)
            limited_facts = {fact["sha"]: fact for fact in limited["commits"]}
            self.assertEqual(set(limited_facts), expected_union, "old graph metadata must remain available")
            for name, members in expected_members.items():
                expected = {sha for sha in members if facts[sha]["committer_date"] >= cutoff}
                self.assertEqual(set(limited["ref_commits"][name]), expected)
            for sha, fact in limited_facts.items():
                in_range = facts[sha]["committer_date"] >= cutoff
                self.assertEqual(fact["complete"], in_range)
                self.assertEqual(fact["parents"], facts[sha]["parents"])
                if in_range:
                    self.assertEqual(fact, facts[sha])
                else:
                    self.assertEqual((fact["additions"], fact["deletions"], fact["language_stats"]), (0, 0, {}))
            print(json.dumps({"result": "PASS", "endpoint": endpoint, "refs": expected_refs,
                              "unique_commits": len(expected_union),
                              "branch_occurrences": sum(map(len, expected_members.values())),
                              "all_parent_line_totals": [sum(pair[0] for pair in totals.values()), sum(pair[1] for pair in totals.values())],
                              "known_repeat_new_facts": 0, "coverage_start": cutoff,
                              "coverage_unique_commits": sum(fact["complete"] for fact in limited_facts.values()),
                              "seconds": round(time.monotonic() - started, 2)}), flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument("--public-repository", action="append", default=[], metavar="URL")
    options, remaining = parser.parse_known_args()
    for index, endpoint in enumerate(options.public_repository):
        def public_test(self, endpoint=endpoint):
            self.validate_public_repository(endpoint)
        setattr(PublicCollectorTests, f"test_public_repository_{index + 1}", public_test)
    unittest.main(argv=[sys.argv[0], *remaining], verbosity=2)
