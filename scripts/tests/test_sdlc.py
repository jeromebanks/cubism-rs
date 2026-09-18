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

    def _receipt(self, author, verdict="pass", head="abc123", created_at=None, reviewer="fresh-codex"):
        return {
            "body": sdlc.review_marker("codex", head, verdict, reviewer),
            "user": {"login": author},
            "created_at": created_at,
        }

    # One human account drives several agents, so the GitHub login is the same
    # on both sides of the review and the agent identities are what differ.
    ONE_ACCOUNT = "implementer"
    IMPLEMENTED_BY = "Claude Opus 5 session_abc"
    HEAD_COMMIT = "docs: a change\n\nAgent-Session: " + IMPLEMENTED_BY

    def test_independent_passing_receipt_satisfies_the_gate(self):
        errors = sdlc.evaluate_merge_gate(
            self._passing_pr(), [self._receipt("fresh-codex")], self.slice, self.config)
        self.assertEqual([], errors)

    def test_different_agents_on_one_account_satisfy_the_gate(self):
        errors = sdlc.evaluate_merge_gate(
            self._passing_pr(author=self.ONE_ACCOUNT),
            [self._receipt(self.ONE_ACCOUNT, reviewer="codex-cli fresh exec session")],
            self.slice, self.config, self.HEAD_COMMIT)
        self.assertEqual([], errors)

    def test_claude_session_trailer_is_still_accepted(self):
        head = "docs: a change\n\nClaude-Session: " + self.IMPLEMENTED_BY
        errors = sdlc.evaluate_merge_gate(
            self._passing_pr(author=self.ONE_ACCOUNT),
            [self._receipt(self.ONE_ACCOUNT, reviewer="codex-cli fresh exec session")],
            self.slice, self.config, head)
        self.assertEqual([], errors)

    def test_same_agent_reviewing_itself_is_rejected(self):
        errors = sdlc.evaluate_merge_gate(
            self._passing_pr(author=self.ONE_ACCOUNT),
            [self._receipt(self.ONE_ACCOUNT, reviewer=self.IMPLEMENTED_BY)],
            self.slice, self.config, self.HEAD_COMMIT)
        self.assertTrue(any("a self-review is not an independent review" in e for e in errors), errors)

    def test_same_agent_is_rejected_case_insensitively(self):
        errors = sdlc.evaluate_merge_gate(
            self._passing_pr(author="Implementer"),
            [self._receipt(self.ONE_ACCOUNT, reviewer=self.IMPLEMENTED_BY.upper())],
            self.slice, self.config, self.HEAD_COMMIT)
        self.assertTrue(any("self-review" in e for e in errors), errors)

    def test_missing_agent_trailer_fails_closed_with_an_actionable_error(self):
        errors = sdlc.evaluate_merge_gate(
            self._passing_pr(author=self.ONE_ACCOUNT),
            [self._receipt(self.ONE_ACCOUNT, reviewer="codex-cli fresh exec session")],
            self.slice, self.config, "docs: a change with no trailer")
        self.assertTrue(any("Agent-Session:" in e for e in errors), errors)

    def test_separate_accounts_need_no_trailer(self):
        errors = sdlc.evaluate_merge_gate(
            self._passing_pr(author=self.ONE_ACCOUNT),
            [self._receipt("reviewer-bot", reviewer="codex-cli fresh exec session")],
            self.slice, self.config, "docs: a change with no trailer")
        self.assertEqual([], errors)

    def test_implementer_identity_reads_the_last_trailer(self):
        message = "x\n\nAgent-Session: first\nAgent-Session: second"
        self.assertEqual("second", sdlc.implementer_identity(message))
        self.assertIsNone(sdlc.implementer_identity("no trailer here"))

    def test_newer_self_review_failure_outranks_an_older_independent_pass(self):
        # A veto needs no independence. Regression test for the case where
        # filtering dependent receipts before resolution discarded a newer
        # failure and let a stale pass authorize the merge.
        comments = [
            self._receipt(self.ONE_ACCOUNT, "pass", reviewer="codex-cli fresh exec session",
                          created_at="2026-01-01T00:00:00Z"),
            self._receipt(self.ONE_ACCOUNT, "fail", reviewer=self.IMPLEMENTED_BY,
                          created_at="2026-01-02T00:00:00Z"),
        ]
        errors = sdlc.evaluate_merge_gate(
            self._passing_pr(author=self.ONE_ACCOUNT), comments,
            self.slice, self.config, self.HEAD_COMMIT)
        self.assertTrue(any("is `fail`, not `pass`" in e for e in errors), errors)

    def test_a_dependent_pass_still_cannot_authorize(self):
        comments = [
            self._receipt(self.ONE_ACCOUNT, "fail", reviewer="codex-cli fresh exec session",
                          created_at="2026-01-01T00:00:00Z"),
            self._receipt(self.ONE_ACCOUNT, "pass", reviewer=self.IMPLEMENTED_BY,
                          created_at="2026-01-02T00:00:00Z"),
        ]
        errors = sdlc.evaluate_merge_gate(
            self._passing_pr(author=self.ONE_ACCOUNT), comments,
            self.slice, self.config, self.HEAD_COMMIT)
        self.assertTrue(errors, "a newer self-authored pass must not clear an independent failure")

    def test_trailer_in_prose_does_not_create_an_identity(self):
        message = "subject\n\nThe convention is `Agent-Session: someone`.\n\nplain closing prose"
        self.assertIsNone(sdlc.implementer_identity(message))

    def test_a_one_paragraph_message_has_no_trailers(self):
        # `git interpret-trailers --parse` finds nothing in either of these.
        self.assertIsNone(sdlc.implementer_identity("Agent-Session: solo"))
        self.assertIsNone(sdlc.implementer_identity("subject\nAgent-Session: no-blank"))

    def test_implementer_identity_agrees_with_git_interpret_trailers(self):
        """Differential test: our parser must not diverge from git's.

        The trailer rules are git's, not ours, and three review rounds were
        spent on cases where a hand-rolled approximation disagreed. Asserting
        parity directly is cheaper than enumerating the next disagreement.
        """
        import shutil, subprocess
        if not shutil.which("git"):
            self.skipTest("git is unavailable")
        cases = [
            "Agent-Session: solo",
            "subject\nAgent-Session: no-blank",
            "subject\n\nAgent-Session: real",
            "subject\n\nplain prose\nAgent-Session: invented",
            "subject\n\nCo-Authored-By: x\nAgent-Session: real2",
            "subject\n\nAgent-Session: first\nAgent-Session: second",
            "subject\n\nbody\n\nAgent-Session: trailing  ",
            "subject\n\nAgent-Session: body-para\n\njust prose",
            "",
            "   ",
            "subject\r\n\r\nAgent-Session: crlf",
            "subject\n\nCloses #1\n\nCo-Authored-By: a\nAgent-Session: last",
        ]
        for message in cases:
            with self.subTest(message=message):
                parsed = subprocess.run(
                    ["git", "interpret-trailers", "--parse"],
                    input=message, capture_output=True, text=True).stdout
                values = [
                    line.split(":", 1)[1].strip() for line in parsed.splitlines()
                    if line.split(":", 1)[0].lower()
                    in {key.lower() for key in sdlc.AGENT_SESSION_TRAILERS}
                ]
                self.assertEqual(values[-1] if values else None,
                                 sdlc.implementer_identity(message))

    def test_implementer_identity_never_diverges_from_git_under_fuzzing(self):
        """Randomised differential test against git, fixed seed.

        Four review rounds were each spent on one more hand-found shape where a
        hand-rolled approximation disagreed with git. Enumerating shapes loses
        that game; sampling the space closes it. The seed is fixed so a failure
        is reproducible, and any divergence is printed with the offending
        message.
        """
        import random, shutil, subprocess
        if not shutil.which("git"):
            self.skipTest("git is unavailable")
        pieces = [
            "subject", "", "body prose", "Other: x", " continued", "\ttabbed",
            "Co-Authored-By: someone <a@b.c>", "Agent-Session: one",
            "Claude-Session: https://example/s", "Agent-Session: two",
            "Closes #1", "Agent-Session:", "Agent-Session:    spaced   ",
            "agent-session: lower", "prose Agent-Session: inline", "   ",
        ]
        rng = random.Random(20260917)
        for _ in range(400):
            message = "\n".join(rng.choice(pieces) for _ in range(rng.randint(1, 7)))
            parsed = subprocess.run(
                ["git", "interpret-trailers", "--parse"],
                input=message, capture_output=True, text=True).stdout
            values = [
                line.split(":", 1)[1].strip() for line in parsed.splitlines()
                if line.split(":", 1)[0].lower()
                in {key.lower() for key in sdlc.AGENT_SESSION_TRAILERS}
            ]
            expected = values[-1] if values else None
            actual = sdlc.implementer_identity(message)
            # One-directional on purpose. The property the gate needs is that we
            # never *invent* an identity git would not produce: under-rejecting
            # merges unreviewed work, while over-rejecting only emits an
            # actionable "add an Agent-Session: trailer" error. So a stricter
            # answer than git's is acceptable; a different or invented one is
            # not. (Git accepts a subject-less paragraph of pure trailers, which
            # `implementer_identity` declines; real commit messages have a
            # subject, and declining is the safe side.)
            self.assertIn(
                actual, (expected, None),
                f"invented an identity git does not produce, for message: {message!r}",
            )

    def test_a_folded_continuation_line_is_not_a_trailer(self):
        # git folds an indented line into the preceding trailer.
        self.assertIsNone(
            sdlc.implementer_identity("subject\n\nOther: x\n Agent-Session: invented"))

    def test_prose_mixed_into_the_final_paragraph_is_not_a_trailer_block(self):
        # `git interpret-trailers --parse` finds nothing here either.
        self.assertIsNone(sdlc.implementer_identity("subject\n\nplain prose\nAgent-Session: invented"))

    def test_only_the_final_trailer_block_is_authoritative(self):
        message = ("subject\n\nAgent-Session: mentioned-in-body\n\n"
                   "Co-Authored-By: x\nAgent-Session: real-session")
        self.assertEqual("real-session", sdlc.implementer_identity(message))

    def test_prose_trailer_still_fails_closed_at_the_gate(self):
        message = "subject\n\nAgent-Session: fake\n\njust prose, no trailer block"
        errors = sdlc.evaluate_merge_gate(
            self._passing_pr(author=self.ONE_ACCOUNT),
            [self._receipt(self.ONE_ACCOUNT, reviewer="codex-cli fresh exec session")],
            self.slice, self.config, message)
        self.assertTrue(any("Agent-Session:" in e for e in errors), errors)

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
        sdlc.load_gate_inputs = lambda number, config: (pr, [self._receipt("fresh-codex")], self.slice, self.HEAD_COMMIT)
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
            self._passing_pr(author=self.ONE_ACCOUNT),
            [self._receipt(self.ONE_ACCOUNT, reviewer=self.IMPLEMENTED_BY)],
            self.slice, self.HEAD_COMMIT)
        sdlc.run_text = lambda command, **kwargs: calls.append(command) or ""
        try:
            import argparse
            code = sdlc.command_merge(argparse.Namespace(pr=99, apply=True), self.config)
        finally:
            sdlc.load_gate_inputs, sdlc.run_text = original_inputs, original_run
        self.assertEqual(1, code)
        self.assertEqual([], calls)

    # --- review receipt: SHA binding ---

    def _record_receipt(self, expect_sha, actual_sha, dry_run=False):
        """Run command_review_receipt against a PR whose head is actual_sha.

        Returns (exit code, gh commands invoked, stdout).
        """
        import argparse
        import contextlib
        import io
        calls = []
        original_fetch, original_run = sdlc.fetch_pr, sdlc.run_text
        sdlc.fetch_pr = lambda number, config: {"headRefOid": actual_sha}
        sdlc.run_text = lambda command, **kwargs: calls.append(command) or ""
        with tempfile.NamedTemporaryFile("w", suffix=".md", delete=False) as handle:
            handle.write("1. `a.py:1` something\n\nVERDICT: pass\n")
            report = Path(handle.name)
        out = io.StringIO()
        try:
            with contextlib.redirect_stdout(out):
                code = sdlc.command_review_receipt(argparse.Namespace(
                    pr=99, kind="codex", verdict="pass", reviewer="fresh-codex",
                    body_file=report, dry_run=dry_run, expect_sha=expect_sha,
                ), self.config)
        finally:
            sdlc.fetch_pr, sdlc.run_text = original_fetch, original_run
            report.unlink(missing_ok=True)
        return code, calls, out.getvalue()

    def test_receipt_is_refused_when_the_head_moved_under_the_review(self):
        # The hole this closes: the reviewer read `reviewed`, something pushed
        # `moved`, and the receipt would otherwise attest to a commit nobody read.
        code, calls, _ = self._record_receipt(expect_sha="reviewed", actual_sha="moved")
        self.assertEqual(1, code)
        self.assertEqual([], calls)

    def test_refusal_names_the_reviewed_and_the_current_sha(self):
        _, _, out = self._record_receipt(expect_sha="reviewed", actual_sha="moved")
        self.assertIn("reviewed", out)
        self.assertIn("moved", out)

    def test_receipt_records_when_the_head_still_matches(self):
        code, calls, out = self._record_receipt(expect_sha="same", actual_sha="same")
        self.assertEqual(0, code)
        self.assertEqual(1, len(calls))
        self.assertEqual(["gh", "pr", "comment", "99"], calls[0][:4])
        self.assertIn("RECORDED: codex=pass for same", out)

    # --- on-the-wire protocol identifiers ---

    def test_comment_markers_are_pinned_and_project_neutral(self):
        # These strings are format, not branding. They are written into GitHub
        # comment bodies and parsed back out, so changing one orphans every
        # receipt and gate record already posted. Pinned deliberately: if this
        # test fails, the change needs a migration, not a new expectation.
        self.assertEqual("<!-- nightshift-review ", sdlc.REVIEW_PREFIX)
        self.assertEqual(" -->", sdlc.REVIEW_SUFFIX)
        self.assertEqual("<!-- nightshift-gate ", sdlc.GATE_PREFIX)
        self.assertEqual(" -->", sdlc.GATE_SUFFIX)
        for marker in (sdlc.REVIEW_PREFIX, sdlc.GATE_PREFIX):
            self.assertNotIn("cubism", marker)

    def test_receipt_round_trips_through_its_marker(self):
        body = sdlc.review_marker("codex", "abc123", "pass", "fresh-codex")
        [receipt] = sdlc.parse_review_receipts([{"body": body, "user": {"login": "r"}}])
        self.assertEqual(
            ("codex", "abc123", "pass", "fresh-codex"),
            (receipt["kind"], receipt["head_sha"], receipt["verdict"], receipt["reviewer"]),
        )

    def test_cockpit_title_names_the_repository_it_describes(self):
        page = sdlc.render_status(sdlc.build_model({
            "repository": "acme/widgets", "generated_at": "now",
            "issues": [self.epic, self.slice], "pulls": [],
        }, self.config))
        self.assertIn("<title>acme/widgets delivery cockpit</title>", page)

    # --- status projection ---

    def test_status_projection_drops_prose_but_keeps_state(self):
        model = sdlc.build_model({
            "repository": "example/repo", "generated_at": "now",
            "issues": [self.epic, self.slice], "pulls": [],
        }, self.config)
        projected = sdlc.status_projection(model)
        rendered = json.dumps(projected)
        self.assertNotIn("Parent epic: #10", rendered)
        self.assertNotIn('"body"', rendered)
        self.assertEqual(model["counts"], projected["counts"])
        self.assertEqual(1, projected["epics"][0]["total_children"])

    def test_status_projection_leaves_the_model_intact(self):
        model = sdlc.build_model({
            "repository": "example/repo", "generated_at": "now",
            "issues": [self.epic, self.slice], "pulls": [],
        }, self.config)
        sdlc.status_projection(model)
        # render_status still needs the untouched model afterwards.
        self.assertIn("Visible delivery", sdlc.render_status(model))

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
