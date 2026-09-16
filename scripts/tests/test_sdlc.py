import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("cubism_sdlc", ROOT / "scripts" / "sdlc.py")
sdlc = importlib.util.module_from_spec(SPEC)
assert SPEC.loader
SPEC.loader.exec_module(sdlc)


class SdlcTests(unittest.TestCase):
    def setUp(self):
        self.config = sdlc.load_config(ROOT / ".sdlc" / "config.json")
        self.epic = {
            "number": 10, "title": "Epic: Visible delivery", "state": "OPEN",
            "body": "## Child issues\n- [ ] #11\n", "labels": [],
            "url": "https://example.test/10", "updatedAt": "2026-01-01T00:00:00Z",
        }
        self.slice = {
            "number": 11, "title": "Ship a bounded thing", "state": "OPEN",
            "body": """## Outcome
Thing ships.
## Parent epic
Parent epic: #10
## Scope
One module.
## Acceptance criteria
- [ ] It works.
## Validation
Run tests.
## Demo / human-visible effect
Run the command.
## Non-goals
No redesign.
## Context
Read one file.
""",
            "labels": [{"name": "type:slice"}, {"name": "status:ready"}],
            "url": "https://example.test/11", "updatedAt": "2026-01-01T00:00:00Z",
        }

    def test_slice_contract_accepts_one_session_issue(self):
        issues = {10: self.epic, 11: self.slice}
        self.assertEqual([], sdlc.validate_slice(self.slice, issues, self.config))

    def test_issue_form_level_three_headings_are_accepted(self):
        self.slice["body"] = self.slice["body"].replace("## ", "### ")
        self.assertEqual([], sdlc.validate_slice(self.slice, {10: self.epic, 11: self.slice}, self.config))

    def test_human_gate_blocks_normal_slice(self):
        self.epic["labels"] = [{"name": "gate:human-review"}]
        errors = sdlc.validate_slice(self.slice, {10: self.epic, 11: self.slice}, self.config)
        self.assertIn(
            "parent epic #10 is paused for human review; no slice, including feedback, may proceed",
            errors,
        )

    def test_review_receipts_are_sha_specific(self):
        marker = sdlc.review_marker("codex", "abc123", "pass", "fresh-codex")
        receipts = sdlc.parse_review_receipts([{"body": marker, "user": {"login": "owner"}}])
        self.assertEqual("abc123", receipts[0]["head_sha"])
        pr = {
            "state": "OPEN", "isDraft": False, "baseRefName": "main",
            "mergeable": "MERGEABLE", "mergeStateStatus": "CLEAN",
            "headRefOid": "different", "body": "Closes #11",
            "statusCheckRollup": [{"name": "CI required checks", "conclusion": "SUCCESS"}],
        }
        errors = sdlc.evaluate_merge_gate(pr, [{"body": marker}], self.slice, self.config)
        self.assertTrue(any("missing clean `codex`" in error for error in errors))

    # --- merge gate: review independence and history resolution ---

    def _passing_pr(self, head="abc123", author="implementer"):
        return {
            "state": "OPEN", "isDraft": False, "baseRefName": "main",
            "mergeable": "MERGEABLE", "mergeStateStatus": "CLEAN",
            "headRefOid": head, "body": "Closes #11",
            "author": {"login": author},
            "statusCheckRollup": [{"name": "CI required checks", "conclusion": "SUCCESS"}],
        }

    def _receipt(self, author, verdict="pass", head="abc123", created_at=None):
        return {
            "body": sdlc.review_marker("codex", head, verdict, "fresh-codex"),
            "user": {"login": author},
            "created_at": created_at,
        }

    def test_independent_passing_receipt_satisfies_the_gate(self):
        errors = sdlc.evaluate_merge_gate(
            self._passing_pr(), [self._receipt("fresh-codex")], self.slice, self.config)
        self.assertEqual([], errors)

    def test_self_authored_receipt_cannot_satisfy_the_gate(self):
        errors = sdlc.evaluate_merge_gate(
            self._passing_pr(author="implementer"),
            [self._receipt("implementer")], self.slice, self.config)
        self.assertTrue(any("a self-review is not an independent review" in e for e in errors), errors)

    def test_self_authored_receipt_is_rejected_case_insensitively(self):
        errors = sdlc.evaluate_merge_gate(
            self._passing_pr(author="Implementer"),
            [self._receipt("implementer")], self.slice, self.config)
        self.assertTrue(any("self-review" in e for e in errors), errors)

    def test_unknown_pr_author_fails_closed(self):
        pr = self._passing_pr()
        del pr["author"]
        errors = sdlc.evaluate_merge_gate(pr, [self._receipt("fresh-codex")], self.slice, self.config)
        self.assertTrue(any("author is unknown" in e for e in errors), errors)

    def test_later_failure_supersedes_an_earlier_pass(self):
        comments = [
            self._receipt("fresh-codex", "pass", created_at="2026-01-01T00:00:00Z"),
            self._receipt("fresh-codex", "fail", created_at="2026-01-02T00:00:00Z"),
        ]
        errors = sdlc.evaluate_merge_gate(self._passing_pr(), comments, self.slice, self.config)
        self.assertTrue(any("is `fail`, not `pass`" in e for e in errors), errors)

    def test_pass_after_a_failure_clears_the_gate(self):
        comments = [
            self._receipt("fresh-codex", "fail", created_at="2026-01-01T00:00:00Z"),
            self._receipt("fresh-codex", "pass", created_at="2026-01-02T00:00:00Z"),
        ]
        self.assertEqual([], sdlc.evaluate_merge_gate(
            self._passing_pr(), comments, self.slice, self.config))

    def test_receipt_order_falls_back_to_comment_sequence(self):
        # No created_at (older comments, or a non-paginated fetch): ascending
        # API order still resolves newest-last.
        comments = [self._receipt("fresh-codex", "pass"), self._receipt("fresh-codex", "fail")]
        errors = sdlc.evaluate_merge_gate(self._passing_pr(), comments, self.slice, self.config)
        self.assertTrue(any("is `fail`, not `pass`" in e for e in errors), errors)

    # --- applied merge ---

    def test_applied_merge_builds_the_exact_gh_command(self):
        calls = []
        pr = self._passing_pr()
        original_inputs, original_run = sdlc.load_gate_inputs, sdlc.run_text
        sdlc.load_gate_inputs = lambda number, config: (pr, [self._receipt("fresh-codex")], self.slice)
        sdlc.run_text = lambda command, **kwargs: calls.append(command) or ""
        try:
            import argparse
            code = sdlc.command_merge(argparse.Namespace(pr=99, apply=True), self.config)
        finally:
            sdlc.load_gate_inputs, sdlc.run_text = original_inputs, original_run
        self.assertEqual(0, code)
        self.assertEqual(
            [["gh", "pr", "merge", "99", "--repo", self.config["repository"], "--merge", "--delete-branch"]],
            calls,
        )

    def test_blocked_merge_never_invokes_gh(self):
        calls = []
        original_inputs, original_run = sdlc.load_gate_inputs, sdlc.run_text
        sdlc.load_gate_inputs = lambda number, config: (
            self._passing_pr(), [self._receipt("implementer")], self.slice)
        sdlc.run_text = lambda command, **kwargs: calls.append(command) or ""
        try:
            import argparse
            code = sdlc.command_merge(argparse.Namespace(pr=99, apply=True), self.config)
        finally:
            sdlc.load_gate_inputs, sdlc.run_text = original_inputs, original_run
        self.assertEqual(1, code)
        self.assertEqual([], calls)

    # --- claim exclusivity ---

    def _stub_run_process(self, returncode, stderr="", stdout=""):
        calls = []

        def fake(args, **kwargs):
            calls.append(args)
            return subprocess.CompletedProcess(args, returncode, stdout, stderr)

        return fake, calls

    def test_claim_ref_creation_uses_the_create_only_endpoint(self):
        fake, calls = self._stub_run_process(0)
        original = sdlc.run_process
        sdlc.run_process = fake
        try:
            created = sdlc.create_remote_ref("refs/heads/issue/42", "deadbeef", self.config)
        finally:
            sdlc.run_process = original
        self.assertTrue(created)
        self.assertEqual(
            [[
                "gh", "api", "--method", "POST",
                f"repos/{self.config['repository']}/git/refs",
                "-f", "ref=refs/heads/issue/42",
                "-f", "sha=deadbeef",
            ]],
            calls,
        )

    def test_losing_a_claim_race_is_reported_not_swallowed(self):
        # GitHub answers 422 when the ref exists. `git push` of the same SHA
        # would instead exit 0 as an up-to-date no-op, which is the race.
        fake, _ = self._stub_run_process(1, stderr="gh: Reference already exists (HTTP 422)")
        original = sdlc.run_process
        sdlc.run_process = fake
        try:
            created = sdlc.create_remote_ref("refs/heads/issue/42", "deadbeef", self.config)
        finally:
            sdlc.run_process = original
        self.assertFalse(created)

    def test_unexpected_claim_failure_fails_closed(self):
        fake, _ = self._stub_run_process(1, stderr="gh: Bad credentials (HTTP 401)")
        original = sdlc.run_process
        sdlc.run_process = fake
        try:
            with self.assertRaises(sdlc.SdlcError):
                sdlc.create_remote_ref("refs/heads/issue/42", "deadbeef", self.config)
        finally:
            sdlc.run_process = original

    # --- human gates ---

    def test_feedback_slice_may_not_bypass_human_review(self):
        self.epic["labels"] = [{"name": "gate:human-review"}]
        self.slice["labels"] = [{"name": "type:feedback"}]
        errors = sdlc.validate_slice(self.slice, {10: self.epic, 11: self.slice}, self.config)
        self.assertTrue(any("no slice, including feedback, may proceed" in e for e in errors), errors)

    def test_feedback_slice_may_proceed_during_changes_requested(self):
        self.epic["labels"] = [{"name": "gate:changes-requested"}]
        self.slice["labels"] = [{"name": "type:feedback"}]
        self.assertEqual([], sdlc.validate_slice(self.slice, {10: self.epic, 11: self.slice}, self.config))

    def test_ordinary_slice_pauses_during_changes_requested(self):
        self.epic["labels"] = [{"name": "gate:changes-requested"}]
        errors = sdlc.validate_slice(self.slice, {10: self.epic, 11: self.slice}, self.config)
        self.assertTrue(any("only `type:feedback` slices may proceed" in e for e in errors), errors)

    def test_status_maps_epic_children_and_renders_html(self):
        data = {
            "repository": "example/repo", "generated_at": "2026-01-01T00:00:00+00:00",
            "issues": [self.epic, self.slice], "pulls": [],
        }
        model = sdlc.build_model(data, self.config)
        self.assertEqual(1, model["epics"][0]["total_children"])
        self.assertEqual(0, model["counts"]["unmapped"])
        page = sdlc.render_status(model)
        self.assertIn("Delivery cockpit", page)
        self.assertIn("Visible delivery", page)

    def test_related_reference_in_checkbox_is_not_a_child(self):
        self.epic["body"] = "## Workstreams\n- [ ] Build the bounded path (split as #11)\n"
        self.assertEqual(set(), sdlc.checkbox_children(self.epic))

    def test_part_of_statement_links_child_to_parent(self):
        self.slice["body"] = "Part of #10's remaining workstream.\n"
        self.assertEqual({10}, sdlc.parent_numbers(self.slice))

    def test_status_escapes_issue_titles(self):
        self.slice["title"] = "unsafe <script>alert(1)</script>"
        page = sdlc.render_status(sdlc.build_model({
            "repository": "example/repo", "generated_at": "now",
            "issues": [self.epic, self.slice], "pulls": [],
        }, self.config))
        self.assertNotIn("<script>", page)
        self.assertIn("&lt;script&gt;", page)


if __name__ == "__main__":
    unittest.main()
