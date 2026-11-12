#!/usr/bin/env python3
"""Distinguish full CI from documentation checks using exact GitHub run evidence."""

import argparse
import json
from pathlib import Path
import re
import subprocess
import sys
from urllib.parse import urlencode

REPOSITORY = "QubeTX/qube-network-diagnostics"
WORKFLOW_PATH = ".github/workflows/ci.yml"
CLASSIFIER = "Change scope / Classify changes"
REQUIRED = "CI required"
FULL = "Full CI"
DOCS = "Documentation checks"
MAX_ITEMS = 2000


def positive_integer(value):
    return type(value) is int and value > 0


def api(path):
    result = subprocess.run(
        ["gh", "api", "--method", "GET", f"repos/{REPOSITORY}/{path}"],
        check=True, capture_output=True, text=True, timeout=30,
    )
    return json.loads(result.stdout)


def paginated(path, key, request):
    """Refuse incomplete, inconsistent, duplicate, or unbounded evidence."""
    records = []
    expected_count = None
    seen_ids = set()
    for page in range(1, MAX_ITEMS // 100 + 1):
        separator = "&" if "?" in path else "?"
        data = request(f"{path}{separator}per_page=100&page={page}")
        if not isinstance(data, dict):
            raise RuntimeError("Malformed CI evidence response")
        count = data.get("total_count")
        batch = data.get(key)
        if (type(count) is not int or not 0 <= count <= MAX_ITEMS
                or not isinstance(batch, list) or len(batch) > 100
                or expected_count is not None and count != expected_count):
            raise RuntimeError("Invalid or changing CI evidence pagination")
        expected_count = count
        for record in batch:
            if not isinstance(record, dict):
                raise RuntimeError("Malformed CI evidence record")
            record_id = record.get("id")
            if not positive_integer(record_id) or record_id in seen_ids:
                raise RuntimeError("Duplicate or invalid CI evidence identity")
            seen_ids.add(record_id)
        records.extend(batch)
        if len(records) == expected_count:
            return records
        if not batch or len(records) > expected_count:
            raise RuntimeError("Incomplete CI evidence pagination")
    raise RuntimeError("CI evidence exceeds pagination bounds")


def is_expected_run(run, sha):
    if not isinstance(run, dict):
        return False
    repository = run.get("repository")
    head_repository = run.get("head_repository")
    if not isinstance(repository, dict) or not isinstance(head_repository, dict):
        return False
    return (
        positive_integer(run.get("id"))
        and run.get("head_sha") == sha
        and run.get("event") == "push"
        and run.get("head_branch") == "main"
        and run.get("path") == WORKFLOW_PATH
        and repository.get("full_name") == REPOSITORY
        and head_repository.get("full_name") == REPOSITORY
    )


def latest_run(sha, request):
    # Do not filter status=success: a newer failed, cancelled, or pending run
    # must never disappear behind an older successful run for the same SHA.
    query = urlencode({"event": "push", "branch": "main", "head_sha": sha})
    runs = paginated(f"actions/workflows/ci.yml/runs?{query}", "workflow_runs", request)
    eligible = [run for run in runs if is_expected_run(run, sha)]
    if not eligible:
        raise RuntimeError("No exact-source main-push CI run exists")
    return max(eligible, key=lambda run: run["id"])


def validate_run(run, sha, run_id, attempt):
    if (not is_expected_run(run, sha) or run["id"] != run_id
            or run.get("run_attempt") != attempt
            or not positive_integer(run.get("run_attempt"))):
        raise RuntimeError("CI run identity, source, repository, ref, workflow, or attempt mismatch")
    if run.get("status") != "completed" or run.get("conclusion") != "success":
        raise RuntimeError("Latest CI run attempt has not completed successfully")


def classify_jobs(jobs, sha, run_id, attempt):
    markers = {}
    for job in jobs:
        # GitHub's latest jobs view can retain a successful job from an earlier
        # attempt of this SAME immutable run. This permits failed-job retries,
        # while rejecting another run/SHA or a future/invalid attempt.
        job_attempt = job.get("run_attempt")
        if (not positive_integer(job.get("run_id")) or job.get("run_id") != run_id or job.get("head_sha") != sha
                or not positive_integer(job_attempt) or job_attempt > attempt):
            raise RuntimeError("CI job belongs to another run, source, or attempt")
        if job.get("status") != "completed" or job.get("conclusion") not in {"success", "skipped"}:
            raise RuntimeError("Latest CI job evidence includes an unfinished or failed job")
        name = job.get("name")
        if not isinstance(name, str) or not name:
            raise RuntimeError("CI job has no valid name")
        if name in {CLASSIFIER, REQUIRED, FULL, DOCS}:
            if name in markers:
                raise RuntimeError(f"Ambiguous duplicate CI marker: {name}")
            markers[name] = job["conclusion"]
    if set(markers) != {CLASSIFIER, REQUIRED, FULL, DOCS}:
        raise RuntimeError("CI evidence lacks the required classification/full/docs markers")
    if markers[CLASSIFIER] != "success" or markers[REQUIRED] != "success":
        raise RuntimeError("CI classification and required gate must both succeed")
    if markers[FULL] == "success" and markers[DOCS] == "skipped":
        return "full"
    if markers[FULL] == "skipped" and markers[DOCS] == "success":
        return "docs"
    raise RuntimeError("CI full/docs marker states are inconsistent")


def verify_ci(sha, run_id=None, attempt=None, allow_docs=False, request=api):
    if not re.fullmatch(r"[0-9a-f]{40}", sha):
        raise ValueError("source must be a complete lowercase commit SHA")
    if (run_id is None) != (attempt is None):
        raise ValueError("run ID and attempt must be supplied together")
    selected = latest_run(sha, request)
    if run_id is not None and (
        not positive_integer(run_id) or not positive_integer(attempt) or selected["id"] != run_id
    ):
        raise RuntimeError("Triggering CI run is not the latest exact-source CI run")
    run_id = selected["id"]
    attempt = selected.get("run_attempt") if attempt is None else attempt
    validate_run(selected, sha, run_id, attempt)
    path = f"actions/runs/{run_id}"
    validate_run(request(path), sha, run_id, attempt)
    validate_run(request(f"{path}/attempts/{attempt}"), sha, run_id, attempt)
    jobs = paginated(f"{path}/jobs?filter=latest", "jobs", request)
    mode = classify_jobs(jobs, sha, run_id, attempt)
    # A retry or new run appearing during pagination invalidates this receipt.
    validate_run(request(path), sha, run_id, attempt)
    latest = latest_run(sha, request)
    validate_run(latest, sha, run_id, attempt)
    if mode == "docs" and not allow_docs:
        raise RuntimeError("Documentation-only CI cannot qualify a release")
    return {"mode": mode, "full": mode == "full", "source_sha": sha,
            "run_id": run_id, "attempt": attempt}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("sha")
    parser.add_argument("--run-id", type=int)
    parser.add_argument("--attempt", type=int)
    parser.add_argument("--allow-docs", action="store_true")
    parser.add_argument("--github-output", type=Path)
    args = parser.parse_args(argv)
    try:
        evidence = verify_ci(args.sha, args.run_id, args.attempt, args.allow_docs)
        if args.github_output:
            with args.github_output.open("a", encoding="utf-8") as stream:
                stream.write(f"full={str(evidence['full']).lower()}\n")
                stream.write(f"source_sha={evidence['source_sha']}\n")
        print(json.dumps(evidence, sort_keys=True))
    except (OSError, KeyError, TypeError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"::error::CI evidence refused: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
