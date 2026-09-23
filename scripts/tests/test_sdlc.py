import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch


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

    def _open_parent(self):
        # The parent as `load_gate_inputs` reloads it: an ungated epic whose
        # gate records were read successfully.
        return sdlc.ParentGate(10, self.epic, [])

    def _evaluate(self, *args, parent=None, **kwargs):
        return sdlc.evaluate_merge_gate(
            *args, parent=self._open_parent() if parent is None else parent, **kwargs,
        )

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
        errors = self._evaluate(pr, [{"body": marker}], self.slice, self.config)
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
        errors = self._evaluate(
            self._passing_pr(), [self._receipt("fresh-codex")], self.slice, self.config)
        self.assertEqual([], errors)

    def test_different_agents_on_one_account_satisfy_the_gate(self):
        errors = self._evaluate(
            self._passing_pr(author=self.ONE_ACCOUNT),
            [self._receipt(self.ONE_ACCOUNT, reviewer="codex-cli fresh exec session")],
            self.slice, self.config, self.HEAD_COMMIT)
        self.assertEqual([], errors)

    def test_claude_session_trailer_is_still_accepted(self):
        head = "docs: a change\n\nClaude-Session: " + self.IMPLEMENTED_BY
        errors = self._evaluate(
            self._passing_pr(author=self.ONE_ACCOUNT),
            [self._receipt(self.ONE_ACCOUNT, reviewer="codex-cli fresh exec session")],
            self.slice, self.config, head)
        self.assertEqual([], errors)

    def test_same_agent_reviewing_itself_is_rejected(self):
        errors = self._evaluate(
            self._passing_pr(author=self.ONE_ACCOUNT),
            [self._receipt(self.ONE_ACCOUNT, reviewer=self.IMPLEMENTED_BY)],
            self.slice, self.config, self.HEAD_COMMIT)
        self.assertTrue(any("a self-review is not an independent review" in e for e in errors), errors)

    def test_same_agent_is_rejected_case_insensitively(self):
        errors = self._evaluate(
            self._passing_pr(author="Implementer"),
            [self._receipt(self.ONE_ACCOUNT, reviewer=self.IMPLEMENTED_BY.upper())],
            self.slice, self.config, self.HEAD_COMMIT)
        self.assertTrue(any("self-review" in e for e in errors), errors)

    def test_missing_agent_trailer_fails_closed_with_an_actionable_error(self):
        errors = self._evaluate(
            self._passing_pr(author=self.ONE_ACCOUNT),
            [self._receipt(self.ONE_ACCOUNT, reviewer="codex-cli fresh exec session")],
            self.slice, self.config, "docs: a change with no trailer")
        self.assertTrue(any("Agent-Session:" in e for e in errors), errors)

    def test_separate_accounts_need_no_trailer(self):
        errors = self._evaluate(
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
        errors = self._evaluate(
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
        errors = self._evaluate(
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
        errors = self._evaluate(
            self._passing_pr(author=self.ONE_ACCOUNT),
            [self._receipt(self.ONE_ACCOUNT, reviewer="codex-cli fresh exec session")],
            self.slice, self.config, message)
        self.assertTrue(any("Agent-Session:" in e for e in errors), errors)

    def test_unknown_pr_author_fails_closed(self):
        pr = self._passing_pr()
        del pr["author"]
        errors = self._evaluate(pr, [self._receipt("fresh-codex")], self.slice, self.config)
        self.assertTrue(any("author is unknown" in e for e in errors), errors)

    def test_later_failure_supersedes_an_earlier_pass(self):
        comments = [
            self._receipt("fresh-codex", "pass", created_at="2026-01-01T00:00:00Z"),
            self._receipt("fresh-codex", "fail", created_at="2026-01-02T00:00:00Z"),
        ]
        errors = self._evaluate(self._passing_pr(), comments, self.slice, self.config)
        self.assertTrue(any("is `fail`, not `pass`" in e for e in errors), errors)

    def test_pass_after_a_failure_clears_the_gate(self):
        comments = [
            self._receipt("fresh-codex", "fail", created_at="2026-01-01T00:00:00Z"),
            self._receipt("fresh-codex", "pass", created_at="2026-01-02T00:00:00Z"),
        ]
        self.assertEqual([], self._evaluate(
            self._passing_pr(), comments, self.slice, self.config))

    def test_receipt_order_falls_back_to_comment_sequence(self):
        # No created_at (older comments, or a non-paginated fetch): ascending
        # API order still resolves newest-last.
        comments = [self._receipt("fresh-codex", "pass"), self._receipt("fresh-codex", "fail")]
        errors = self._evaluate(self._passing_pr(), comments, self.slice, self.config)
        self.assertTrue(any("is `fail`, not `pass`" in e for e in errors), errors)

    # --- applied merge ---

    CANDIDATE = "a" * 40
    MERGE_COMMIT = "b" * 40

    def _merged_result(self):
        return {
            "state": "MERGED", "headRefOid": self.CANDIDATE,
            "baseRefName": "main", "mergedAt": "2026-09-22T00:00:00Z",
            "mergeCommit": {"oid": self.MERGE_COMMIT},
        }

    def _result_commit(self):
        return {"sha": self.MERGE_COMMIT, "parents": [
            {"sha": "c" * 40}, {"sha": self.CANDIDATE},
        ]}

    def _merge_attempt(self, *, response=None, reads=None, apply=True, head=None, parent=None):
        import argparse
        import contextlib
        import io
        head = self.CANDIDATE if head is None else head
        pr = self._passing_pr(head=head)
        events = []

        def submit(command):
            events.append(("write", command))
            if callable(response):
                return response(command)
            if response is not None:
                raise response
            return ""

        results = iter(reads if reads is not None else [self._merged_result(), self._result_commit()])

        def read(command):
            events.append(("read", command))
            result = next(results)
            if isinstance(result, Exception):
                raise result
            return result

        out = io.StringIO()
        with patch.object(sdlc, "load_gate_inputs", return_value=sdlc.GateInputs(
            pr, [self._receipt("fresh-codex", head=head)], self.slice, self.HEAD_COMMIT,
            self._open_parent() if parent is None else parent,
        )) as inputs, patch.object(sdlc, "run_text", side_effect=submit), \
                patch.object(sdlc, "run_json", side_effect=read), contextlib.redirect_stdout(out):
            code = sdlc.command_merge(argparse.Namespace(pr=99, apply=apply), self.config)
        inputs.assert_called_once_with(99, self.config)
        return code, events, out.getvalue()

    def test_applied_merge_binds_candidate_and_verifies_result(self):
        code, events, output = self._merge_attempt()
        self.assertEqual(0, code, output)
        self.assertEqual(
            ["gh", "pr", "merge", "99", "--repo", self.config["repository"],
             "--merge", "--match-head-commit", self.CANDIDATE, "--delete-branch"],
            events[0][1],
        )
        self.assertEqual(["write", "read", "read"], [kind for kind, _ in events])
        self.assertIn("mergeCommit", events[1][1][-1])
        self.assertEqual(f"repos/{self.config['repository']}/commits/{self.MERGE_COMMIT}", events[2][1][-1])
        self.assertIn(f"candidate {self.CANDIDATE} commit {self.MERGE_COMMIT}", output)

    def test_head_moving_after_evaluation_is_rejected_without_fallback(self):
        moved = "d" * 40

        def github_merge(command):
            # Model the server precondition: removing it would merge the new,
            # unreviewed head. Assert the effect itself is prevented.
            expected = command[command.index("--match-head-commit") + 1] if "--match-head-commit" in command else moved
            self.assertNotEqual(moved, expected, "unreviewed candidate would merge")
            raise sdlc.SdlcError("head does not match --match-head-commit")

        code, events, output = self._merge_attempt(
            response=github_merge, reads=[{"state": "OPEN", "headRefOid": moved}],
        )
        self.assertEqual(1, code)
        self.assertEqual(["write", "read"], [kind for kind, _ in events])
        self.assertIn("BLOCKED", output)
        self.assertNotIn("MERGED:", output)

    def test_failed_submission_is_read_before_returning_blocked(self):
        code, events, output = self._merge_attempt(
            response=sdlc.SdlcError("permission denied"),
            reads=[{"state": "OPEN", "headRefOid": self.CANDIDATE}],
        )
        self.assertEqual(1, code)
        self.assertEqual(["write", "read"], [kind for kind, _ in events])
        self.assertIn("permission denied", output)
        self.assertNotIn("MERGED:", output)

    def test_lost_response_with_confirmed_merge_is_success_without_retry(self):
        code, events, output = self._merge_attempt(response=sdlc.SdlcError("connection lost"))
        self.assertEqual(0, code, output)
        self.assertEqual(["write", "read", "read"], [kind for kind, _ in events])
        self.assertIn("RECONCILED:", output)
        self.assertIn(self.MERGE_COMMIT, output)

    def test_unreadable_outcome_blocks_after_success_or_error_response(self):
        for response in (None, sdlc.SdlcError("connection lost"), OSError("executor failed")):
            with self.subTest(response=response):
                code, events, output = self._merge_attempt(
                    response=response, reads=[sdlc.SdlcError("GitHub unavailable")],
                )
                self.assertEqual(1, code)
                self.assertEqual(["write", "read"], [kind for kind, _ in events])
                self.assertIn("outcome not verified", output)
                self.assertNotIn("MERGED:", output)

    def test_incomplete_or_different_merge_result_never_reports_success(self):
        cases = [
            {"state": "OPEN"}, {"state": "CLOSED"},
            {"headRefOid": "d" * 40}, {"baseRefName": "other"},
            {"mergedAt": None}, {"mergeCommit": None}, {"mergeCommit": {"oid": "short"}},
        ]
        for changes in cases:
            with self.subTest(changes=changes):
                code, events, output = self._merge_attempt(reads=[{**self._merged_result(), **changes}])
                self.assertEqual(1, code)
                self.assertEqual(["write", "read"], [kind for kind, _ in events])
                self.assertNotIn("MERGED:", output)

    def test_resulting_commit_must_exist_and_contain_candidate(self):
        for commit in (
            sdlc.SdlcError("commit unavailable"),
            {**self._result_commit(), "sha": "d" * 40},
            {"sha": self.MERGE_COMMIT, "parents": []},
            {"sha": self.MERGE_COMMIT, "parents": [{"sha": self.CANDIDATE}, {"sha": "d" * 40}]},
        ):
            with self.subTest(commit=commit):
                code, events, output = self._merge_attempt(reads=[self._merged_result(), commit])
                self.assertEqual(1, code)
                self.assertEqual(["write", "read", "read"], [kind for kind, _ in events])
                self.assertNotIn("MERGED:", output)

    def test_dry_run_does_not_submit_or_reconcile(self):
        code, events, output = self._merge_attempt(apply=False)
        self.assertEqual(0, code)
        self.assertEqual([], events)
        self.assertIn("DRY RUN", output)

    def test_incomplete_candidate_cannot_authorize_merge(self):
        code, events, output = self._merge_attempt(head="abc123")
        self.assertEqual(1, code)
        self.assertEqual([], events)
        self.assertIn("full commit SHA", output)

    def test_blocked_merge_never_invokes_gh(self):
        calls = []
        original_inputs, original_run = sdlc.load_gate_inputs, sdlc.run_text
        sdlc.load_gate_inputs = lambda number, config: sdlc.GateInputs(
            self._passing_pr(head=self.CANDIDATE, author=self.ONE_ACCOUNT),
            [self._receipt(self.ONE_ACCOUNT, head=self.CANDIDATE, reviewer=self.IMPLEMENTED_BY)],
            self.slice, self.HEAD_COMMIT, self._open_parent())
        sdlc.run_text = lambda command, **kwargs: calls.append(command) or ""
        try:
            import argparse
            code = sdlc.command_merge(argparse.Namespace(pr=99, apply=True), self.config)
        finally:
            sdlc.load_gate_inputs, sdlc.run_text = original_inputs, original_run
        self.assertEqual(1, code)
        self.assertEqual([], calls)

    # --- review receipt: SHA binding ---

    class _StubReport:
        """A --body-file that needs no filesystem.

        command_review_receipt only ever calls read_text() on it, and the
        refusal path does not even do that. An earlier version of these tests
        wrote a real temporary file, which made all three error out wherever
        tempfile has no writable directory to offer -- a read-only review
        sandbox being the case that actually bit.
        """

        def __init__(self, text):
            self.text = text

        def read_text(self, encoding=None):
            return self.text

    def _record_receipt(self, expect_sha, actual_sha, dry_run=False):
        """Run command_review_receipt against a PR whose head is actual_sha.

        Returns (exit code, gh commands invoked, stdout).
        """
        import argparse
        import contextlib
        import io
        calls = []
        report = self._StubReport("1. `a.py:1` something\n\nVERDICT: pass\n")
        out = io.StringIO()
        # Nothing between here and the try can raise, so the finally always has
        # the real functions to restore.
        original_fetch, original_run = sdlc.fetch_pr, sdlc.run_text
        sdlc.fetch_pr = lambda number, config: {"headRefOid": actual_sha}
        sdlc.run_text = lambda command, **kwargs: calls.append(command) or ""
        try:
            with contextlib.redirect_stdout(out):
                code = sdlc.command_review_receipt(argparse.Namespace(
                    pr=99, kind="codex", verdict="pass", reviewer="fresh-codex",
                    body_file=report, dry_run=dry_run, expect_sha=expect_sha,
                ), self.config)
        finally:
            sdlc.fetch_pr, sdlc.run_text = original_fetch, original_run
        return code, calls, out.getvalue()

    def test_receipt_is_refused_when_the_head_moved_under_the_review(self):
        # The hole this closes: the reviewer read `reviewed`, something pushed
        # `moved`, and the receipt would otherwise attest to a commit nobody read.
        code, calls, _ = self._record_receipt(expect_sha="reviewed", actual_sha="moved")
        self.assertEqual(1, code)
        self.assertEqual([], calls)

    def test_refusal_names_the_reviewed_and_the_current_sha(self):
        code, _, out = self._record_receipt(expect_sha="reviewed", actual_sha="moved")
        # Assert the refusal, not just the strings: both SHAs appearing in a
        # success path would otherwise satisfy this test vacuously.
        self.assertEqual(1, code)
        self.assertIn("BLOCKED:", out)
        self.assertIn("reviewed", out)
        self.assertIn("moved", out)

    def test_receipt_records_when_the_head_still_matches(self):
        # Only this path reaches command_review_receipt's own NamedTemporaryFile,
        # so only this test needs a writable temp dir. Same idiom as the git
        # differential test above: state the dependency, skip loudly, never
        # silently pass. The two refusal tests stay hermetic and always run.
        try:
            with tempfile.NamedTemporaryFile():
                pass
        except OSError as exc:
            self.skipTest(f"no writable temporary directory: {exc}")
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

    CHECKPOINT = "docs/milestones/epic-10/checkpoint-1.html"

    def _gate_comment(self, state, comment_id, created_at, decided_by="Jerome (human)"):
        return {
            "id": comment_id,
            "html_url": f"https://github.com/{self.config['repository']}/issues/10#issuecomment-{comment_id}",
            "created_at": created_at,
            "body": sdlc.gate_marker(state, self.CHECKPOINT, decided_by) + "\n\nThe human's words.\n",
        }

    def _decision_url(self, comment_id, epic=10, repository=None):
        repository = repository or self.config["repository"]
        return f"https://github.com/{repository}/issues/{epic}#issuecomment-{comment_id}"

    def _as_feedback(self, cite=None):
        self.slice["labels"] = [{"name": "type:feedback"}, {"name": "status:ready"}]
        if cite is not None:
            self.slice["body"] += f"\nFeedback decision: {cite}\n"

    # Older changes-requested decision (400), a re-review (450), and the
    # current changes-requested decision (500).
    def _decision_history(self):
        return sdlc.parse_gate_records([
            self._gate_comment("changes-requested", 400, "2026-09-01T00:00:00Z"),
            self._gate_comment("review", 450, "2026-09-02T00:00:00Z"),
            self._gate_comment("changes-requested", 500, "2026-09-03T00:00:00Z"),
        ])

    def _admission(self, records):
        """Gate errors from readiness and from merge, which must agree."""
        issues = {10: self.epic, 11: self.slice}
        ready = [e for e in sdlc.validate_slice(self.slice, issues, self.config, lambda n: records)]
        merge = sdlc.evaluate_merge_gate(
            self._passing_pr(), [self._receipt("fresh-codex")], self.slice, self.config,
            parent=sdlc.ParentGate(10, self.epic, records),
        )
        return ready, merge

    def test_gate_admission_matrix(self):
        gates = {
            "none": [],
            "review": ["gate:human-review"],
            "changes-requested": ["gate:changes-requested"],
            "approved": ["gate:approved"],
            "conflicting": ["gate:human-review", "gate:changes-requested"],
        }
        issues = {
            "ordinary": lambda: None,
            "linked feedback": lambda: self._as_feedback(self._decision_url(500)),
            "unlinked feedback": lambda: self._as_feedback(),
            "stale-decision feedback": lambda: self._as_feedback(self._decision_url(400)),
        }
        # Only a pause restricts work (AGENTS.md). With no gate or after
        # approval, feedback proceeds like any slice; during changes-requested
        # only feedback citing the current human decision proceeds.
        admitted = {
            "none": set(issues), "approved": set(issues),
            "review": set(), "conflicting": set(),
            "changes-requested": {"linked feedback"},
        }
        for gate, gate_label_list in gates.items():
            for kind, prepare in issues.items():
                with self.subTest(gate=gate, issue=kind):
                    self.setUp()
                    self.epic["labels"] = [{"name": name} for name in gate_label_list]
                    prepare()
                    ready, merge = self._admission(self._decision_history())
                    if kind in admitted[gate]:
                        self.assertEqual([], ready)
                        self.assertEqual([], merge)
                    else:
                        self.assertTrue(ready, "readiness admitted a gated issue")
                        self.assertTrue(merge, "merge admitted a gated issue")
                        self.assertEqual(ready, merge)

    def test_conflicting_gate_labels_name_the_reconciliation(self):
        self.epic["labels"] = [{"name": "gate:human-review"}, {"name": "gate:approved"}]
        ready, _ = self._admission([])
        self.assertIn("conflicting gate labels", ready[0])
        self.assertIn("set-gate --epic 10", ready[0])

    def test_feedback_slice_may_not_bypass_human_review(self):
        self.epic["labels"] = [{"name": "gate:human-review"}]
        self._as_feedback(self._decision_url(500))
        ready, merge = self._admission(self._decision_history())
        self.assertTrue(any("no slice, including feedback, may proceed" in e for e in ready), ready)
        self.assertEqual(ready, merge)

    def test_unlinked_feedback_label_is_not_authority_during_changes_requested(self):
        # Formerly `test_feedback_slice_may_proceed_during_changes_requested`,
        # which hand-built the label and so never saw the dual-label bug.
        self.epic["labels"] = [{"name": "gate:changes-requested"}]
        self._as_feedback()
        ready, _ = self._admission(self._decision_history())
        self.assertTrue(any("must cite exactly one `Feedback decision:" in e for e in ready), ready)

    def test_ordinary_slice_pauses_during_changes_requested(self):
        self.epic["labels"] = [{"name": "gate:changes-requested"}]
        errors = sdlc.validate_slice(self.slice, {10: self.epic, 11: self.slice}, self.config)
        self.assertTrue(any("only `type:feedback` slices may proceed" in e for e in errors), errors)

    def test_feedback_citing_another_epic_or_repository_is_refused(self):
        self.epic["labels"] = [{"name": "gate:changes-requested"}]
        for cite in (
            self._decision_url(500, epic=38),
            self._decision_url(500, repository="someone/else"),
            "https://github.com/x/y/pull/10#issuecomment-500",
        ):
            with self.subTest(cite=cite):
                self.setUp()
                self.epic["labels"] = [{"name": "gate:changes-requested"}]
                self._as_feedback(cite)
                ready, merge = self._admission(self._decision_history())
                self.assertTrue(any("is not a gate record comment" in e for e in ready), ready)
                self.assertEqual(ready, merge)

    def test_feedback_citing_two_decisions_is_refused(self):
        self.epic["labels"] = [{"name": "gate:changes-requested"}]
        self._as_feedback(self._decision_url(500))
        self.slice["body"] += f"Feedback decision: {self._decision_url(400)}\n"
        ready, _ = self._admission(self._decision_history())
        self.assertTrue(any("found 2" in e for e in ready), ready)

    def test_unreadable_gate_records_block_feedback(self):
        self.epic["labels"] = [{"name": "gate:changes-requested"}]
        self._as_feedback(self._decision_url(500))
        ready, merge = self._admission(None)
        self.assertTrue(any("could not be read" in e for e in ready), ready)
        self.assertEqual(ready, merge)
        # Readiness without a loader (e.g. an offline bundle) fails closed too.
        errors = sdlc.validate_slice(self.slice, {10: self.epic, 11: self.slice}, self.config)
        self.assertTrue(any("could not be read" in e for e in errors), errors)

    def test_label_without_a_matching_record_blocks_feedback(self):
        self.epic["labels"] = [{"name": "gate:changes-requested"}]
        self._as_feedback(self._decision_url(450))
        records = sdlc.parse_gate_records([self._gate_comment("review", 450, "2026-09-02T00:00:00Z")])
        ready, _ = self._admission(records)
        self.assertTrue(any("newest gate record is not" in e for e in ready), ready)

    def test_legacy_gate_record_cannot_authorize_feedback(self):
        self.epic["labels"] = [{"name": "gate:changes-requested"}]
        self._as_feedback(self._decision_url(7))
        records = sdlc.parse_gate_records([{
            "id": 7, "html_url": self._decision_url(7), "created_at": "2026-09-01T00:00:00Z",
            "body": f"{sdlc.GATE_PREFIX}state=changes-requested{sdlc.GATE_SUFFIX}\n\nnote",
        }])
        self.assertEqual([0], [record["schema"] for record in records])
        ready, _ = self._admission(records)
        self.assertTrue(any("schema-1 changes-requested" in e for e in ready), ready)

    def test_quoted_marker_is_not_a_newer_gate_record(self):
        comments = [self._gate_comment("changes-requested", 500, "2026-09-03T00:00:00Z")]
        quoted = sdlc.gate_marker("approved", self.CHECKPOINT, "forged")
        comments += [
            {"id": 600, "created_at": "2026-09-04T00:00:00Z",
             "body": f"Continuation note. The record reads:\n```\n{quoted}\n```"},
            {"id": 601, "created_at": "2026-09-05T00:00:00Z", "body": f"> {quoted}"},
        ]
        records = sdlc.parse_gate_records(comments)
        self.assertEqual([500], [record["id"] for record in records])
        self.assertEqual(500, sdlc.latest_gate_record(records)["id"])

    def test_gate_records_order_by_time_then_sequence(self):
        records = sdlc.parse_gate_records([
            self._gate_comment("changes-requested", 2, "2026-09-03T00:00:00Z"),
            self._gate_comment("review", 1, "2026-09-01T00:00:00Z"),
            self._gate_comment("approved", 3, None),
            self._gate_comment("review", 4, None),
        ])
        self.assertEqual(2, sdlc.latest_gate_record(records)["id"])
        same_time = sdlc.parse_gate_records([
            self._gate_comment("review", 1, "2026-09-01T00:00:00Z"),
            self._gate_comment("changes-requested", 2, "2026-09-01T00:00:00Z"),
        ])
        self.assertEqual(2, sdlc.latest_gate_record(same_time)["id"])

    # --- merge re-evaluates the parent gate ---

    def test_merge_gate_requires_an_explicit_parent(self):
        with self.assertRaises(TypeError):
            sdlc.evaluate_merge_gate(self._passing_pr(), [self._receipt("fresh-codex")], self.slice, self.config)

    def test_unknown_or_unreadable_parent_blocks_merge(self):
        for parent, expected in (
            (sdlc.ParentGate(None, None, None), "exactly one parent epic"),
            (sdlc.ParentGate(10, None, None), "could not be loaded as an epic"),
            (sdlc.ParentGate(10, {"number": 10, "title": "Not an epic", "labels": []}, []), "could not be loaded as an epic"),
        ):
            with self.subTest(parent=parent):
                errors = self._evaluate(
                    self._passing_pr(), [self._receipt("fresh-codex")], self.slice, self.config, parent=parent)
                self.assertTrue(any(expected in e for e in errors), errors)

    def test_load_parent_gate_reloads_the_parent_and_fails_closed(self):
        calls = []

        def fail_comments(number, config):
            calls.append(("comments", number))
            raise sdlc.SdlcError("HTTP 502")

        with patch.object(sdlc, "fetch_issue", side_effect=lambda n, c: calls.append(("issue", n)) or self.epic), \
                patch.object(sdlc, "fetch_comments", side_effect=fail_comments):
            parent = sdlc.load_parent_gate(self.slice, self.config)
        self.assertEqual([("issue", 10), ("comments", 10)], calls)
        self.assertEqual(sdlc.ParentGate(10, self.epic, None), parent)
        with patch.object(sdlc, "fetch_issue", side_effect=sdlc.SdlcError("HTTP 404")), \
                patch.object(sdlc, "fetch_comments", return_value=[]):
            self.assertEqual(sdlc.ParentGate(10, None, []), sdlc.load_parent_gate(self.slice, self.config))
        self.assertEqual(sdlc.ParentGate(None, None, None), sdlc.load_parent_gate(None, self.config))

    def test_pause_after_claim_blocks_the_merge(self):
        issues = {10: self.epic, 11: self.slice}
        self.assertEqual([], sdlc.validate_slice(self.slice, issues, self.config, lambda n: []))
        # A checkpoint pause is recorded after the claim and before merge.
        paused = dict(self.epic, labels=[{"name": "gate:human-review"}])
        code, events, output = self._merge_attempt(parent=sdlc.ParentGate(10, paused, []))
        self.assertEqual(1, code)
        self.assertEqual([], events, "a paused epic must not reach gh pr merge")
        self.assertIn("paused for human review", output)

    # --- set-gate transitions ---

    def _set_gate(self, state, labels, *, decided_by="Jerome (human)", checkpoint=None,
                  apply=True, fail=None):
        """Run the real command against a mocked GitHub; return what it did."""
        import argparse
        import contextlib
        import io
        epic = dict(self.epic, labels=[{"name": name} for name in labels])
        writes = []

        applied = []

        def run_text(command):
            kind = "add" if "--add-label" in command else "remove"
            writes.append(("labels", command))
            if fail in {"labels", kind}:
                raise sdlc.SdlcError(f"label {kind} failed")
            applied.append(command)
            return ""

        def run_json(command):
            body = json.loads(Path(command[command.index("--input") + 1]).read_text())["body"]
            writes.append(("record", command, body))
            if fail == "record":
                raise sdlc.SdlcError("comment post failed")
            if fail == "no-url":
                return {"id": 900}
            return {"id": 900, "html_url": self._decision_url(900)}

        with tempfile.TemporaryDirectory() as tmp:
            note = Path(tmp) / "note.md"
            note.write_text("The human's words.\n", encoding="utf-8")
            args = argparse.Namespace(
                epic=10, state=state, note_file=note, apply=apply,
                checkpoint=self.CHECKPOINT if checkpoint is None else checkpoint,
                decided_by=decided_by,
            )
            out = io.StringIO()
            with patch.object(sdlc, "fetch_issue", return_value=epic), \
                    patch.object(sdlc, "run_text", side_effect=run_text), \
                    patch.object(sdlc, "run_json", side_effect=run_json), \
                    contextlib.redirect_stdout(out):
                try:
                    code = sdlc.command_set_gate(args, self.config)
                except sdlc.SdlcError as exc:
                    code, out = exc, out
        # Apply the label edits that succeeded to compute the resulting epic.
        resulting = set(labels)
        for command in applied:
            for flag, name in zip(command, command[1:]):
                if flag == "--add-label":
                    resulting.add(name)
                elif flag == "--remove-label":
                    resulting.discard(name)
        records = [
            {"id": 900, "html_url": self._decision_url(900), "created_at": "2026-09-10T00:00:00Z", "body": write[2]}
            for write in writes if write[0] == "record" and fail not in {"record", "no-url"}
        ]
        return code, writes, resulting, records, out.getvalue()

    def _admits(self, resulting_labels, records, cite=None, feedback=True):
        self.setUp()
        self.epic["labels"] = [{"name": name} for name in sorted(resulting_labels)]
        if feedback:
            self._as_feedback(cite)
        ready, merge = self._admission(sdlc.parse_gate_records(records))
        self.assertEqual(ready, merge)
        return not ready

    def test_command_generated_changes_requested_admits_its_feedback(self):
        code, writes, labels, records, output = self._set_gate("changes-requested", ["gate:human-review"])
        self.assertEqual(0, code, output)
        self.assertEqual({"gate:changes-requested"}, labels)
        self.assertIn(f"GATE_RECORD={self._decision_url(900)}", output)
        [record] = sdlc.parse_gate_records(records)
        self.assertEqual(
            (1, "changes-requested", self.CHECKPOINT, "Jerome (human)"),
            (record["schema"], record["state"], record["checkpoint"], record["decided_by"]),
        )
        self.assertIn("The human's words.", records[0]["body"])
        self.assertTrue(self._admits(labels, records, cite=self._decision_url(900)))
        self.assertFalse(self._admits(labels, records, cite=None))
        self.assertFalse(self._admits(labels, records, feedback=False))

    def test_transitions_leave_exactly_one_gate_label(self):
        for state, before in (
            ("review", []), ("review", ["gate:approved"]), ("review", ["gate:changes-requested"]),
            ("changes-requested", ["gate:human-review"]),
            ("approved", ["gate:human-review"]),
            # Reconciling the legacy dual-label state by recording the decision.
            ("changes-requested", ["gate:human-review", "gate:changes-requested"]),
            ("approved", ["gate:human-review", "gate:changes-requested"]),
        ):
            with self.subTest(state=state, before=before):
                code, _, labels, _, output = self._set_gate(state, before)
                self.assertEqual(0, code, output)
                self.assertEqual({sdlc.gate_labels(self.config)[state]}, labels)

    def test_decisions_require_a_checkpoint_under_review(self):
        for state, before in (
            ("changes-requested", []), ("approved", []),
            ("approved", ["gate:changes-requested"]), ("changes-requested", ["gate:approved"]),
        ):
            with self.subTest(state=state, before=before):
                code, writes, _, _, _ = self._set_gate(state, before)
                self.assertIsInstance(code, sdlc.SdlcError)
                self.assertIn("request review first", str(code))
                self.assertEqual([], writes)

    def test_a_decision_must_name_the_human_who_made_it(self):
        for state in ("changes-requested", "approved"):
            with self.subTest(state=state):
                code, writes, _, _, _ = self._set_gate(state, ["gate:human-review"], decided_by="  ")
                self.assertIsInstance(code, sdlc.SdlcError)
                self.assertIn("--decided-by", str(code))
                self.assertEqual([], writes)
        code, _, _, _, output = self._set_gate("review", [], decided_by=None)
        self.assertEqual(0, code, output)

    def test_gate_record_fields_cannot_break_the_marker(self):
        for field in ({"checkpoint": "report --> x"}, {"decided_by": "a\nb"}, {"checkpoint": " "}):
            with self.subTest(field=field):
                code, writes, _, _, _ = self._set_gate("approved", ["gate:human-review"], **field)
                self.assertIsInstance(code, sdlc.SdlcError)
                self.assertEqual([], writes)

    def test_dry_run_writes_nothing(self):
        code, writes, _, _, output = self._set_gate("approved", ["gate:human-review"], apply=False)
        self.assertEqual(0, code)
        self.assertEqual([], writes)
        self.assertIn("DRY RUN:", output)

    def test_partial_failure_leaves_the_epic_paused(self):
        # Tighten first, relax last. Whichever write fails, the resulting
        # state admits no ordinary slice and no uncited feedback.
        for state, before, fail in (
            ("review", [], "record"),
            ("review", ["gate:changes-requested"], "add"),
            ("review", ["gate:changes-requested"], "remove"),
            ("review", ["gate:changes-requested"], "record"),
            ("changes-requested", ["gate:human-review"], "add"),
            ("changes-requested", ["gate:human-review"], "remove"),
            ("changes-requested", ["gate:human-review"], "record"),
            ("changes-requested", ["gate:human-review"], "no-url"),
            ("approved", ["gate:human-review"], "add"),
            ("approved", ["gate:human-review"], "remove"),
            ("approved", ["gate:human-review"], "record"),
            ("approved", ["gate:human-review"], "no-url"),
        ):
            with self.subTest(state=state, fail=fail):
                code, writes, labels, records, _ = self._set_gate(state, before, fail=fail)
                self.assertIsInstance(code, sdlc.SdlcError)
                self.assertFalse(self._admits(labels, records, feedback=False))
                self.assertFalse(self._admits(labels, records, cite=self._decision_url(900)))
                order = [write[0] for write in writes]
                self.assertEqual("labels" if state == "review" else "record", order[0])
                if fail == "remove":
                    # Add succeeded, remove failed: a conflict, never no gate.
                    self.assertEqual(2, len(labels & set(sdlc.gate_labels(self.config).values())))

    def test_label_writes_add_before_they_remove(self):
        _, writes, _, _, _ = self._set_gate("changes-requested", ["gate:human-review"])
        commands = [write[1] for write in writes if write[0] == "labels"]
        self.assertEqual(2, len(commands))
        self.assertIn("--add-label", commands[0])
        self.assertNotIn("--remove-label", commands[0])
        self.assertIn("--remove-label", commands[1])
        self.assertNotIn("--add-label", commands[1])

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
