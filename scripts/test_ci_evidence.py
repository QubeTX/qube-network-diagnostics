"""Exact-run CI evidence fixtures; no network, Cargo, or publication."""

from contextlib import redirect_stderr, redirect_stdout
from copy import deepcopy
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from urllib.parse import parse_qs, urlsplit

import yaml

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("ci_evidence", ROOT / "scripts/verify-ci-evidence.py")
evidence = importlib.util.module_from_spec(spec)
spec.loader.exec_module(evidence)
SHA = "a" * 40
OTHER = "b" * 40
RUN_ID = 321
ATTEMPT = 2


def run(run_id=RUN_ID, **updates):
    return {
        "id": run_id, "run_attempt": ATTEMPT, "head_sha": SHA,
        "event": "push", "head_branch": "main", "path": evidence.WORKFLOW_PATH,
        "repository": {"full_name": evidence.REPOSITORY},
        "head_repository": {"full_name": evidence.REPOSITORY},
        "status": "completed", "conclusion": "success", **updates,
    }


def job(name, job_id, conclusion="success", **updates):
    return {"id": job_id, "run_id": RUN_ID, "run_attempt": ATTEMPT,
            "head_sha": SHA, "name": name, "status": "completed",
            "conclusion": conclusion, **updates}


def markers(mode="full"):
    return [job(evidence.CLASSIFIER, 1001), job(evidence.REQUIRED, 1002),
            job(evidence.FULL, 1003, "success" if mode == "full" else "skipped"),
            job(evidence.DOCS, 1004, "skipped" if mode == "full" else "success")]


class GitHub:
    def __init__(self, mode="full"):
        self.run = run()
        self.runs = [self.run]
        self.jobs = markers(mode)
        self.calls = []
        self.metadata_updates = {}
        self.attempt_updates = {}
        self.metadata_reads = 0
        self.list_reads = 0
        self.on_metadata = None
        self.on_list = None
        self.empty_jobs_page = None

    def __call__(self, path):
        self.calls.append(path)
        parsed = urlsplit(path)
        query = parse_qs(parsed.query)
        if parsed.path == "actions/workflows/ci.yml/runs":
            self.list_reads += 1
            if self.on_list:
                self.on_list(self)
            self.assert_query(query, {"head_sha": SHA, "branch": "main", "event": "push"})
            self.assert_query(query, {"per_page": "100"})
            page = int(query["page"][0])
            return deepcopy({"total_count": len(self.runs), "workflow_runs": self.runs[(page - 1) * 100:page * 100]})
        if parsed.path == f"actions/runs/{self.run['id']}":
            self.metadata_reads += 1
            if self.on_metadata:
                self.on_metadata(self)
            return deepcopy({**self.run, **self.metadata_updates})
        if parsed.path == f"actions/runs/{self.run['id']}/attempts/{ATTEMPT}":
            return deepcopy({**self.run, **self.attempt_updates})
        if parsed.path == f"actions/runs/{self.run['id']}/jobs":
            self.assert_query(query, {"filter": "latest", "per_page": "100"})
            page = int(query["page"][0])
            batch = [] if page == self.empty_jobs_page else self.jobs[(page - 1) * 100:page * 100]
            return deepcopy({"total_count": len(self.jobs), "jobs": batch})
        raise AssertionError(f"Unexpected API request: {path}")

    @staticmethod
    def assert_query(query, expected):
        for key, value in expected.items():
            if query.get(key) != [value]:
                raise AssertionError((key, query, value))
        if "status" in query:
            raise AssertionError("Filtering successful runs could hide a newer failure")


class EvidenceTests(unittest.TestCase):
    def verify(self, github=None, **options):
        return evidence.verify_ci(SHA, request=github or GitHub(), **options)

    def test_full_exact_trigger_and_release_lookup(self):
        for options in ({}, {"run_id": RUN_ID, "attempt": ATTEMPT}):
            self.assertEqual(self.verify(**options), {
                "mode": "full", "full": True, "source_sha": SHA,
                "run_id": RUN_ID, "attempt": ATTEMPT,
            })

    def test_docs_are_positive_publisher_noop_but_never_release_evidence(self):
        result = self.verify(GitHub("docs"), run_id=RUN_ID, attempt=ATTEMPT, allow_docs=True)
        self.assertEqual((result["mode"], result["full"]), ("docs", False))
        with self.assertRaisesRegex(RuntimeError, "cannot qualify"):
            self.verify(GitHub("docs"))

    def test_successful_same_run_jobs_can_carry_from_prior_attempt(self):
        github = GitHub()
        for item in github.jobs:
            item["run_attempt"] = 1
        self.assertTrue(self.verify(github, run_id=RUN_ID, attempt=ATTEMPT)["full"])

    def test_latest_job_failure_or_skip_never_borrows_prior_success(self):
        for conclusion in ("failure", "cancelled", "timed_out", "neutral", "skipped"):
            github = GitHub()
            github.jobs[2]["conclusion"] = conclusion
            with self.subTest(conclusion=conclusion), self.assertRaises(RuntimeError):
                self.verify(github)

    def test_job_markers_missing_ambiguous_or_inconsistent(self):
        cases = []
        for index in range(4):
            values = markers()
            del values[index]
            cases.append(values)
        cases.extend([
            markers() + [job(evidence.FULL, 2000)],
            markers()[:2] + [job(evidence.FULL, 1003), job(evidence.DOCS, 1004)],
            markers()[:2] + [job(evidence.FULL, 1003, "skipped"), job(evidence.DOCS, 1004, "skipped")],
        ])
        for values in cases:
            github = GitHub()
            github.jobs = values
            with self.subTest(values=values), self.assertRaises(RuntimeError):
                self.verify(github, allow_docs=True)

    def test_failed_classifier_required_or_extra_job_refuses(self):
        for name in (evidence.CLASSIFIER, evidence.REQUIRED, "New optional job"):
            github = GitHub()
            if name == "New optional job":
                github.jobs.append(job(name, 2000, "failure"))
            else:
                next(item for item in github.jobs if item["name"] == name)["conclusion"] = "failure"
            with self.subTest(name=name), self.assertRaises(RuntimeError):
                self.verify(github)

    def test_job_identity_and_attempt_boundaries(self):
        for updates in ({"head_sha": OTHER}, {"run_id": 999}, {"run_attempt": 3},
                        {"run_attempt": 0}, {"run_attempt": None}, {"run_attempt": True},
                        {"status": "in_progress"}):
            github = GitHub()
            github.jobs[0].update(updates)
            with self.subTest(updates=updates), self.assertRaises(RuntimeError):
                self.verify(github)

    def test_run_identity_boundaries(self):
        cases = [
            {"head_sha": OTHER}, {"head_branch": "feature"}, {"event": "pull_request"},
            {"path": ".github/workflows/fake.yml"}, {"repository": {"full_name": "attacker/repo"}},
            {"head_repository": {"full_name": "attacker/repo"}}, {"id": 999},
            {"run_attempt": 1}, {"run_attempt": True},
            {"status": "in_progress"}, {"conclusion": "failure"}, {"conclusion": "cancelled"},
        ]
        for updates in cases:
            github = GitHub()
            github.metadata_updates = updates
            with self.subTest(updates=updates), self.assertRaises(RuntimeError):
                self.verify(github, run_id=RUN_ID, attempt=ATTEMPT)

    def test_attempt_endpoint_is_bound_too(self):
        for updates in ({"run_attempt": 1}, {"head_sha": OTHER}, {"conclusion": "cancelled"}):
            github = GitHub()
            github.attempt_updates = updates
            with self.subTest(updates=updates), self.assertRaises(RuntimeError):
                self.verify(github)

    def test_stale_triggering_attempt_or_run_refuses(self):
        for options in ({"run_id": RUN_ID, "attempt": 1}, {"run_id": RUN_ID - 1, "attempt": ATTEMPT}):
            with self.subTest(options=options), self.assertRaises(RuntimeError):
                self.verify(**options)

    def test_newer_unsuccessful_run_prevents_old_success_fallback(self):
        for conclusion, status in (("failure", "completed"), ("cancelled", "completed"), (None, "in_progress")):
            github = GitHub()
            github.runs.append(run(RUN_ID + 1, conclusion=conclusion, status=status))
            with self.subTest(conclusion=conclusion), self.assertRaises(RuntimeError):
                self.verify(github)

    def test_more_than_100_jobs_and_runs_are_paginated(self):
        github = GitHub()
        github.jobs = [job(f"Other job {n}", 2000 + n) for n in range(101)] + markers()
        self.assertTrue(self.verify(github)["full"])
        self.assertTrue(any("filter=latest&per_page=100&page=2" in path for path in github.calls))
        github = GitHub()
        github.runs = [run(n) for n in range(1, RUN_ID + 1)]
        self.assertTrue(self.verify(github)["full"])
        self.assertTrue(any("runs?" in path and "page=4" in path for path in github.calls))

    def test_incomplete_duplicate_and_invalid_pagination_refuses(self):
        github = GitHub()
        github.jobs = [job(f"Other job {n}", 2000 + n) for n in range(101)] + markers()
        github.empty_jobs_page = 2
        with self.assertRaisesRegex(RuntimeError, "Incomplete"):
            self.verify(github)
        github = GitHub()
        github.jobs.append(deepcopy(github.jobs[0]))
        with self.assertRaisesRegex(RuntimeError, "Duplicate"):
            self.verify(github)
        for count in (-1, None, evidence.MAX_ITEMS + 1):
            with self.subTest(count=count), self.assertRaises(RuntimeError):
                evidence.paginated("path", "jobs", lambda _: {"total_count": count, "jobs": []})
        pages = iter([
            {"total_count": 2, "jobs": [job("one", 1)]},
            {"total_count": 3, "jobs": [job("two", 2)]},
        ])
        with self.assertRaisesRegex(RuntimeError, "changing"):
            evidence.paginated("path", "jobs", lambda _: next(pages))
        for response in ([], {"total_count": 1, "jobs": [None]}, {"total_count": 1, "jobs": None}):
            with self.subTest(response=response), self.assertRaises(RuntimeError):
                evidence.paginated("path", "jobs", lambda _: response)

    def test_invalid_arguments_fail_before_requests(self):
        for sha, run_id, attempt in (("main", None, None), (SHA, RUN_ID, None), (SHA, None, ATTEMPT)):
            with self.subTest(sha=sha, run_id=run_id, attempt=attempt), self.assertRaises(ValueError):
                evidence.verify_ci(sha, run_id, attempt, request=lambda _: self.fail("Unexpected API request"))

    def test_retry_or_new_run_during_lookup_refuses(self):
        github = GitHub()
        def retry(client):
            if client.metadata_reads == 2:
                client.metadata_updates = {"run_attempt": 3, "status": "in_progress"}
        github.on_metadata = retry
        with self.assertRaises(RuntimeError):
            self.verify(github)
        github = GitHub()
        def new_run(client):
            if client.list_reads == 2:
                client.runs.append(run(RUN_ID + 1))
        github.on_list = new_run
        with self.assertRaises(RuntimeError):
            self.verify(github)

    def test_old_run_without_new_markers_cannot_qualify(self):
        github = GitHub()
        github.jobs = [job(evidence.REQUIRED, 1001)]
        with self.assertRaises(RuntimeError):
            self.verify(github, allow_docs=True)

    def test_cli_writes_outputs_only_after_verified_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "outputs"
            args = [SHA, "--run-id", str(RUN_ID), "--attempt", str(ATTEMPT), "--allow-docs", "--github-output", str(output)]
            with patch.object(evidence, "verify_ci", return_value={"full": False, "mode": "docs", "source_sha": SHA}), redirect_stdout(io.StringIO()):
                self.assertEqual(evidence.main(args), 0)
            self.assertEqual(output.read_text(), f"full=false\nsource_sha={SHA}\n")
            output.unlink()
            with patch.object(evidence, "verify_ci", side_effect=RuntimeError("bad evidence")), redirect_stderr(io.StringIO()):
                self.assertEqual(evidence.main(args), 1)
            self.assertFalse(output.exists())


class WorkflowBoundaryTests(unittest.TestCase):
    def workflow(self, name):
        return yaml.load((ROOT / ".github/workflows" / name).read_text(), Loader=yaml.BaseLoader)

    def test_publisher_has_no_rust_or_secret_before_exact_run_gate(self):
        jobs = self.workflow("crates-publish.yml")["jobs"]
        context = jobs["context"]
        content = json.dumps(context)
        self.assertEqual(context["permissions"], {"contents": "read", "actions": "read"})
        self.assertNotIn("secrets.", content)
        self.assertNotIn("rust-toolchain", content)
        self.assertNotIn("cargo ", content)
        for field in ("head_sha", "id", "run_attempt"):
            self.assertIn(f"github.event.workflow_run.{field}", content)
        self.assertIn("--allow-docs", content)
        publish = jobs["publish"]
        self.assertEqual(publish["needs"], "context")
        self.assertEqual(publish["if"], "needs.context.outputs.full == 'true'")

    def test_release_requires_classifier_full_proof_and_original_apple_gate(self):
        release = self.workflow("release.yml")
        self.assertEqual(release["jobs"]["changes"]["uses"], "./.github/workflows/change-scope.yml")
        plan = release["jobs"]["plan"]
        self.assertEqual(plan["needs"], "changes")
        self.assertIn("needs.changes.result == 'success'", plan["if"])
        self.assertIn("needs.changes.outputs.docs_only == 'false'", plan["if"])
        content = json.dumps(plan)
        self.assertIn('verify-ci-evidence.py', content)
        self.assertNotIn('--allow-docs', content)
        self.assertIn('wait-apple-qualification.py', content)


if __name__ == "__main__":
    unittest.main()
