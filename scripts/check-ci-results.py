#!/usr/bin/env python3
"""Require an explicit valid full or documentation-only CI result set."""

import json
import os
import sys


ALWAYS = {"changes", "apple-policy", "shellcheck"}
EXPENSIVE = {"fmt", "clippy", "test", "build", "linux-targets", "audit", "dist-plan"}


def verify(results):
    expected = ALWAYS | EXPENSIVE | {"documentation"}
    if set(results) != expected:
        raise ValueError("Missing or unexpected required CI dependencies")
    docs = results["changes"].get("outputs", {}).get("docs_only")
    if docs not in ("true", "false"):
        raise ValueError("Change classification did not return an explicit boolean")
    expected_results = {name: "success" for name in ALWAYS}
    expected_results.update({name: "skipped" if docs == "true" else "success" for name in EXPENSIVE})
    expected_results["documentation"] = "success" if docs == "true" else "skipped"
    if any(results[name].get("result") != value for name, value in expected_results.items()):
        raise ValueError("A required CI check failed, was cancelled, or ran/skipped unexpectedly")
    return "documentation" if docs == "true" else "full"


def main():
    try:
        mode = verify(json.loads(os.environ["RESULTS"]))
    except (KeyError, ValueError, TypeError, AttributeError):
        print("Required CI does not have a complete successful result set.", file=sys.stderr)
        return 1
    print(f"Every required check passed for the {mode} validation path.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
