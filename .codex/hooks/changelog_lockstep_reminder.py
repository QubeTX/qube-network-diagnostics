#!/usr/bin/env python3
"""PostToolUse reminder: keep CHANGELOG.md and HUMAN_CHANGELOG.md in lockstep.

This repo's "Changelog rule" (see CLAUDE.md) requires that every edit to
CHANGELOG.md be matched by a corresponding plain-English entry in
HUMAN_CHANGELOG.md *in the same commit*. This hook fires after an Edit/Write to
CHANGELOG.md and, if HUMAN_CHANGELOG.md has NOT also been modified in the working
tree, emits a non-blocking reminder.

Non-blocking by design: it never denies or fails the tool. It only surfaces a
systemMessage (exit 0). If HUMAN_CHANGELOG.md is already dirty in the working
tree, the lockstep is presumed satisfied and the hook stays silent.
"""

import json
import os
import subprocess
import sys


def is_changelog(path: str) -> bool:
    if not path:
        return False
    base = os.path.basename(path.replace("\\", "/")).lower()
    return base == "changelog.md"


def human_changelog_dirty(project_dir: str) -> bool:
    """True if HUMAN_CHANGELOG.md shows as modified/added in `git status`.

    Fail-safe: if git can't be queried, return True (assume satisfied) so the
    hook stays quiet rather than nagging on every CHANGELOG edit.
    """
    try:
        out = subprocess.run(
            ["git", "status", "--porcelain", "--", "HUMAN_CHANGELOG.md"],
            cwd=project_dir,
            capture_output=True,
            text=True,
            timeout=10,
        )
    except Exception:
        return True
    if out.returncode != 0:
        return True
    return bool(out.stdout.strip())


def main() -> None:
    raw = sys.stdin.read()
    try:
        data = json.loads(raw)
    except Exception:
        sys.exit(0)

    tool_input = data.get("tool_input", {}) or {}
    file_path = tool_input.get("file_path") or tool_input.get("path") or ""
    if not is_changelog(file_path):
        sys.exit(0)

    project_dir = (
        os.environ.get("CLAUDE_PROJECT_DIR")
        or data.get("cwd")
        or os.getcwd()
    )

    if human_changelog_dirty(project_dir):
        # Lockstep already in progress — stay quiet.
        sys.exit(0)

    message = (
        "Changelog lockstep reminder: you edited CHANGELOG.md but "
        "HUMAN_CHANGELOG.md is unchanged in the working tree. This repo's "
        "Changelog rule requires a matching plain-English entry in "
        "HUMAN_CHANGELOG.md in the SAME commit (no version numbers, file paths, "
        "function names, or jargon — just what changed and why it matters). "
        "Update HUMAN_CHANGELOG.md before committing."
    )
    print(json.dumps({"systemMessage": message, "suppressOutput": False}))
    sys.exit(0)


if __name__ == "__main__":
    main()
