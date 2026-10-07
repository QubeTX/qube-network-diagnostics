#!/usr/bin/env python3
"""Require successful native Apple qualification for the exact release commit."""

import argparse
import json
import re
import subprocess
import sys
import time

REPOSITORY = "QubeTX/qube-network-diagnostics"
WORKFLOW = "macos-installer.yml"
REQUIRED_JOBS = {
    "Installer identity preflight (Apple Silicon)",
    "Installer identity preflight (Intel)",
    "Candidate native binaries (Apple Silicon)",
    "Candidate native binaries (Intel)",
    "Candidate signed archive (Apple Silicon)",
    "Candidate signed archive (Intel)",
    "Candidate signed and notarized PKG and compatibility DMG",
    "Candidate lifecycle (Apple Silicon)",
    "Candidate lifecycle (Intel)",
}


def select_run(runs, sha):
    eligible = [run for run in runs if (
        run.get("head_sha") == sha
        and run.get("head_branch") == "main"
        and run.get("event") == "push"
        and run.get("path") == f".github/workflows/{WORKFLOW}"
        and (run.get("head_repository") or {}).get("full_name") == REPOSITORY
    )]
    return max(eligible, key=lambda run: run["id"], default=None)


def jobs_passed(jobs):
    # A workflow can succeed when critical jobs were skipped. Require positive
    # evidence from each native signer and lifecycle test, not the aggregate alone.
    return all(any(job.get("name") == name and job.get("conclusion") == "success"
                   for job in jobs) for name in REQUIRED_JOBS)


def api(path, *args):
    result = subprocess.run(
        ["gh", "api", f"repos/{REPOSITORY}/{path}", *args],
        capture_output=True, text=True, check=True, timeout=90,
    )
    return json.loads(result.stdout)


def wait_for_qualification(sha, timeout=3600, interval=30, request=api,
                           clock=time.monotonic, sleep=time.sleep):
    deadline = clock() + timeout
    previous = None
    while True:
        data = request(f"actions/workflows/{WORKFLOW}/runs", "--method", "GET",
                       "-f", "event=push", "-f", "branch=main", "-f", f"head_sha={sha}",
                       "-f", "per_page=100")
        run = select_run(data["workflow_runs"], sha)
        if run and run["status"] == "completed":
            if run.get("conclusion") != "success":
                raise RuntimeError(f"Apple qualification run {run['id']} ended: {run.get('conclusion')}")
            jobs = request(f"actions/runs/{run['id']}/attempts/{run['run_attempt']}/jobs?per_page=100")
            if not jobs_passed(jobs["jobs"]):
                raise RuntimeError("Apple qualification lacks successful signing/lifecycle jobs")
            print(f"Apple qualification passed for {sha}: {run['html_url']}", flush=True)
            return run["id"]
        state = (run["id"], run["status"]) if run else (None, "not yet scheduled")
        if state != previous:
            print(f"Waiting for exact-source Apple qualification: {state}", flush=True)
            previous = state
        remaining = deadline - clock()
        if remaining <= 0:
            raise RuntimeError("Timed out waiting for exact-source Apple qualification")
        sleep(min(interval, remaining))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("sha")
    args = parser.parse_args()
    if not re.fullmatch(r"[0-9a-f]{40}", args.sha):
        parser.error("sha must be a complete lowercase commit SHA")
    try:
        wait_for_qualification(args.sha)
    except (OSError, KeyError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"::error::Release held: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
