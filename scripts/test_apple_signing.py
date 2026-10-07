"""Regression tests for the Apple credential boundary and release contract."""

import json
import os
from pathlib import Path
import runpy
import subprocess
import tempfile
import unittest

import yaml

ROOT = Path(__file__).resolve().parent.parent
policy = runpy.run_path(str(ROOT / "scripts/apple-signing-policy.py"))
qualification = runpy.run_path(str(ROOT / "scripts/wait-apple-qualification.py"))
SHA = "a" * 40
OTHER_SHA = "b" * 40
REPO = "QubeTX/qube-network-diagnostics"


def context(**overrides):
    return {
        "GITHUB_EVENT_NAME": "push", "GITHUB_REF": "refs/heads/main",
        "GITHUB_SHA": SHA, "GITHUB_REPOSITORY": REPO, **overrides,
    }


def release_context(**overrides):
    return context(GITHUB_REF="refs/tags/v4.0.1", CALLER_TAG="v4.0.1",
                   CALLER_SHA=SHA, CALLER_VERSION="4.0.1", **overrides)


def workflow(name):
    # BaseLoader keeps GitHub's `on` key a string instead of YAML 1.1 boolean True.
    return yaml.load((ROOT / ".github/workflows" / name).read_text(encoding="utf-8"),
                     Loader=yaml.BaseLoader)


class ContextTests(unittest.TestCase):
    def test_permitted_contexts(self):
        cases = [
            (context(), "candidate"),
            (context(GITHUB_EVENT_NAME="pull_request", GITHUB_REF="refs/pull/40/merge"), "pr"),
            (context(GITHUB_EVENT_NAME="pull_request", GITHUB_REF="refs/pull/40/merge",
                     GITHUB_REPOSITORY="fork/nd300"), "pr"),
            (context(GITHUB_EVENT_NAME="workflow_dispatch", PREFLIGHT_ONLY="true"), "preflight"),
            (release_context(), "release"),
            (context(GITHUB_EVENT_NAME="workflow_dispatch", GITHUB_REF="refs/tags/v4.0.1",
                     PREFLIGHT_ONLY="false", CALLER_TAG="v4.0.1"), "release"),
        ]
        for env, mode in cases:
            with self.subTest(env=env):
                self.assertEqual(policy["classify_context"](env)["mode"], mode)

    def test_unauthorized_events_and_refs(self):
        cases = [
            context(GITHUB_REF="refs/heads/feature"),
            context(GITHUB_REPOSITORY="fork/nd300"),
            context(GITHUB_EVENT_NAME="workflow_dispatch", GITHUB_REF="refs/heads/feature",
                    PREFLIGHT_ONLY="true"),
            context(GITHUB_EVENT_NAME="pull_request_target"),
            context(GITHUB_EVENT_NAME="workflow_run"),
            context(GITHUB_EVENT_NAME="workflow_dispatch", PREFLIGHT_ONLY="false"),
            context(GITHUB_EVENT_NAME="workflow_dispatch", PREFLIGHT_ONLY="true", CALLER_TAG="v4.0.1"),
            context(CALLER_SHA=SHA),
            context(GITHUB_SHA=SHA + "\nmode=release"),
            context(GITHUB_EVENT_NAME="pull_request", GITHUB_REF="refs/pull/40/merge", CALLER_SHA=SHA),
            context(GITHUB_EVENT_NAME="pull_request", GITHUB_REF="refs/heads/main"),
        ]
        for env in cases:
            with self.subTest(env=env), self.assertRaises(policy["PolicyError"]):
                policy["classify_context"](env)

    def test_tag_aliases_input_injection_and_identity_mismatch(self):
        for tag in ["v4.0.1-rc1", "v04.0.1", "v4.0.1\n", "v4.0.1';echo sentinel;'", "main", "v4/0/1"]:
            env = release_context()
            env.update(CALLER_TAG=tag, GITHUB_REF=f"refs/tags/{tag}", CALLER_VERSION=tag[1:])
            with self.subTest(tag=tag), self.assertRaises(policy["PolicyError"]):
                policy["classify_context"](env)
        for key, value in [("CALLER_SHA", OTHER_SHA), ("CALLER_VERSION", "4.0.2"),
                           ("GITHUB_REF", "refs/tags/v4.0.2"), ("PREFLIGHT_ONLY", "true")]:
            env = release_context()
            env[key] = value
            with self.subTest(key=key), self.assertRaises(policy["PolicyError"]):
                policy["classify_context"](env)

    def test_off_main_and_mismatched_source_fail_before_secret_job(self):
        with tempfile.TemporaryDirectory() as temp:
            cargo = Path(temp) / "Cargo.toml"
            cargo.write_text('[package]\nname="nd300"\nversion="4.0.1"\n', encoding="utf-8")

            def run(args):
                if args[1:3] == ["rev-parse", "HEAD"]:
                    return SHA
                if args[1] == "merge-base":
                    raise subprocess.CalledProcessError(1, args)
                return ""

            with self.assertRaises(subprocess.CalledProcessError):
                policy["verify_source"](policy["classify_context"](context()), run, cargo)
            with self.assertRaises(policy["PolicyError"]):
                policy["verify_source"](policy["classify_context"](context()), lambda _: OTHER_SHA, cargo)

    def test_release_evidence_binds_one_commit(self):
        with tempfile.TemporaryDirectory() as temp:
            cargo = Path(temp) / "Cargo.toml"
            cargo.write_text('[package]\nname="nd300"\nversion="4.0.1"\n', encoding="utf-8")
            for tag_sha, target_sha, passed in [(SHA, SHA, True), (OTHER_SHA, SHA, False), (SHA, OTHER_SHA, False)]:
                def run(args):
                    if args[:2] == ["git", "rev-parse"]:
                        return SHA if args[2] == "HEAD" else tag_sha
                    if args[0] == "gh":
                        return json.dumps({"tagName": "v4.0.1", "targetCommitish": target_sha})
                    return ""
                with self.subTest(tag_sha=tag_sha, target_sha=target_sha):
                    if passed:
                        result = policy["verify_source"](policy["classify_context"](release_context()), run, cargo)
                        self.assertEqual(result["version"], "4.0.1")
                    else:
                        with self.assertRaises(policy["PolicyError"]):
                            policy["verify_source"](policy["classify_context"](release_context()), run, cargo)

    def test_rejected_dispatch_does_not_write_github_outputs(self):
        with tempfile.TemporaryDirectory() as temp:
            output = Path(temp) / "outputs"
            env = {**os.environ, **context(GITHUB_EVENT_NAME="workflow_dispatch",
                   GITHUB_REF="refs/heads/unreviewed", PREFLIGHT_ONLY="true"),
                   "GITHUB_OUTPUT": str(output)}
            result = subprocess.run([os.sys.executable, str(ROOT / "scripts/apple-signing-policy.py")],
                                    env=env, capture_output=True, text=True, check=False)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(output.exists())


class QualificationTests(unittest.TestCase):
    def run_data(self, **overrides):
        return {"id": 123, "head_sha": SHA, "head_branch": "main", "event": "push",
                "path": ".github/workflows/macos-installer.yml", "head_repository": {"full_name": REPO},
                "status": "completed", "conclusion": "success", "run_attempt": 2,
                "html_url": "https://github.com/QubeTX/qube-network-diagnostics/actions/runs/123", **overrides}

    def test_other_sources_cannot_supply_release_qualification(self):
        for changes in [{"head_sha": OTHER_SHA}, {"event": "pull_request"}, {"head_branch": "feature"},
                        {"event": "workflow_dispatch"}, {"head_repository": {"full_name": "fork/repo"}},
                        {"path": ".github/workflows/unrelated.yml"}]:
            self.assertIsNone(qualification["select_run"]([self.run_data(**changes)], SHA))

    def test_latest_run_and_positive_job_evidence_are_required(self):
        success = self.run_data()
        failure = self.run_data(id=124, conclusion="failure")
        self.assertEqual(qualification["select_run"]([success, failure], SHA)["id"], 124)
        jobs = [{"name": name, "conclusion": "success"} for name in qualification["REQUIRED_JOBS"]]
        self.assertTrue(qualification["jobs_passed"](jobs))
        for conclusion in ["skipped", "cancelled", "failure", None]:
            self.assertFalse(qualification["jobs_passed"]([{**jobs[0], "conclusion": conclusion}, *jobs[1:]]))
        self.assertFalse(qualification["jobs_passed"](jobs[1:]))

    def test_wait_is_bounded_and_terminal_failure_is_immediate(self):
        ticks = [0]
        calls = []
        def sleep(seconds):
            ticks[0] += seconds
        def pending(path, *args):
            calls.append(path)
            return {"workflow_runs": []}
        with self.assertRaisesRegex(RuntimeError, "Timed out"):
            qualification["wait_for_qualification"](SHA, timeout=60, interval=30,
                                                     request=pending, clock=lambda: ticks[0], sleep=sleep)
        self.assertEqual(ticks[0], 60)
        self.assertEqual(len(calls), 3)
        for conclusion in ["failure", "cancelled", "timed_out", "skipped", "action_required"]:
            with self.subTest(conclusion=conclusion), self.assertRaisesRegex(RuntimeError, "ended"):
                qualification["wait_for_qualification"](
                    SHA, request=lambda *args: {"workflow_runs": [self.run_data(conclusion=conclusion)]},
                    sleep=lambda _: self.fail("terminal failure must not sleep"))

    def test_success_checks_jobs_from_the_same_attempt(self):
        calls = []
        def request(path, *args):
            calls.append(path)
            if "/attempts/2/jobs" in path:
                return {"jobs": [{"name": name, "conclusion": "success"}
                                 for name in qualification["REQUIRED_JOBS"]]}
            return {"workflow_runs": [self.run_data()]}
        self.assertEqual(qualification["wait_for_qualification"](SHA, request=request), 123)
        self.assertIn("actions/runs/123/attempts/2/jobs?per_page=100", calls)


class WorkflowContractTests(unittest.TestCase):
    def test_all_apple_secret_consumers_require_environment(self):
        consumers = set()
        for path in (ROOT / ".github/workflows").glob("*.yml"):
            # The one-time migration handles repository credentials only on
            # protected main. It is deleted after environment cutover.
            if path.name == "migrate-apple-secrets.yml":
                continue
            for name, job in workflow(path.name)["jobs"].items():
                if "secrets.APPLE_" in json.dumps(job):
                    consumers.add((path.name, name))
                    self.assertEqual(job.get("environment"), "apple-signing", (path.name, name))
        self.assertEqual(consumers, {
            ("release.yml", "build-local-artifacts"),
            ("macos-installer.yml", "credential-preflight"),
            ("macos-installer.yml", "build"),
            ("macos-installer.yml", "candidate-archive"),
            ("macos-installer.yml", "candidate-package"),
        })

    def test_pr_native_builds_never_enter_signing_environment(self):
        mac = workflow("macos-installer.yml")
        self.assertEqual(mac["on"]["push"]["branches"], ["main"])
        self.assertNotIn("paths", mac["on"]["push"])
        self.assertNotIn("workflow_run", mac["on"])
        self.assertNotIn("pull_request_target", mac["on"])
        jobs = mac["jobs"]
        for name in ["signing-context", "candidate-thin", "candidate-validate", "validate"]:
            self.assertNotIn("environment", jobs[name])
            self.assertNotIn("secrets.APPLE_", json.dumps(jobs[name]))
        for name in ["credential-preflight", "build", "candidate-archive", "candidate-package"]:
            self.assertIn("signing-context", jobs[name]["needs"])
            self.assertIn("needs.signing-context.outputs.mode", jobs[name]["if"])
            self.assertNotIn('"pr"', jobs[name]["if"])
        for name in ["candidate-archive", "candidate-package", "candidate-validate"]:
            self.assertIn("== 'candidate'", jobs[name]["if"])
        self.assertIn('"pr"', jobs["candidate-thin"]["if"])

    def test_same_run_artifacts_and_exact_checkouts(self):
        jobs = workflow("macos-installer.yml")["jobs"]
        for name in ["credential-preflight", "build", "candidate-thin", "candidate-archive",
                     "candidate-package", "candidate-validate"]:
            for step in jobs[name]["steps"]:
                if step.get("uses", "").startswith("actions/checkout@"):
                    self.assertEqual(step["with"]["ref"], "${{ needs.signing-context.outputs.source_sha }}")
                if step.get("uses", "").startswith("actions/download-artifact@"):
                    self.assertNotIn("run-id", step["with"])
                    self.assertNotIn("repository", step["with"])
        self.assertNotIn("${{ inputs.", "\n".join(
            step.get("run", "") for job in jobs.values() for step in job.get("steps", [])))

    def test_release_interface_inventory_and_legacy_bridge_are_preserved(self):
        mac = workflow("macos-installer.yml")
        self.assertEqual(set(mac["on"]["workflow_call"]["inputs"]), {"tag", "source_sha", "version"})
        release = workflow("release.yml")
        caller = release["jobs"]["macos-installer"]
        self.assertNotIn("secrets", caller)
        self.assertEqual(caller["uses"], "./.github/workflows/macos-installer.yml")
        self.assertIn("wait-apple-qualification.py", json.dumps(release["jobs"]["plan"]))
        self.assertIn("wait-apple-qualification.py", json.dumps(mac["jobs"]["signing-context"]))
        self.assertEqual(caller["permissions"]["actions"], "read")
        publish = mac["jobs"]["publish"]
        content = json.dumps(publish)
        for asset in ["nd300-universal-apple-darwin.pkg", "nd300-universal-apple-darwin.pkg.sha256",
                      "nd300-universal-apple-darwin.dmg", "nd300-universal-apple-darwin.dmg.sha256"]:
            self.assertIn(asset, content)
        self.assertIn("= 32", content)
        self.assertNotIn("--clobber", content)
        self.assertEqual(set(publish["needs"]), {"build", "validate"})
        self.assertEqual(set(mac["jobs"]["legacy-bridge"]["needs"]), {"build", "publish"})
        self.assertIn("macos-installer", release["jobs"]["announce"]["needs"])


if __name__ == "__main__":
    unittest.main()
