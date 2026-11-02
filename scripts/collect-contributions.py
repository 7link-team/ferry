#!/usr/bin/env python3
"""Collect a version-1 contribution artifact from existing, complete Git history.

This one-shot command neither fetches nor runs repository code. The caller owns
the cache/fetch lock and supplies the server's known-complete SHA set for analysis
version 1. Membership is always rebuilt from frozen branch targets, even when
every reachable commit's analysis is already known.
"""

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile


SHA = re.compile(r"(?:[0-9a-fA-F]{40}|[0-9a-fA-F]{64})\Z")


class Git:
    def __init__(self, repository):
        self.repository = str(Path(repository).resolve())
        self.environment = dict(os.environ)
        for key in ("GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_COMMON_DIR",
                    "GIT_OBJECT_DIRECTORY", "GIT_ALTERNATE_OBJECT_DIRECTORIES"):
            self.environment.pop(key, None)
        self.environment.update(GIT_TERMINAL_PROMPT="0", GIT_NO_LAZY_FETCH="1")

    def run(self, *arguments):
        process = subprocess.run(
            ["git", "--no-replace-objects", "-c", "core.quotePath=false",
             "-C", self.repository, *arguments], env=self.environment,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0,
        )
        if process.returncode:
            raise ValueError(process.stderr.decode("utf-8", "replace").strip())
        return process.stdout


def branch_targets(git):
    targets = {}
    rows = git.run("for-each-ref", "--format=%(refname)%00%(objectname)%00%(symref)",
                   "refs/heads/", "refs/remotes/")
    for row in rows.splitlines():
        reference, sha, symbolic = row.decode("utf-8", "replace").split("\0")
        if symbolic:
            continue
        if reference.startswith("refs/heads/"):
            name = reference[len("refs/heads/"):]
        else:
            remote_branch = reference[len("refs/remotes/"):]
            if "/" not in remote_branch:
                raise ValueError("remote branch has no remote name: " + reference)
            name = remote_branch.split("/", 1)[1]
        if name in targets and targets[name] != sha:
            raise ValueError("conflicting local/remote branch targets: " + name)
        targets[name] = sha
    return dict(sorted(targets.items()))


def diff_rows(git, sha, parent):
    options = ("--no-ext-diff", "--no-textconv", "--no-renames", "--numstat", "-z")
    data = git.run("diff", *options, parent, sha, "--")
    for row in data.split(b"\0"):
        if row:
            added, deleted, filename = row.split(b"\t", 2)
            yield (0 if added == b"-" else int(added),
                   0 if deleted == b"-" else int(deleted),
                   filename.decode("utf-8", "replace"))


def commit_metadata(git, sha):
    # Raw headers preserve pre-epoch dates: Git's %at/%ct formatting returns
    # empty strings for negative timestamps even though the raw object has them.
    headers, message = git.run("cat-file", "commit", sha).split(b"\n\n", 1)
    fields = {}
    parent_shas = []
    for line in headers.splitlines():
        if line.startswith(b"parent "):
            parent_shas.append(line[7:].decode("ascii"))
        elif line.startswith((b"author ", b"committer ", b"encoding ")):
            key, value = line.split(b" ", 1)
            fields[key] = value
    identity = re.compile(rb"(.*) <([^>]*)> (-?\d+) [+-]\d{4}\Z")
    author = identity.fullmatch(fields[b"author"])
    committer = identity.fullmatch(fields[b"committer"])
    if author is None or committer is None:
        raise ValueError("invalid commit identity: " + sha)
    encoding = fields.get(b"encoding", b"utf-8").decode("ascii")
    return {
        "sha": sha, "parents": parent_shas,
        "message": message.decode(encoding, "replace"),
        "author_name": author[1].decode(encoding, "replace"),
        "author_email": author[2].decode(encoding, "replace"),
        "author_date": int(author[3]) * 1000, "committer_date": int(committer[3]) * 1000,
    }


def analyze(git, sha, complete=True):
    metadata = commit_metadata(git, sha)
    parent_shas = metadata["parents"]
    additions = deletions = 0
    languages = {}
    for index, parent in enumerate(parent_shas if complete else []):
        for added, deleted, filename in diff_rows(git, sha, parent):
            additions += added
            deletions += deleted
            if index == 0:
                # Go filepath.Ext semantics, including dotfiles and empty suffix.
                basename = filename.rsplit("/", 1)[-1]
                extension = basename[basename.rfind("."):] if "." in basename else ""
                languages[extension] = languages.get(extension, 0) + 1
    if complete and not parent_shas:
        # Upstream's initial commit has zero line totals, but counts all files.
        for row in git.run("ls-tree", "-r", "-z", sha, "--").split(b"\0"):
            if not row:
                continue
            tree_metadata, filename = row.split(b"\t", 1)
            if tree_metadata.split()[1] != b"blob":
                continue
            basename = filename.decode("utf-8", "replace").rsplit("/", 1)[-1]
            extension = basename[basename.rfind("."):] if "." in basename else ""
            languages[extension] = languages.get(extension, 0) + 1
    return {
        **metadata,
        "additions": additions, "deletions": deletions,
        "language_stats": dict(sorted(languages.items())), "complete": complete,
    }


def collect(git, task_id, generation, repository_id, coverage_start, known_complete=()):
    if git.run("rev-parse", "--is-shallow-repository").strip() != b"false":
        raise ValueError("shallow history cannot produce complete coverage")
    refs = branch_targets(git)
    memberships = {}
    reachable = {}
    for name, target in refs.items():
        members = []
        # Do not use --since: commit dates can decrease and then increase along
        # a parent chain. Visit every ancestor and filter each date individually.
        for row in git.run("rev-list", "--timestamp", target, "--").splitlines():
            date, sha = row.decode("ascii").split()
            timestamp = int(date) * 1000
            if timestamp == 0 or timestamp > 9223372036854775807:
                timestamp = commit_metadata(git, sha)["committer_date"]
            reachable[sha] = timestamp
            if coverage_start == 0 or timestamp >= coverage_start:
                members.append(sha)
        memberships[name] = sorted(members)
    # Keep the entire parent graph so the importer can independently prove that
    # every required ancestor is represented. Old commits need metadata only.
    commits = [analyze(git, sha, coverage_start == 0 or reachable[sha] >= coverage_start)
               for sha in sorted(reachable.keys() - set(known_complete))]
    return {
        "schema_version": 1, "analysis_version": 1,
        "task_id": task_id, "generation": generation, "repository": repository_id,
        "coverage_start": coverage_start, "refs": refs, "ref_commits": memberships,
        "commits": commits, "complete": True,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository", required=True)
    parser.add_argument("--task-id", required=True)
    parser.add_argument("--generation", type=int, required=True)
    parser.add_argument("--repository-id", required=True)
    parser.add_argument("--coverage-start", type=int, required=True, help="committer timestamp in milliseconds; 0 for all history")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--known-complete", type=Path, help="JSON array of complete SHAs at analysis_version=1")
    args = parser.parse_args()
    temporary = None
    try:
        known = json.loads(args.known_complete.read_text(encoding="utf-8")) if args.known_complete else []
        if not isinstance(known, list) or any(not isinstance(sha, str) or not SHA.fullmatch(sha) for sha in known):
            raise ValueError("known-complete must be a JSON array of full commit SHAs")
        artifact = collect(Git(args.repository), args.task_id, args.generation,
                           args.repository_id, args.coverage_start, (sha.lower() for sha in known))
        # Never expose a partly written complete artifact to the importer.
        with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8", newline="\n",
                                         dir=args.output.parent, delete=False) as output:
            temporary = Path(output.name)
            json.dump(artifact, output, ensure_ascii=False, separators=(",", ":"))
            output.write("\n")
        os.replace(temporary, args.output)
    except (ValueError, OSError, LookupError, KeyError) as error:
        print("Contribution collection failed: " + str(error), file=sys.stderr)
        return 1
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
