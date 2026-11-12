"""Exercise complete Git change sets and intentional versus accidental CI skips."""

import copy
import json
import os
from pathlib import Path
import runpy
import subprocess
import sys
import tempfile
import unittest

import yaml


ROOT = Path(__file__).resolve().parent.parent
scope = runpy.run_path(str(ROOT / "scripts/classify-ci-changes.py"))
gate = runpy.run_path(str(ROOT / "scripts/check-ci-results.py"))
REPO = "QubeTX/qube-network-diagnostics"


class PathPolicyTests(unittest.TestCase):
    def test_documentation_allowlist(self):
        for path in ("AGENTS.md", "CLAUDE.md", "CODEX_PROJECT.md", "ND300-Project-Plan.md",
                     "docs/guide.md", "docs/nested/guide.md", ".tasks/tasks/docci.md"):
            for status, old, new in (("M", "100644", "100644"), ("A", "000000", "100644"),
                                     ("D", "100644", "000000")):
                with self.subTest(path=path, status=status):
                    self.assertTrue(scope["documentation_change"]((status, old, new, path)))
        self.assertTrue(scope["documentation_change"](("M", "100644", "100644", "README.md")))

    def test_code_release_and_unknown_paths_remain_full(self):
        paths = ("src/main.rs", "src/notes.md", "Cargo.toml", "Cargo.lock", "build.rs",
                 "CHANGELOG.md", "HUMAN_CHANGELOG.md", "METHODOLOGY.md", "LICENSE",
                 "man/nd300.1", ".github/workflows/ci.yml", ".github/README.md",
                 "scripts/classify-ci-changes.py", "scripts/check-ci-results.py",
                 "wix/main.wxs", "inno/global.iss", "docs/evidence.json",
                 ".tasks/board-server.mjs", ".tasks/config.json", "unknown.md",
                 "docs/../src/code.md", "docs//guide.md", "docs/./guide.md",
                 "/docs/guide.md", "docs\\guide.md", "docs/guide.md\n", "docs/a\t.md")
        for path in paths:
            with self.subTest(path=path):
                self.assertFalse(scope["documentation_change"](("M", "100644", "100644", path)))

    def test_readme_removal_addition_and_special_modes_remain_full(self):
        for change in (("A", "000000", "100644", "README.md"),
                       ("D", "100644", "000000", "README.md"),
                       ("M", "100644", "100755", "docs/guide.md"),
                       ("M", "100755", "100755", "docs/guide.md"),
                       ("T", "100644", "120000", "docs/guide.md"),
                       ("M", "120000", "120000", "docs/guide.md"),
                       ("M", "160000", "160000", "docs/guide.md")):
            self.assertFalse(scope["documentation_change"](change))

    def test_raw_parser_rejects_malformed_records(self):
        for data in (b"unterminated", b"header\0", b"header\0path\0", b":x 100644 a b M\0x\0"):
            with self.subTest(data=data), self.assertRaises((ValueError, UnicodeError)):
                scope["parse_raw_diff"](data)


class GitScopeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.repo = Path(self.temp.name)
        self.git("init", "-b", "main")
        self.git("config", "user.name", "CI fixture")
        self.git("config", "user.email", "ci-fixture@example.invalid")
        self.git("config", "core.autocrlf", "false")
        self.git("config", "commit.gpgsign", "false")
        self.write("README.md", "readme\n")
        self.write("docs/guide.md", "guide\n")
        self.write("src/main.rs", "fn main() {}\n")
        self.base = self.save("baseline")

    def git(self, *args):
        return subprocess.run(["git", *args], cwd=self.repo, check=True, timeout=30,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE).stdout

    def write(self, path, text):
        file = self.repo / path
        file.parent.mkdir(parents=True, exist_ok=True)
        file.write_text(text, encoding="utf-8")

    def save(self, message):
        # This isolated disposable repository contains only this test's fixtures.
        self.git("add", ".")
        self.git("commit", "-m", message)
        return self.git("rev-parse", "HEAD").decode().strip()

    def push(self, base=None, head=None, **overrides):
        head = head or self.git("rev-parse", "HEAD").decode().strip()
        event = {"repository": {"full_name": REPO}, "ref": "refs/heads/main",
                 "before": base or self.base, "after": head, "created": False, "deleted": False}
        event.update(overrides)
        env = {"GITHUB_REPOSITORY": REPO, "GITHUB_EVENT_NAME": "push",
               "GITHUB_REF": "refs/heads/main", "GITHUB_SHA": head}
        return env, event

    def classify_push(self, **kwargs):
        return scope["classify"](*self.push(**kwargs), run=self.git)[0]

    def pr_merge(self, source_change=False):
        self.git("switch", "-c", "topic")
        if source_change:
            self.write("src/main.rs", "fn main() { println!(\"changed\"); }\n")
            self.save("earlier code change")
        self.write("docs/guide.md", "updated guide\n")
        head = self.save("last commit only docs")
        self.git("switch", "main")
        self.write("src/other.rs", "// independent base-branch change\n")
        base = self.save("base advances")
        self.git("merge", "--no-ff", "topic", "-m", "synthetic PR merge")
        merge = self.git("rev-parse", "HEAD").decode().strip()
        event = {"repository": {"full_name": REPO}, "number": 51, "pull_request": {
            "base": {"sha": base, "repo": {"full_name": REPO}},
            "head": {"sha": head, "repo": {"full_name": "contributor/fork"}},
        }}
        env = {"GITHUB_REPOSITORY": REPO, "GITHUB_EVENT_NAME": "pull_request",
               "GITHUB_REF": "refs/pull/51/merge", "GITHUB_SHA": merge}
        return env, event

    def test_ordinary_documentation_push_and_text_check(self):
        self.write("README.md", "readme edit  \n")
        self.write("docs/new.md", "new guide\n")
        self.save("documentation")
        self.assertTrue(self.classify_push())
        self.assertTrue(scope["classify"](*self.push(), run=self.git, check_docs=True)[0])

    def test_multi_commit_push_does_not_hide_earlier_code(self):
        self.write("src/main.rs", "fn main() { println!(\"code\"); }\n")
        middle = self.save("code")
        self.write("docs/guide.md", "documentation\n")
        self.save("docs")
        self.assertTrue(self.classify_push(base=middle))
        self.assertFalse(self.classify_push())

    def test_full_pr_diff_not_only_last_commit(self):
        self.assertFalse(scope["classify"](*self.pr_merge(source_change=True), run=self.git)[0])

    def test_docs_pr_against_advanced_base_and_fork(self):
        self.assertTrue(scope["classify"](*self.pr_merge(), run=self.git)[0])

    def test_merge_result_cannot_hide_code_change(self):
        env, event = self.pr_merge()
        self.write("src/main.rs", "fn main() { println!(\"merge result\"); }\n")
        self.git("add", "src/main.rs")
        self.git("commit", "--amend", "--no-edit")
        env["GITHUB_SHA"] = self.git("rev-parse", "HEAD").decode().strip()
        self.assertFalse(scope["classify"](env, event, run=self.git)[0])

    def test_rename_from_code_to_docs_is_full(self):
        self.git("mv", "src/main.rs", "docs/main.md")
        self.save("rename")
        self.assertFalse(self.classify_push())

    def test_docs_rename_and_delete_are_safe_but_readme_removal_is_full(self):
        self.git("mv", "docs/guide.md", "docs/renamed.md")
        middle = self.save("rename docs")
        self.assertTrue(self.classify_push())
        self.git("rm", "docs/renamed.md")
        self.save("remove guide")
        self.assertTrue(self.classify_push(base=middle))
        self.git("rm", "README.md")
        self.save("remove packaged readme")
        self.assertFalse(self.classify_push())

    def test_executable_mode_and_symlink_are_full_without_native_symlink_creation(self):
        self.git("update-index", "--chmod=+x", "docs/guide.md")
        self.git("commit", "-m", "executable docs")
        self.assertFalse(self.classify_push())
        blob = self.git("rev-parse", "HEAD:docs/guide.md").decode().strip()
        self.git("update-index", "--cacheinfo", "120000", blob, "docs/guide.md")
        self.git("commit", "-m", "symlink docs")
        self.assertFalse(self.classify_push())

    def test_more_than_300_changed_files_are_all_inspected(self):
        for number in range(305):
            self.write(f"docs/{number}.md", "guide\n")
        self.write("src/main.rs", "fn main() { println!(\"last\"); }\n")
        self.save("large mixed diff")
        self.assertFalse(self.classify_push())

    def test_empty_or_ambiguous_ranges_cannot_skip(self):
        self.assertFalse(self.classify_push())
        for updates in ({"before": "0" * 40}, {"before": "f" * 40}, {"created": True},
                        {"deleted": True}, {"after": "b" * 40}, {"created": None}):
            with self.subTest(updates=updates), self.assertRaises((ValueError, subprocess.SubprocessError)):
                scope["classify"](*self.push(**updates), run=self.git)

    def test_shallow_repository_cannot_skip_even_with_visible_diff_objects(self):
        self.write("docs/guide.md", "updated guide\n")
        head = self.save("docs")
        (self.repo / ".git" / "shallow").write_text(head + "\n", encoding="ascii")
        self.assertEqual(self.git("rev-parse", "--is-shallow-repository").strip(), b"true")
        with self.assertRaises(ValueError):
            scope["classify"](*self.push(), run=self.git)

    def test_repository_checkout_and_pr_identity_mismatches_cannot_skip(self):
        env, event = self.pr_merge()
        variants = []
        wrong_repo = copy.deepcopy(event)
        wrong_repo["repository"]["full_name"] = "other/repository"
        variants.append((env, wrong_repo))
        wrong_parent = copy.deepcopy(event)
        wrong_parent["pull_request"]["head"]["sha"] = self.base
        variants.append((env, wrong_parent))
        variants.append(({**env, "GITHUB_SHA": "e" * 40}, event))
        variants.append(({**env, "GITHUB_REF": "refs/pull/99/merge"}, event))
        for bad_env, bad_event in variants:
            with self.subTest(env=bad_env, event=bad_event), self.assertRaises(ValueError):
                scope["classify"](bad_env, bad_event, run=self.git)

    def test_explicit_release_and_manual_events_are_always_full(self):
        for name, ref in (("push", "refs/tags/v4.0.3"), ("workflow_dispatch", "refs/heads/main"),
                          ("workflow_call", "refs/tags/v4.0.3"), ("workflow_run", "refs/heads/main")):
            self.assertFalse(scope["classify"]({"GITHUB_EVENT_NAME": name, "GITHUB_REF": ref}, {})[0])

    def test_cli_missing_event_falls_back_to_full_and_binary_docs_fail_validation(self):
        output = self.repo / ".git" / "scope-output"
        env = {**os.environ, "GITHUB_EVENT_PATH": str(self.repo / "absent-event"),
               "GITHUB_OUTPUT": str(output)}
        result = subprocess.run([sys.executable, str(ROOT / "scripts/classify-ci-changes.py")],
                                cwd=self.repo, env=env, capture_output=True, check=False)
        self.assertEqual(result.returncode, 0)
        self.assertIn("docs_only=false", output.read_text())
        self.write("docs/binary.md", "binary\0data")
        self.save("binary masquerading as text")
        self.assertTrue(self.classify_push())
        with self.assertRaises(ValueError):
            scope["classify"](*self.push(), run=self.git, check_docs=True)


class RequiredGateTests(unittest.TestCase):
    def results(self, docs):
        result = {name: {"result": "success"} for name in gate["ALWAYS"]}
        result["changes"]["outputs"] = {"docs_only": str(docs).lower()}
        result.update({name: {"result": "skipped" if docs else "success"} for name in gate["EXPENSIVE"]})
        result["documentation"] = {"result": "success" if docs else "skipped"}
        return result

    def test_positive_full_and_docs_paths(self):
        self.assertEqual(gate["verify"](self.results(False)), "full")
        self.assertEqual(gate["verify"](self.results(True)), "documentation")

    def test_failures_cancellation_and_unexpected_skips_cannot_pass(self):
        for docs in (False, True):
            valid = self.results(docs)
            for name in valid:
                for outcome in ("failure", "cancelled", "skipped", "success", "unknown"):
                    if outcome == valid[name]["result"]:
                        continue
                    changed = copy.deepcopy(valid)
                    changed[name]["result"] = outcome
                    with self.subTest(docs=docs, name=name, outcome=outcome), self.assertRaises(ValueError):
                        gate["verify"](changed)

    def test_missing_extra_or_invalid_classification_cannot_pass(self):
        valid = self.results(False)
        for name in valid:
            broken = copy.deepcopy(valid)
            del broken[name]
            with self.assertRaises(ValueError):
                gate["verify"](broken)
        for value in (None, "", "TRUE", True):
            broken = copy.deepcopy(valid)
            broken["changes"]["outputs"]["docs_only"] = value
            with self.assertRaises(ValueError):
                gate["verify"](broken)
        valid["surprise"] = {"result": "success"}
        with self.assertRaises(ValueError):
            gate["verify"](valid)

    def test_workflow_wires_strict_gate_and_full_marker(self):
        data = yaml.load((ROOT / ".github/workflows/ci.yml").read_text(), Loader=yaml.BaseLoader)
        jobs = data["jobs"]
        self.assertEqual(set(jobs["required"]["needs"]), gate["ALWAYS"] | gate["EXPENSIVE"] | {"documentation"})
        self.assertEqual(jobs["required"]["if"], "${{ always() }}")
        self.assertEqual(jobs["changes"]["name"], "Change scope")
        self.assertEqual(jobs["changes"]["uses"], "./.github/workflows/change-scope.yml")
        for name in gate["EXPENSIVE"]:
            self.assertEqual(jobs[name]["needs"], "changes")
            self.assertEqual(jobs[name]["if"], "${{ needs.changes.outputs.docs_only == 'false' }}")
        self.assertEqual(set(jobs["full-ci"]["needs"]), {"changes", "required"})
        self.assertEqual(jobs["full-ci"]["name"], "Full CI")
        self.assertEqual(jobs["full-ci"]["if"], "${{ needs.changes.outputs.docs_only == 'false' }}")
        self.assertEqual(jobs["documentation"]["name"], "Documentation checks")
        self.assertNotIn("if", jobs["apple-policy"])
        self.assertNotIn("if", jobs["shellcheck"])


if __name__ == "__main__":
    unittest.main()
