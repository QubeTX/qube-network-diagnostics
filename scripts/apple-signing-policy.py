#!/usr/bin/env python3
"""Resolve an exact Apple signing source before a job enters apple-signing.

GitHub's environment ref policy and repository rulesets are the authorization
boundary. This additional fail-closed gate binds permitted runs to their source;
it must never be used as a substitute for moving secrets out of repository scope.
"""

import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tomllib

REPOSITORY = "QubeTX/qube-network-diagnostics"
STABLE_TAG = re.compile(r"v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)")
SHA = re.compile(r"[0-9a-f]{40}")


class PolicyError(ValueError):
    """A run is not an authorized signing source."""


def classify_context(env):
    event = env.get("GITHUB_EVENT_NAME", "")
    ref = env.get("GITHUB_REF", "")
    sha = env.get("GITHUB_SHA", "")
    tag = env.get("CALLER_TAG", "")
    caller_sha = env.get("CALLER_SHA", "")
    version = env.get("CALLER_VERSION", "")
    preflight = env.get("PREFLIGHT_ONLY", "")
    if not SHA.fullmatch(sha):
        raise PolicyError("workflow SHA must be an exact commit")

    # Ordinary PR builds have no signing authority, even within this repository.
    # A reusable call carrying release inputs cannot masquerade as that path.
    if event == "pull_request" and re.fullmatch(r"refs/pull/[1-9][0-9]*/merge", ref):
        if tag or caller_sha or version or preflight not in ("", "false"):
            raise PolicyError("pull requests cannot call the signing workflow")
        return {"mode": "pr", "source_sha": sha, "tag": "", "version": ""}

    if env.get("GITHUB_REPOSITORY") != REPOSITORY:
        raise PolicyError("signing is restricted to the owning repository")
    if event == "push" and ref == "refs/heads/main":
        if tag or caller_sha or version or preflight not in ("", "false"):
            raise PolicyError("main qualification cannot carry release overrides")
        return {"mode": "candidate", "source_sha": sha, "tag": "", "version": ""}
    if event == "workflow_dispatch" and ref == "refs/heads/main" and preflight == "true":
        if tag or caller_sha or version:
            raise PolicyError("main preflight cannot select a different source")
        return {"mode": "preflight", "source_sha": sha, "tag": "", "version": ""}

    if not STABLE_TAG.fullmatch(tag) or ref != f"refs/tags/{tag}":
        raise PolicyError("release signing requires the exact stable release tag")
    if event == "push":
        if caller_sha != sha or version != tag[1:] or preflight not in ("", "false"):
            raise PolicyError("reusable release inputs must match the tag-push context")
    elif event == "workflow_dispatch":
        if preflight != "false" or caller_sha or version:
            raise PolicyError("repair dispatch must run on its tag with preflight_only=false")
    else:
        raise PolicyError("this event cannot authorize Apple signing")
    return {"mode": "release", "source_sha": sha, "tag": tag, "version": tag[1:]}


def command(args):
    return subprocess.run(args, check=True, text=True, capture_output=True, timeout=90).stdout.strip()


def verify_source(context, run=command, cargo_path=Path("Cargo.toml")):
    """Verify the checked-out source, main ancestry and immutable release identity."""
    if context["mode"] == "pr":
        return context
    sha = context["source_sha"]
    if run(["git", "rev-parse", "HEAD"]) != sha:
        raise PolicyError("checkout does not match the workflow commit")
    run(["git", "fetch", "--no-tags", "origin", "+refs/heads/main:refs/remotes/origin/main"])
    run(["git", "merge-base", "--is-ancestor", sha, "refs/remotes/origin/main"])
    with cargo_path.open("rb") as handle:
        package = tomllib.load(handle)["package"]
    version = package["version"]
    if package["name"] != "nd300" or not STABLE_TAG.fullmatch(f"v{version}"):
        raise PolicyError("source must contain the stable nd300 package version")
    if context["mode"] == "release":
        tag = context["tag"]
        if context["version"] != version:
            raise PolicyError("release tag and Cargo version disagree")
        resolved_tag = run(["git", "rev-parse", f"refs/tags/{tag}^{{commit}}"])
        if resolved_tag != sha:
            raise PolicyError("release tag does not resolve to the workflow commit")
        release = json.loads(run([
            "gh", "release", "view", tag, "--repo", REPOSITORY,
            "--json", "tagName,targetCommitish",
        ]))
        if release.get("tagName") != tag or release.get("targetCommitish") != sha:
            raise PolicyError("GitHub Release does not bind the exact tag and source")
    return {**context, "version": version}


def main():
    try:
        context = verify_source(classify_context(os.environ))
        with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as handle:
            for key, value in context.items():
                handle.write(f"{key}={value}\n")
        print(f"Apple signing context: {context['mode']} at {context['source_sha']}")
    except (PolicyError, OSError, KeyError, ValueError, subprocess.SubprocessError) as error:
        print(f"::error::Apple signing context refused: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
