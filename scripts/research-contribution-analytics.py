"""Reproduce analytics design cases using disposable Git history and SQLite.

Research evidence only: this does not exercise Ferry or run gitinsight.
No network, credentials, existing repositories or persistent databases are used.
"""

import json
import os
from pathlib import Path
import sqlite3
import subprocess
import tempfile


def main():
    with tempfile.TemporaryDirectory(prefix="contribution-research-") as temporary:
        root = Path(temporary)
        environment = dict(os.environ)
        environment.update(
            GIT_CONFIG_NOSYSTEM="1",
            GIT_CONFIG_GLOBAL=os.devnull,
            GIT_TERMINAL_PROMPT="0",
        )
        # Do not inherit a calling Git process's repository/index configuration.
        for key in ("GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_COMMON_DIR"):
            environment.pop(key, None)

        def git(*arguments, day=None, author_day=None):
            child_environment = dict(environment)
            if day:
                child_environment["GIT_COMMITTER_DATE"] = f"2025-01-{day:02}T12:00:00Z"
                child_environment["GIT_AUTHOR_DATE"] = (
                    f"2025-01-{author_day or day:02}T12:00:00Z"
                )
            return subprocess.run(
                ["git", "-c", "core.autocrlf=false", "-c", "commit.gpgsign=false", *arguments],
                cwd=root,
                env=child_environment,
                check=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                encoding="utf-8",
                creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0,
            ).stdout.strip()

        def commit(filename, content, message, day, author_day=None):
            (root / filename).write_text(content, encoding="utf-8", newline="\n")
            git("add", "--", filename)
            git("commit", "-m", message, day=day, author_day=author_day)
            return git("rev-parse", "HEAD")

        def reachable(reference):
            return set(git("rev-list", reference).splitlines())

        def branch_sets():
            return {
                branch: reachable(branch)
                for branch in git("for-each-ref", "--format=%(refname)", "refs/heads/").splitlines()
            }

        def stats(sha):
            parents = git("show", "-s", "--format=%P", sha).split()
            comparisons = [
                git("diff", "--no-renames", "--numstat", parent, sha)
                for parent in parents
            ] if parents else [git("diff-tree", "--root", "--no-commit-id", "--numstat", "-r", sha)]
            additions = deletions = 0
            for comparison in comparisons:
                for row in comparison.splitlines():
                    added, deleted, _ = row.split("\t", 2)
                    additions += int(added)
                    deletions += int(deleted)
            return additions, deletions

        git("init", "--initial-branch=main")
        git("config", "user.name", "Research Author")
        git("config", "user.email", "research@example.invalid")
        base = commit("base.txt", "base\n", "chore: initial", 1)
        main_commit = commit("main.txt", "one\ntwo\n", "feat: main", 3, author_day=1)
        git("checkout", "-b", "feature", base)
        feature = commit("feature.txt", "a\nb\nc\n", "fix: feature", 2)
        git("checkout", "main")
        git("merge", "--no-ff", "feature", "-m", "merge: feature", day=4)
        merge = git("rev-parse", "HEAD")
        git("branch", "copy", merge)
        git("checkout", "-b", "cherry", base)
        git("cherry-pick", feature, day=5)
        cherry = git("rev-parse", "HEAD")

        branches = branch_sets()
        union = set().union(*branches.values())
        assert len(union) == 5
        assert sum(map(len, branches.values())) == 12
        assert cherry != feature and stats(cherry) == stats(feature)
        assert stats(merge) == (5, 0)
        first_parent = git("show", "-s", "--format=%P", merge).split()[0]
        first_parent_added = sum(
            int(row.split("\t", 1)[0])
            for row in git("diff", "--numstat", first_parent, merge).splitlines()
        )
        assert first_parent_added == 3
        assert sum(stats(sha)[0] for sha in union) == 14
        assert sum(stats(sha)[0] for sha in union if sha != merge) == 9
        dates = git("show", "-s", "--format=%aI%n%cI", main_commit).splitlines()
        assert dates[0] != dates[1]

        # A known SHA can acquire a new branch relation without another diff.
        git("branch", "new", feature)
        after_new_branch = branch_sets()
        assert set().union(*after_new_branch.values()) == union
        assert len(after_new_branch["refs/heads/new"]) == 2

        git("checkout", "main")
        new_sha = commit("later.txt", "later\n", "feat: later", 6)
        after_push = set().union(*branch_sets().values())
        assert after_push - union == {new_sha}
        git("reset", "--hard", base)
        for branch in ("copy", "feature", "new"):
            git("branch", "-D", branch)
        after_rewrite = set().union(*branch_sets().values())
        assert after_rewrite == {base, cherry}
        assert after_push - after_rewrite == {main_commit, feature, merge, new_sha}

        with sqlite3.connect(":memory:") as database:
            database.execute("CREATE TABLE facts (branch TEXT, sha TEXT, is_merge INTEGER, additions INTEGER)")
            rows = [("main", "normal", 0, 10), ("main", "merge", 1, 5)]
            for _ in range(2):
                database.execute("DELETE FROM facts WHERE branch='main' AND is_merge=0")
                database.executemany("INSERT INTO facts VALUES (?, ?, ?, ?)", rows)
            counts = dict(database.execute("SELECT sha, COUNT(*) FROM facts GROUP BY sha"))
            assert counts == {"normal": 1, "merge": 2}
            database.execute("DELETE FROM facts")
            database.executemany("INSERT INTO facts VALUES (?, ?, ?, ?)", [
                ("main", "shared", 0, 10), ("feature", "shared", 0, 10),
            ])
            heatmap = database.execute("SELECT COUNT(DISTINCT sha), SUM(additions) FROM facts").fetchone()
            assert heatmap == (1, 20)

        assert sum(max(a - d, 0) for a, d in [(10, 0), (0, 8)]) == 10
        assert max(sum(a - d for a, d in [(10, 0), (0, 8)]), 0) == 2
        print(json.dumps({
            "result": "PASS",
            "scope": "disposable Git and SQLite models; not application integration",
            "initial_unique_commits": len(union),
            "initial_branch_occurrences": sum(map(len, branches.values())),
            "all_parent_merge_additions": stats(merge)[0],
            "first_parent_merge_additions": first_parent_added,
            "new_branch_new_analysis_required": 0,
            "push_new_commits": len(after_push - union),
            "unreachable_after_rewrite": len(after_push - after_rewrite),
            "replacement_model_counts": counts,
            "heatmap_model": {"unique_commits": heatmap[0], "additions": heatmap[1]},
        }, indent=2))


if __name__ == "__main__":
    main()
