"""Keep documentation-only skips behind the shared, successful classifier."""

from pathlib import Path
import unittest

import yaml

ROOT = Path(__file__).resolve().parent.parent
FULL_ONLY = "${{ needs.changes.result == 'success' && needs.changes.outputs.docs_only != 'true' }}"
WINDOWS_EXISTING_PATHS = {
    ".github/workflows/windows-origin-matrix.yml",
    "scripts/windows-origin-matrix.ps1",
    "scripts/harden-cargo-dist-windows-installer.ps1",
    "scripts/test-harden-cargo-dist-windows-installer.ps1",
    "src/actions/migrate.rs", "src/actions/maintenance.rs", "src/actions/takeover.rs",
    "src/actions/uninstall.rs", "src/actions/update.rs", "src/cli.rs",
    "Cargo.toml", "Cargo.lock", "wix/**", "wix-corporate/**", "inno/**",
}
SCOPE_LOGIC_PATHS = {
    ".github/workflows/change-scope.yml", "scripts/classify-ci-changes.py",
    "scripts/check-ci-results.py",
    "scripts/test_ci_changes.py", "scripts/test_workflow_scope.py",
    "scripts/verify-ci-evidence.py", "scripts/test_ci_evidence.py",
}


def workflow(name):
    return yaml.load((ROOT / ".github/workflows" / name).read_text(encoding="utf-8"),
                     Loader=yaml.BaseLoader)


def dependencies(jobs, name):
    needs = jobs[name].get("needs", [])
    direct = {needs} if isinstance(needs, str) else set(needs)
    return direct | set().union(*(dependencies(jobs, parent) for parent in direct))


class WorkflowScopeTests(unittest.TestCase):
    def test_mac_and_parity_share_a_secret_free_classifier(self):
        for name in ["macos-installer.yml", "measurement-parity.yml"]:
            data = workflow(name)
            with self.subTest(workflow=name):
                self.assertEqual(data["permissions"], {"contents": "read"})
                self.assertEqual(data["jobs"].get("changes"), {
                    "name": "Change scope", "uses": "./.github/workflows/change-scope.yml",
                })

    def test_every_mac_job_follows_successful_classification_and_source_validation(self):
        data = workflow("macos-installer.yml")
        jobs = data["jobs"]
        self.assertEqual(jobs["signing-context"].get("needs"), "changes")
        self.assertEqual(jobs["signing-context"].get("if"), FULL_ONLY)
        for name in set(jobs) - {"changes", "signing-context"}:
            with self.subTest(job=name):
                self.assertIn("signing-context", dependencies(jobs, name))
                self.assertIn("changes", dependencies(jobs, name))
        # Keep ordinary runs observable, and retain the same explicit release,
        # repair and preflight entrypoints for fail-closed source validation.
        self.assertEqual(set(data["on"]), {"pull_request", "push", "workflow_call", "workflow_dispatch"})
        self.assertEqual(data["on"]["push"], {"branches": ["main"]})
        self.assertFalse(data["on"]["pull_request"])
        self.assertEqual(set(data["on"]["workflow_call"]["inputs"]),
                         {"tag", "source_sha", "version", "preflight_only"})

    def test_recorded_parity_only_runs_after_successful_full_classification(self):
        data = workflow("measurement-parity.yml")
        self.assertEqual(set(data["jobs"]), {"changes", "recorded-traces"})
        job = data["jobs"]["recorded-traces"]
        self.assertEqual(job.get("needs"), "changes")
        self.assertEqual(job.get("if"), FULL_ONLY)
        self.assertEqual(set(data["on"]), {"pull_request", "push"})
        self.assertEqual(data["on"]["push"], {"branches": ["main"]})

    def test_windows_keeps_existing_coverage_and_tests_scope_logic_changes(self):
        data = workflow("windows-origin-matrix.yml")
        self.assertEqual(set(data["on"]), {"pull_request", "workflow_dispatch"})
        self.assertEqual(set(data["on"]["pull_request"]["paths"]),
                         WINDOWS_EXISTING_PATHS | SCOPE_LOGIC_PATHS)
        dispatch = data["on"]["workflow_dispatch"]["inputs"]
        self.assertEqual(dispatch["mode"]["options"], ["public", "candidate"])
        self.assertEqual(data["jobs"]["build-candidate"]["if"],
                         "github.event_name == 'pull_request' || inputs.mode == 'candidate'")


if __name__ == "__main__":
    unittest.main()
