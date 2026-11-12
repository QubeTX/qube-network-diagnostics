#!/usr/bin/env python3
"""Recognize a narrow docs-only diff; ambiguous changes keep the full CI path."""

import json
import os
from pathlib import Path
import re
import subprocess
import sys


ROOT_DOCS = frozenset({
    "README.md", "AGENTS.md", "CLAUDE.md", "CODEX_PROJECT.md",
    "ND300-Project-Plan.md",
})
SHA = re.compile(r"[0-9a-f]{40}\Z")


class ScopeError(ValueError):
    """The complete change set cannot be established safely."""


def git(*args):
    return subprocess.run(
        ["git", *args], check=True, stdout=subprocess.PIPE,
        stderr=subprocess.PIPE, timeout=60,
    ).stdout


def commit(value):
    if not isinstance(value, str) or not SHA.fullmatch(value) or value == "0" * 40:
        raise ScopeError("missing or invalid commit identity")
    return value


def parse_raw_diff(data):
    """Parse --raw -z --no-renames so both sides of a rename are considered."""
    fields = data.split(b"\0")
    if fields.pop() != b"" or len(fields) % 2:
        raise ScopeError("malformed NUL-delimited change list")
    changes = []
    for index in range(0, len(fields), 2):
        header = fields[index].decode("ascii").split()
        if len(header) != 5 or not header[0].startswith(":"):
            raise ScopeError("malformed raw change metadata")
        old_mode, new_mode = header[0][1:], header[1]
        old_oid, new_oid, status = header[2:]
        if not all(re.fullmatch(r"[0-7]{6}", mode) for mode in (old_mode, new_mode)):
            raise ScopeError("invalid file mode")
        if not SHA.fullmatch(old_oid) or not SHA.fullmatch(new_oid):
            raise ScopeError("invalid blob identity")
        changes.append((status, old_mode, new_mode, fields[index + 1].decode("utf-8")))
    return changes


def documentation_change(change):
    status, old_mode, new_mode, path = change
    expected_modes = {
        "A": ("000000", "100644"),
        "M": ("100644", "100644"),
        "D": ("100644", "000000"),
    }
    if expected_modes.get(status) != (old_mode, new_mode):
        return False
    # Do not normalize an unusual name into the allowlist. Executable files,
    # symlinks, submodules and type/mode changes also remain full above.
    if "\\" in path or any(ord(char) < 32 or ord(char) == 127 for char in path):
        return False
    if any(part in ("", ".", "..") for part in path.split("/")):
        return False
    if path == "README.md":
        # The archive and Cargo metadata require this file to exist.
        return status == "M"
    return path in ROOT_DOCS or (
        path.startswith(("docs/", ".tasks/")) and path.endswith(".md")
    )


def diff(base, head, run):
    return parse_raw_diff(run(
        "diff", "--raw", "--no-abbrev", "-z", "--no-renames",
        "--no-ext-diff", "--no-textconv", base, head, "--",
    ))


def classify(env, event, run=git, check_docs=False):
    event_name = env.get("GITHUB_EVENT_NAME", "")
    ref = env.get("GITHUB_REF", "")
    # Explicit releases, repairs and preflights never use a documentation skip.
    if event_name not in ("push", "pull_request") or ref.startswith("refs/tags/"):
        return False, "explicit-or-unsupported-event", 0
    if run("rev-parse", "--is-shallow-repository").decode("ascii").strip() != "false":
        raise ScopeError("complete repository history is unavailable")
    if event.get("repository", {}).get("full_name") != env.get("GITHUB_REPOSITORY"):
        raise ScopeError("event repository does not match workflow")
    checkout = run("rev-parse", "--verify", "HEAD").decode("ascii").strip()
    source = commit(env.get("GITHUB_SHA"))
    if checkout != source:
        raise ScopeError("checkout does not match workflow commit")

    if event_name == "pull_request":
        pr = event.get("pull_request", {})
        number = event.get("number")
        if not isinstance(number, int) or number < 1 or ref != f"refs/pull/{number}/merge":
            raise ScopeError("not a pull-request merge context")
        base, head = commit(pr.get("base", {}).get("sha")), commit(pr.get("head", {}).get("sha"))
        if pr.get("base", {}).get("repo", {}).get("full_name") != env.get("GITHUB_REPOSITORY"):
            raise ScopeError("pull-request base repository mismatch")
        parents = run("rev-list", "--parents", "-n", "1", source).decode("ascii").split()
        if parents != [source, base, head]:
            raise ScopeError("pull-request merge parents do not match event")
        bases = run("merge-base", "--all", base, head).decode("ascii").split()
        if len(bases) != 1:
            raise ScopeError("ambiguous pull-request merge base")
        merge_base = commit(bases[0])
        # Cover the complete PR, not just its latest commit. Also inspect the
        # effective merge result so merge resolution cannot hide a code change.
        changes = diff(merge_base, head, run) + diff(base, source, run)
    else:
        if ref != "refs/heads/main" or event.get("ref") != ref:
            raise ScopeError("not a main-branch push")
        if event.get("created") is not False or event.get("deleted") is not False:
            raise ScopeError("new or deleted branch has no ordinary push range")
        base, head = commit(event.get("before")), commit(event.get("after"))
        if head != source:
            raise ScopeError("push after commit does not match workflow")
        run("merge-base", "--is-ancestor", base, head)
        changes = diff(base, head, run)

    if not changes:
        return False, "empty-change-set", 0
    docs_only = all(documentation_change(change) for change in changes)
    if check_docs:
        if not docs_only:
            raise ScopeError("documentation check requires a proven docs-only diff")
        # Check actual Markdown text without imposing a prose style or treating
        # Markdown's intentional trailing spaces as a formatting error.
        paths = {path for status, _, _, path in changes if status != "D"}
        for path in paths:
            content = run("show", f"{source}:{path}").decode("utf-8")
            if "\0" in content:
                raise ScopeError("documentation contains binary data")
    return docs_only, "documentation-only" if docs_only else "full-validation-input", len(changes)


def main():
    if sys.argv[1:] not in ([], ["--check-docs"]):
        print("Usage: classify-ci-changes.py [--check-docs]", file=sys.stderr)
        return 2
    check_docs = sys.argv[1:] == ["--check-docs"]
    try:
        event = json.loads(Path(os.environ["GITHUB_EVENT_PATH"]).read_text(encoding="utf-8"))
        if not isinstance(event, dict):
            raise ScopeError("event must be an object")
        docs_only, reason, count = classify(os.environ, event, check_docs=check_docs)
    except (KeyError, OSError, ValueError, UnicodeError, AttributeError,
            subprocess.SubprocessError):
        # Deliberately avoid dumping untrusted paths/event content into workflow
        # command syntax. Missing history or malformed data costs a full run.
        if check_docs:
            print("Documentation check could not verify the complete text change.", file=sys.stderr)
            return 1
        docs_only, reason, count = False, "unproven-change-set", 0
    if check_docs and not docs_only:
        print("Documentation check requires a proven documentation-only event.", file=sys.stderr)
        return 1
    value = str(docs_only).lower()
    print(f"docs_only={value}; reason={reason}; inspected_changes={count}")
    output = os.environ.get("GITHUB_OUTPUT")
    if output:
        with open(output, "a", encoding="utf-8") as stream:
            stream.write(f"docs_only={value}\nreason={reason}\n")
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as stream:
            stream.write(f"### Change scope\n\nDocumentation only: **{value}**. "
                         f"Reason: `{reason}`. Inspected change records: {count}.\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
