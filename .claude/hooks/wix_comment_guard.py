#!/usr/bin/env python3
"""PreToolUse guard: block WiX edits that put `--` inside an XML comment body.

Background
----------
WiX 3's `candle.exe` rejects a double hyphen (`--`) that appears *inside* an XML
comment body with error CNDL0104. This is legal XML to many editors but illegal
to candle's stricter parser. cargo-dist swallows candle's stderr, so the failure
only surfaces at release time on the Windows MSI build -- exactly the bug that
broke the v3.2.0 MSI release (fixed in e4a5060). This hook catches it locally,
at edit time, before it can reach CI.

Contract
--------
Runs as a Claude Code PreToolUse hook on Edit / Write / MultiEdit targeting
`wix/**.wxs` and `wix-corporate/**.wxs`. Reads the hook payload as JSON on stdin,
reconstructs the file content that the tool *would* produce, scans every XML
comment body in it, and -- if any comment body contains `--` -- emits a deny
decision (permissionDecision=deny + exit 2) naming CNDL0104 and the offending
comment. A clean edit is allowed (exit 0, silent).

What counts as a violation
--------------------------
An XML comment is `<!--` ... `-->`. The comment *body* is the text strictly
between the opening `<!--` and the next `-->`. A `--` anywhere in that body is a
CNDL0104 violation. The `--` that is part of the `<!--`/`-->` delimiters
themselves is NOT a violation, and `--` inside attribute *values*
(e.g. ExeCommand='x --flag') is NOT a violation because it is not inside a
comment. Multi-line comment bodies and multiple comments per file are handled.

Fail-open
---------
Any unexpected error (bad JSON, unknown schema) exits 0 (allow). A guard that
can't parse its input must not block legitimate work -- the worst case is the
pre-existing behavior (the bug surfaces at release), never a wedged editor.
"""

import json
import re
import sys

# Only guard WiX source files: anything under wix/ or wix-corporate/ ending in
# .wxs. Path separators are normalized to `/` before matching so Windows
# backslash paths (sent as `\\` in the JSON payload) are handled the same as
# POSIX paths. Case-insensitive for Windows path case-insensitivity.
WIX_PATH_RE = re.compile(
    r"(?:^|/)(?:wix|wix-corporate)/.*\.wxs$",
    re.IGNORECASE,
)


def is_wix_path(path: str) -> bool:
    if not path:
        return False
    normalized = path.replace("\\", "/")
    return WIX_PATH_RE.search(normalized) is not None


def comment_bodies(content: str):
    """Yield (body, full_comment) for every `<!-- ... -->` in content.

    The body is the text strictly between the opening `<!--` and the next `-->`.
    `re.DOTALL` lets a single comment span multiple lines. The `.*?` is lazy so
    `<!-- a --><!-- b -->` yields two comments, not one greedy span.
    """
    for m in re.finditer(r"<!--(.*?)-->", content, re.DOTALL):
        yield m.group(1), m.group(0)


def find_violations(content: str):
    """Return a list of offending comment snippets (each contains `--` in body)."""
    violations = []
    for body, full in comment_bodies(content):
        if "--" in body:
            snippet = full.strip()
            if len(snippet) > 200:
                snippet = snippet[:200] + "..."
            violations.append(snippet)
    return violations


def candidate_contents(tool_name: str, tool_input: dict):
    """Reconstruct the post-edit text(s) the tool would produce.

    - Write: the new file content is `content`.
    - Edit: the replacement text is `new_string` (we scan the replacement, since
      that is the only text the edit introduces; an existing-but-untouched bad
      comment would already be in the file and is out of scope for this edit).
    - MultiEdit: every edit's `new_string`.
    Returns a list of strings to scan.
    """
    out = []
    if tool_name == "Write":
        c = tool_input.get("content")
        if isinstance(c, str):
            out.append(c)
    elif tool_name == "Edit":
        c = tool_input.get("new_string")
        if isinstance(c, str):
            out.append(c)
    elif tool_name == "MultiEdit":
        for edit in tool_input.get("edits", []) or []:
            c = edit.get("new_string")
            if isinstance(c, str):
                out.append(c)
    return out


def deny(message: str) -> None:
    """Emit a PreToolUse deny decision and exit 2 (blocking)."""
    payload = {
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": message,
        },
        "systemMessage": message,
    }
    print(json.dumps(payload))
    sys.stderr.write(message + "\n")
    sys.exit(2)


def main() -> None:
    raw = sys.stdin.read()
    try:
        data = json.loads(raw)
    except Exception:
        # Can't parse the payload -> fail open.
        sys.exit(0)

    tool_name = data.get("tool_name", "")
    tool_input = data.get("tool_input", {}) or {}

    file_path = tool_input.get("file_path") or tool_input.get("path") or ""
    if not is_wix_path(file_path):
        sys.exit(0)

    all_violations = []
    for content in candidate_contents(tool_name, tool_input):
        all_violations.extend(find_violations(content))

    if not all_violations:
        sys.exit(0)

    lines = [
        "BLOCKED: WiX CNDL0104 risk — a double hyphen (`--`) appears inside an "
        "XML comment body.",
        "",
        f"File: {file_path}",
        "",
        "WiX 3's candle.exe rejects `--` inside a comment body (error CNDL0104). "
        "cargo-dist hides candle's stderr, so this only surfaces at release time "
        "on the Windows MSI build — this is exactly what broke the v3.2.0 MSI "
        "release.",
        "",
        "Offending comment(s):",
    ]
    for v in all_violations:
        lines.append(f"  {v}")
    lines += [
        "",
        "Fix: rewrite the comment body so it contains no `--`. Use an em dash "
        "(—), a single hyphen, the word 'to', or 'minus minus' instead. The "
        "`<!--` / `-->` delimiters themselves are fine; only `--` BETWEEN them "
        "is illegal. `--` inside attribute values (e.g. ExeCommand='x --flag') "
        "is allowed — only comment bodies are checked.",
    ]
    deny("\n".join(lines))


if __name__ == "__main__":
    main()
