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

# The live GitHub readers, captured before any test patches them, so a test
# can drive the real reader against a mocked `run_json`.
REAL_READERS = {
    name: getattr(sdlc, name) for name in ("fetch_blocked_by", "fetch_prerequisite", "fetch_pull_merge")
}


class SdlcTests(unittest.TestCase):
    def setUp(self):
        self.config = sdlc.load_config(ROOT / ".sdlc" / "config.json")
        # No test may reach GitHub. A reader a test forgot to patch fails
        # loudly instead of reading this repository's real issues.
        guard = patch.object(sdlc, "run_json", side_effect=AssertionError("unpatched GitHub read"))
        guard.start()
        self.addCleanup(guard.stop)
        # The native dependency graph every command reads, live or bundled.
        # By default the slice has no prerequisites. A value that is an
        # exception is raised, as an unreadable GitHub answer would be.
        self.graph = {"blocked_by": {11: []}, "issues": {}, "pulls": {}}
        for name, key in (
            ("fetch_blocked_by", "blocked_by"),
            ("fetch_prerequisite", "issues"),
            ("fetch_pull_merge", "pulls"),
        ):
            reader = patch.object(sdlc, name, side_effect=lambda n, _config, key=key: self._graph_read(key, n))
            reader.start()
            self.addCleanup(reader.stop)
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

    # A gate-record loader that read the epic's comments and found no records.
    READ_OK = staticmethod(lambda _number: [])

    @staticmethod
    def _unreachable(number):
        raise AssertionError(f"no prerequisite exists, yet #{number} was read")

    # A dependency source that read the slice's blockers and found none.
    NO_BLOCKERS = sdlc.DependencySource(lambda _number: [], _unreachable, _unreachable)

    def _graph_read(self, key, number):
        if number not in self.graph[key]:
            raise sdlc.SdlcError(f"HTTP 404: no {key} for #{number}")
        value = self.graph[key][number]
        if isinstance(value, Exception):
            raise value
        return value

    def _graph_source(self):
        return sdlc.DependencySource(*(
            (lambda n, key=key: self._graph_read(key, n)) for key in ("blocked_by", "issues", "pulls")
        ))

    def _graph_bundle(self):
        """The graph as an offline bundle's keys; unreadable entries are absent."""
        return {
            "dependencies": {
                str(n): v for n, v in self.graph["blocked_by"].items() if not isinstance(v, Exception)
            },
            "pull_requests": {str(n): v for n, v in self.graph["pulls"].items()},
            "prerequisites": list(self.graph["issues"].values()),
        }

    REPO_API = "https://api.github.com/repos/jeromebanks/cubism-rs"

    def _blocker(self, number, repo_api=None):
        # One entry of the REST blocked_by list (see the #98 probe).
        return {"number": number, "repository_url": repo_api or self.REPO_API,
                "html_url": f"https://github.com/jeromebanks/cubism-rs/issues/{number}",
                "state": "closed", "state_reason": "completed", "id": 900000 + number}

    def _prerequisite(self, number, state="CLOSED", reason="COMPLETED", prs=(), blocked_by=()):
        self.graph["issues"][number] = {
            "number": number, "state": state, "stateReason": reason,
            "closedByPullRequestsReferences": [
                {"number": pr, "repository": {"name": "cubism-rs", "owner": {"login": "jeromebanks"}}}
                for pr in prs
            ],
        }
        self.graph["blocked_by"][number] = [self._blocker(n) for n in blocked_by]

    def _pull(self, number, state="MERGED", base="main", oid="e" * 40):
        self.graph["pulls"][number] = {
            "number": number, "state": state, "baseRefName": base, "mergeCommit": {"oid": oid} if oid else None,
        }

    def _blocked_by(self, *numbers):
        self.graph["blocked_by"][11] = [self._blocker(n) for n in numbers]

    def _prerequisite_errors(self):
        return sdlc.prerequisite_errors(11, self._graph_source(), self.config)

    def _open_parent(self):
        # The parent as `load_gate_inputs` reloads it: an ungated epic whose
        # gate records were read successfully.
        return sdlc.ParentGate(10, self.epic, [])

    def _evaluate(self, *args, parent=None, **kwargs):
        kwargs.setdefault("prerequisites", ())
        return sdlc.evaluate_merge_gate(
            *args, parent=self._open_parent() if parent is None else parent, **kwargs,
        )

    def test_slice_contract_accepts_one_session_issue(self):
        issues = {10: self.epic, 11: self.slice}
        self.assertEqual([], sdlc.validate_slice(self.slice, issues, self.config, self.READ_OK, self.NO_BLOCKERS))

    def test_issue_form_level_three_headings_are_accepted(self):
        self.slice["body"] = self.slice["body"].replace("## ", "### ")
        self.assertEqual([], sdlc.validate_slice(self.slice, {10: self.epic, 11: self.slice}, self.config, self.READ_OK, self.NO_BLOCKERS))

    def test_human_gate_blocks_normal_slice(self):
        self.epic["labels"] = [{"name": "gate:human-review"}]
        errors = sdlc.validate_slice(self.slice, {10: self.epic, 11: self.slice}, self.config, self.READ_OK, self.NO_BLOCKERS)
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
        # An ordinary single-parent commit (the default head_parent_count=1)
        # gets the trailer advice, never the merge-commit/rebase advice.
        self.assertFalse(any("rebase" in e for e in errors), errors)

    def test_separate_accounts_need_no_trailer(self):
        errors = self._evaluate(
            self._passing_pr(author=self.ONE_ACCOUNT),
            [self._receipt("reviewer-bot", reviewer="codex-cli fresh exec session")],
            self.slice, self.config, "docs: a change with no trailer")
        self.assertEqual([], errors)

    # --- #75: a `gh pr update-branch` merge-commit head names rebase, not a trailer ---

    MERGE_COMMIT_HEAD = "Merge branch 'main' into issue/11"

    def test_merge_commit_head_with_no_trailer_names_rebase_not_a_trailer(self):
        errors = self._evaluate(
            self._passing_pr(author=self.ONE_ACCOUNT, head="mergesha"),
            [self._receipt(self.ONE_ACCOUNT, head="mergesha", reviewer="codex-cli fresh exec session")],
            self.slice, self.config, self.MERGE_COMMIT_HEAD, head_parent_count=2)
        self.assertTrue(any("rebase" in e and "gh pr update-branch" in e for e in errors), errors)
        self.assertFalse(any("add an" in e and "Agent-Session:" in e for e in errors), errors)

    def test_merge_commit_head_carrying_its_own_trailer_still_resolves_identity(self):
        # `head_parent_count` only changes which error a trailerless head
        # gets. A merge commit that carries its own trailer resolves an
        # identity exactly as any other commit does; that path is unchanged
        # by this fix, and this locks it in.
        errors = self._evaluate(
            self._passing_pr(author=self.ONE_ACCOUNT),
            [self._receipt(self.ONE_ACCOUNT, reviewer="codex-cli fresh exec session")],
            self.slice, self.config, self.HEAD_COMMIT, head_parent_count=2)
        self.assertEqual([], errors)

    def test_merge_commit_head_with_no_receipt_yet_still_names_rebase(self):
        # The real sequence: `gh pr update-branch` produces a new head before
        # any receipt has been recorded against it, so the identity-mismatch
        # branch above never runs. The missing-receipt error must still name
        # rebase, or the operator burns a review round to find out.
        errors = self._evaluate(
            self._passing_pr(author=self.ONE_ACCOUNT, head="mergesha"),
            [], self.slice, self.config, self.MERGE_COMMIT_HEAD, head_parent_count=2)
        self.assertTrue(
            any("missing clean" in e and "rebase" in e and "gh pr update-branch" in e for e in errors), errors)

    def test_behind_merge_state_names_rebase_instead_of_update_branch(self):
        pr = dict(self._passing_pr(author=self.ONE_ACCOUNT), mergeStateStatus="BEHIND")
        errors = self._evaluate(pr, [], self.slice, self.config, self.MERGE_COMMIT_HEAD, head_parent_count=2)
        self.assertTrue(
            any("merge state is not ready (BEHIND)" in e and "rebase" in e and "gh pr update-branch" in e
                for e in errors),
            errors,
        )

    def test_different_account_receipt_needs_no_rebase_on_a_merge_commit_head(self):
        # Codex round 1 finding: the rebase advice is scoped to a *same-account*
        # receipt, exactly like the pre-existing trailer requirement it
        # replaces text for. A different-account receipt already establishes
        # independence without a trailer (`test_separate_accounts_need_no_trailer`),
        # and a merge-commit head changes nothing about that — this is
        # unrelated, unchanged, pre-existing behavior, not a new gap.
        errors = self._evaluate(
            self._passing_pr(author=self.ONE_ACCOUNT),
            [self._receipt("reviewer-bot", reviewer="codex-cli fresh exec session")],
            self.slice, self.config, self.MERGE_COMMIT_HEAD, head_parent_count=2)
        self.assertEqual([], errors)

    def test_fetch_commit_parent_count_reads_the_parents_list(self):
        with patch.object(sdlc, "run_json", return_value={"parents": [{"sha": "a"}, {"sha": "b"}]}) as reader:
            self.assertEqual(2, sdlc.fetch_commit_parent_count("deadbeef", self.config))
        reader.assert_called_once_with(
            ["gh", "api", f"repos/{self.config['repository']}/commits/deadbeef"])

    def test_fetch_commit_parent_count_defaults_missing_parents_to_zero(self):
        with patch.object(sdlc, "run_json", return_value={}):
            self.assertEqual(0, sdlc.fetch_commit_parent_count("deadbeef", self.config))

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

    def _merge_attempt(self, *, response=None, reads=None, apply=True, head=None, parent=None, prerequisites=()):
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
            self._open_parent() if parent is None else parent, prerequisites,
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
            self.slice, self.HEAD_COMMIT, self._open_parent(), ())
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
        ready = [e for e in sdlc.validate_slice(self.slice, issues, self.config, lambda n: records, self.NO_BLOCKERS)]
        merge = sdlc.evaluate_merge_gate(
            self._passing_pr(), [self._receipt("fresh-codex")], self.slice, self.config,
            parent=sdlc.ParentGate(10, self.epic, records), prerequisites=(),
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
        errors = sdlc.validate_slice(self.slice, {10: self.epic, 11: self.slice}, self.config, self.READ_OK, self.NO_BLOCKERS)
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

    def test_unreadable_gate_records_block_even_without_a_gate_label(self):
        # Round-1 finding 3: labels alone must not decide when the record half
        # of the gate is unknown.
        for gate in ([], [{"name": "gate:approved"}]):
            with self.subTest(gate=gate):
                self.setUp()
                self.epic["labels"] = gate
                ready, merge = self._admission(None)
                self.assertTrue(any("could not be read" in e for e in ready), ready)
                self.assertEqual(ready, merge)

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
        self.assertTrue(any("complete schema-1 changes-requested" in e for e in ready), ready)

    def test_incomplete_decision_record_cannot_authorize_feedback(self):
        # Round-3 finding 8: a hand-written schema-1 record lacking the
        # checkpoint or the deciding human is not a recorded decision.
        for payload in (
            {"schema": 1, "state": "changes-requested"},
            {"schema": 1, "state": "changes-requested", "checkpoint": self.CHECKPOINT},
            {"schema": 1, "state": "changes-requested", "decided_by": "Jerome"},
            {"schema": 1, "state": "changes-requested", "checkpoint": " ", "decided_by": "Jerome"},
            {"schema": 1, "state": "changes-requested", "checkpoint": self.CHECKPOINT, "decided_by": 7},
        ):
            with self.subTest(payload=payload):
                self.setUp()
                self.epic["labels"] = [{"name": "gate:changes-requested"}]
                self._as_feedback(self._decision_url(510))
                records = sdlc.parse_gate_records([
                    self._gate_comment("changes-requested", 500, "2026-09-03T00:00:00Z"),
                    {"id": 510, "created_at": "2026-09-04T00:00:00Z",
                     "body": f"{sdlc.GATE_PREFIX}{json.dumps(payload)}{sdlc.GATE_SUFFIX}"},
                ])
                ready, merge = self._admission(records)
                self.assertTrue(any("not a complete schema-1" in e for e in ready), ready)
                self.assertEqual(ready, merge)

    def test_quoted_marker_is_not_a_newer_gate_record(self):
        comments = [self._gate_comment("changes-requested", 500, "2026-09-03T00:00:00Z")]
        quoted = sdlc.gate_marker("approved", self.CHECKPOINT, "forged")
        comments += [
            {"id": 600, "created_at": "2026-09-04T00:00:00Z",
             "body": f"Continuation note. The record reads:\n```\n{quoted}\n```"},
            {"id": 601, "created_at": "2026-09-05T00:00:00Z", "body": f"> {quoted}"},
            # Round-1 finding 2: an indented Markdown code block.
            {"id": 602, "created_at": "2026-09-06T00:00:00Z", "body": f"    {quoted}\n"},
            {"id": 603, "created_at": "2026-09-07T00:00:00Z", "body": f"\n{quoted}\n"},
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
            # The issue names #10 but the caller resolved no parent for it.
            (sdlc.ParentGate(None, None, None), "parent epic #10 was not loaded"),
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

    def test_offline_check_slice_uses_bundled_gate_records(self):
        # Round-2 finding 6: an offline bundle can still pass when complete.
        import argparse
        import contextlib
        import io
        for bundle, expected in (
            ({"gate_comments": {"10": []}, "dependencies": {"11": []}}, 0),
            ({"dependencies": {"11": []}}, 1),
            ({"gate_comments": {"38": []}, "dependencies": {"11": []}}, 1),
            # Complete gate records but no dependency data: unknown, so blocked.
            ({"gate_comments": {"10": []}}, 1),
        ):
            with self.subTest(bundle=bundle):
                with tempfile.TemporaryDirectory() as tmp:
                    path = Path(tmp) / "bundle.json"
                    path.write_text(json.dumps(dict(bundle, issues=[self.epic, self.slice], pulls=[])))
                    out = io.StringIO()
                    with contextlib.redirect_stdout(out):
                        code = sdlc.command_check_slice(argparse.Namespace(issue=11, input=path), self.config)
                self.assertEqual(expected, code, out.getvalue())

    def test_real_merge_inputs_reload_a_pause_recorded_after_claim(self):
        # Round-2 finding 7: drive the real `load_gate_inputs` wiring, patching
        # only GitHub reads, so removing the parent reload turns this red.
        import argparse
        import contextlib
        import io
        pr = dict(self._passing_pr(head=self.CANDIDATE), number=99, headRefName="issue/11")
        paused = dict(self.epic, labels=[{"name": "gate:human-review"}])
        issues = {10: paused, 11: self.slice}
        comments = {99: [self._receipt("fresh-codex", head=self.CANDIDATE)], 10: []}
        writes = []
        out = io.StringIO()
        with patch.object(sdlc, "fetch_pr", return_value=pr), \
                patch.object(sdlc, "fetch_issue", side_effect=lambda n, c: issues[n]), \
                patch.object(sdlc, "fetch_comments", side_effect=lambda n, c: comments[n]), \
                patch.object(sdlc, "fetch_commit_message", return_value=self.HEAD_COMMIT), \
                patch.object(sdlc, "fetch_commit_parent_count", return_value=1), \
                patch.object(sdlc, "run_text", side_effect=lambda command, **k: writes.append(command) or ""), \
                contextlib.redirect_stdout(out):
            code = sdlc.command_merge(argparse.Namespace(pr=99, apply=True), self.config)
        self.assertEqual(1, code)
        self.assertEqual([], writes)
        self.assertIn("paused for human review", out.getvalue())
        # And the same wiring admits it once the epic is ungated.
        issues[10] = self.epic
        with patch.object(sdlc, "fetch_pr", return_value=pr), \
                patch.object(sdlc, "fetch_issue", side_effect=lambda n, c: issues[n]), \
                patch.object(sdlc, "fetch_comments", side_effect=lambda n, c: comments[n]), \
                patch.object(sdlc, "fetch_commit_message", return_value=self.HEAD_COMMIT), \
                patch.object(sdlc, "fetch_commit_parent_count", return_value=1):
            inputs = sdlc.load_gate_inputs(99, self.config)
            self.assertEqual((), sdlc.merge_eligibility(99, self.config).errors)
        self.assertEqual(sdlc.ParentGate(10, self.epic, []), inputs.parent)
        self.assertEqual(1, inputs.head_parent_count)

    def test_real_merge_inputs_thread_head_parent_count_to_the_gate(self):
        # #75: a value that only `load_gate_inputs` fetches must actually
        # reach `evaluate_merge_gate`, not just live on `GateInputs` unused.
        # Hardcoding `head_parent_count=1` in `load_gate_inputs` turns this red.
        import argparse
        import contextlib
        import io
        pr = dict(
            self._passing_pr(head=self.CANDIDATE, author=self.ONE_ACCOUNT),
            number=99, headRefName="issue/11",
        )
        issues = {10: self.epic, 11: self.slice}
        comments = {
            99: [self._receipt(self.ONE_ACCOUNT, head=self.CANDIDATE, reviewer="codex-cli fresh exec session")],
            10: [],
        }
        writes = []
        out = io.StringIO()
        with patch.object(sdlc, "fetch_pr", return_value=pr), \
                patch.object(sdlc, "fetch_issue", side_effect=lambda n, c: issues[n]), \
                patch.object(sdlc, "fetch_comments", side_effect=lambda n, c: comments[n]), \
                patch.object(sdlc, "fetch_commit_message", return_value=self.MERGE_COMMIT_HEAD), \
                patch.object(sdlc, "fetch_commit_parent_count", return_value=2), \
                patch.object(sdlc, "run_text", side_effect=lambda command, **k: writes.append(command) or ""), \
                contextlib.redirect_stdout(out):
            code = sdlc.command_merge(argparse.Namespace(pr=99, apply=True), self.config)
        self.assertEqual(1, code)
        self.assertEqual([], writes, "a merge-commit head with no trailer must never reach `gh pr merge`")
        self.assertIn("rebase", out.getvalue())
        self.assertIn("gh pr update-branch", out.getvalue())
        self.assertNotIn("add an", out.getvalue())

    def test_pause_after_claim_blocks_the_merge(self):
        issues = {10: self.epic, 11: self.slice}
        self.assertEqual([], sdlc.validate_slice(self.slice, issues, self.config, lambda n: [], self.NO_BLOCKERS))
        # A checkpoint pause is recorded after the claim and before merge.
        paused = dict(self.epic, labels=[{"name": "gate:human-review"}])
        code, events, output = self._merge_attempt(parent=sdlc.ParentGate(10, paused, []))
        self.assertEqual(1, code)
        self.assertEqual([], events, "a paused epic must not reach gh pr merge")
        self.assertIn("paused for human review", output)

    # --- set-gate transitions ---

    def _set_gate(self, state, labels, *, decided_by="Jerome (human)", checkpoint=None,
                  apply=True, fail=None, existing=None):
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
            if existing is None:
                # The record `set-gate --state review` posted for this checkpoint.
                existing = [self._gate_comment("review", 800, "2026-09-09T00:00:00Z")]
            comments = patch.object(sdlc, "fetch_comments", return_value=existing) \
                if not isinstance(existing, Exception) else \
                patch.object(sdlc, "fetch_comments", side_effect=existing)
            with patch.object(sdlc, "fetch_issue", return_value=epic), comments, \
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

    def test_a_decision_must_answer_the_recorded_checkpoint(self):
        # Round-1 finding 1: the label alone does not say *what* is under review.
        for state in ("changes-requested", "approved"):
            for existing, expected in (
                ([], "no current schema-1 review record"),
                ([self._gate_comment("changes-requested", 700, "2026-09-08T00:00:00Z")], "no current schema-1 review record"),
                ([{"id": 7, "created_at": "2026-09-01T00:00:00Z",
                   "body": f"{sdlc.GATE_PREFIX}state=review{sdlc.GATE_SUFFIX}"}], "no current schema-1 review record"),
                ([{"id": 8, "created_at": "2026-09-01T00:00:00Z",
                   "body": sdlc.GATE_PREFIX + '{"schema":1,"state":"review"}' + sdlc.GATE_SUFFIX}],
                 "no current schema-1 review record"),
                (sdlc.SdlcError("HTTP 502"), "could not be read"),
            ):
                with self.subTest(state=state, existing=existing):
                    code, writes, _, _, _ = self._set_gate(state, ["gate:human-review"], existing=existing)
                    self.assertIsInstance(code, sdlc.SdlcError)
                    self.assertIn(expected, str(code))
                    self.assertEqual([], writes)
            with self.subTest(state=state, checkpoint="different"):
                code, writes, _, _, _ = self._set_gate(
                    state, ["gate:human-review"], checkpoint="docs/milestones/epic-10/other.html")
                self.assertIsInstance(code, sdlc.SdlcError)
                self.assertIn("does not match the checkpoint under review", str(code))
                self.assertEqual([], writes)

    def test_next_names_a_gate_conflict_it_skips(self):
        import contextlib
        import io
        self.epic["labels"] = [{"name": "gate:human-review"}, {"name": "gate:approved"}]
        err = io.StringIO()
        with patch.object(sdlc, "fetch_status_data", return_value={"issues": [self.epic, self.slice], "pulls": []}), \
                patch.object(sdlc, "fetch_comments", return_value=[]), \
                contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(err):
            code = sdlc.command_next(None, self.config)
        self.assertEqual(1, code)
        self.assertIn("SKIPPED: issue #11", err.getvalue())
        self.assertIn("set-gate --epic 10", err.getvalue())

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

    def test_failed_first_write_never_loosens_the_prior_state(self):
        # Round-2 finding 5: a failed `review` cannot pause, but must fail
        # loudly and leave exactly the prior state, never a looser one.
        for before in ([], ["gate:approved"]):
            with self.subTest(before=before):
                code, writes, labels, records, _ = self._set_gate("review", before, fail="add")
                self.assertIsInstance(code, sdlc.SdlcError)
                self.assertEqual(set(before), labels)
                self.assertEqual([], records)
                self.assertEqual(["labels"], [write[0] for write in writes])

    def test_label_writes_add_before_they_remove(self):
        _, writes, _, _, _ = self._set_gate("changes-requested", ["gate:human-review"])
        commands = [write[1] for write in writes if write[0] == "labels"]
        self.assertEqual(2, len(commands))
        self.assertIn("--add-label", commands[0])
        self.assertNotIn("--remove-label", commands[0])
        self.assertIn("--remove-label", commands[1])
        self.assertNotIn("--add-label", commands[1])

    # Issue states for the admission matrix, each a mutation of the ready slice.
    def _set_labels(self, *names):
        self.slice["labels"] = [{"name": name} for name in names]

    ADMISSION_STATES = {
        "ready": lambda self: None,
        "in-progress": lambda self: self._set_labels("type:slice", "in-progress"),
        "in-review": lambda self: self._set_labels("type:slice", "in-review"),
        "ready feedback": lambda self: self._set_labels("type:feedback", "status:ready"),
        "ready+blocked": lambda self: self._set_labels("type:slice", "status:ready", "status:blocked"),
        "blocked in-progress": lambda self: self._set_labels("type:slice", "in-progress", "status:blocked"),
        "needs-slicing": lambda self: self._set_labels("type:slice", "status:ready", "needs-slicing"),
        "closed": lambda self: self.slice.update(state="CLOSED"),
        "epic": lambda self: self._set_labels("type:slice", "status:ready", "type:epic"),
        "not a slice": lambda self: self._set_labels("status:ready"),
        "missing section": lambda self: self.slice.update(
            body=self.slice["body"].replace("## Non-goals\nNo redesign.\n", "")),
        "no parent": lambda self: self.slice.update(
            body=self.slice["body"].replace("Parent epic: #10\n", "")),
        "two parents": lambda self: self.slice.update(body=self.slice["body"] + "Part of #12\n"),
        "parent paused": lambda self: self.epic.update(labels=[{"name": "gate:human-review"}]),
        # Native prerequisites (R3b): only a verified completion admits.
        "prereq satisfied": lambda self: (
            self._blocked_by(5), self._prerequisite(5, prs=[7]), self._pull(7)),
        "prereq open": lambda self: (self._blocked_by(5), self._prerequisite(5, state="OPEN", reason=None)),
        "prereq not planned": lambda self: (self._blocked_by(5), self._prerequisite(5, reason="NOT_PLANNED")),
        "prereq without merged PR": lambda self: (
            self._blocked_by(5), self._prerequisite(5, prs=[7]), self._pull(7, state="CLOSED", oid=None)),
        "prereqs unreadable": lambda self: self.graph["blocked_by"].update(
            {11: sdlc.SdlcError("HTTP 403: Resource not accessible")}),
    }
    # Which states each mode admits; every other state must be refused.
    ADMITTED = {
        "check": {"ready", "in-progress", "in-review", "ready feedback", "prereq satisfied"},
        "start": {"ready", "ready feedback", "prereq satisfied"},
        "continue": {"ready", "in-progress", "in-review", "ready feedback", "missing section", "prereq satisfied"},
    }

    class _Admitted(Exception):
        """Raised by a stubbed side effect once a command has passed admission."""

    def _command_admits(self, command):
        """Run one real command against the fixture; True when it admits #11."""
        import argparse
        import contextlib
        import io
        issues = {10: self.epic, 11: self.slice}
        if command == "merge":
            parent = sdlc.load_parent_from(self.slice, issues, self.config, self.READ_OK)
            return self._evaluate(
                self._passing_pr(), [self._receipt("fresh-codex")], self.slice, self.config, parent=parent,
                prerequisites=self._prerequisite_errors(),
            ) == []
        graph = self._graph_bundle()
        data = {
            "issues": [self.epic, self.slice, *graph.pop("prerequisites")], "pulls": [],
            "gate_comments": {"10": []}, **graph,
        }
        out = io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(io.StringIO()):
            if command == "check-slice":
                with tempfile.TemporaryDirectory() as tmp:
                    bundle = Path(tmp) / "bundle.json"
                    bundle.write_text(json.dumps(data), encoding="utf-8")
                    return sdlc.command_check_slice(argparse.Namespace(issue=11, input=bundle), self.config) == 0
            with patch.object(sdlc, "fetch_status_data", return_value=data), \
                    patch.object(sdlc, "fetch_comments", return_value=[]):
                if command == "next":
                    sdlc.command_next(None, self.config)
                    return "NEXT=11" in out.getvalue()
                # Claim probes the remote claim ref once, before admission: a
                # resume continues only if it exists. Any git effect after that
                # probe means the issue was admitted.
                resume = command.startswith("claim --resume")
                probes = [0 if command == "claim --resume" else 2]

                def run_process(args, **kwargs):
                    if probes:
                        return subprocess.CompletedProcess(args, probes.pop(), "", "")
                    raise self._Admitted

                with patch.object(sdlc, "run_process", side_effect=run_process), \
                        patch.object(sdlc, "run_text", side_effect=self._Admitted):
                    try:
                        code = sdlc.command_claim(argparse.Namespace(issue=11, resume=resume), self.config)
                    except self._Admitted:
                        return True
                    self.assertEqual(1, code)
                    self.assertIn("BLOCKED:", out.getvalue())
                    return False

    def test_admission_matrix_is_consistent_across_commands(self):
        commands = {
            "check-slice": "check", "next": "start", "claim": "start",
            "claim --resume": "continue", "merge": "continue",
        }
        for state, prepare in self.ADMISSION_STATES.items():
            for command, mode in commands.items():
                with self.subTest(state=state, command=command):
                    self.setUp()
                    prepare(self)
                    expected = state in self.ADMITTED[mode]
                    self.assertEqual(expected, self._command_admits(command))
                    # The command's verdict is the shared predicate's verdict.
                    errors = sdlc.validate_slice(
                        self.slice, {10: self.epic, 11: self.slice}, self.config, self.READ_OK,
                        self._graph_source(), mode=mode)
                    self.assertEqual(expected, errors == [], errors)

    def test_resume_without_an_existing_claim_starts_new_work(self):
        # `--resume` must not be a way to start unready work under the looser
        # continue rule: with no remote claim to resume, it is a start.
        for state in self.ADMISSION_STATES:
            with self.subTest(state=state):
                self.setUp()
                self.ADMISSION_STATES[state](self)
                self.assertEqual(
                    state in self.ADMITTED["start"], self._command_admits("claim --resume (unclaimed)"))

    def test_claim_fails_closed_when_the_claim_probe_fails(self):
        import argparse
        import contextlib
        import io
        data = {"issues": [self.epic, self.slice], "pulls": []}
        for resume in (False, True):
            with self.subTest(resume=resume):
                effects = []

                def run_process(args, **kwargs):
                    effects.append(args)
                    return subprocess.CompletedProcess(args, 128, "", "fatal: could not read from remote")

                with patch.object(sdlc, "fetch_status_data", return_value=data), \
                        patch.object(sdlc, "fetch_comments", return_value=[]), \
                        patch.object(sdlc, "run_process", side_effect=run_process), \
                        patch.object(sdlc, "run_text", side_effect=self._Admitted), \
                        patch.object(sdlc, "create_remote_ref", side_effect=self._Admitted), \
                        contextlib.redirect_stdout(io.StringIO()):
                    with self.assertRaisesRegex(sdlc.SdlcError, "could not read from remote"):
                        sdlc.command_claim(argparse.Namespace(issue=11, resume=resume), self.config)
                self.assertEqual(1, len(effects), "a failed probe must not be retried or acted on")

    def test_resume_acts_on_the_probe_it_was_admitted_by(self):
        # Admitted in continue mode because the claim existed, a resume must
        # never then create a claim, even if the branch vanished meanwhile.
        import argparse
        import contextlib
        import io
        self._set_labels("type:slice", "in-progress")
        data = {"issues": [self.epic, self.slice], "pulls": []}
        calls = []

        def run_process(args, **kwargs):
            calls.append(args)
            # The first probe sees the claim; any later probe would not.
            return subprocess.CompletedProcess(args, 0 if len(calls) == 1 else 2, "", "")

        with patch.object(sdlc, "fetch_status_data", return_value=data), \
                patch.object(sdlc, "fetch_comments", return_value=[]), \
                patch.object(sdlc, "run_process", side_effect=run_process), \
                patch.object(sdlc, "run_text", return_value=""), \
                patch.object(sdlc, "create_remote_ref", side_effect=AssertionError("resume created a claim")), \
                patch.object(sdlc.Path, "exists", return_value=True), \
                contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(0, sdlc.command_claim(argparse.Namespace(issue=11, resume=True), self.config))
        probes = [call for call in calls if call[:2] == ["git", "ls-remote"]]
        self.assertEqual(1, len(probes))

    def test_next_says_why_it_skips_any_ready_slice(self):
        import contextlib
        import io
        self._set_labels("type:slice", "status:ready", "needs-slicing")
        err = io.StringIO()
        with patch.object(sdlc, "fetch_status_data", return_value={"issues": [self.epic, self.slice], "pulls": []}), \
                patch.object(sdlc, "fetch_comments", return_value=[]), \
                contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(err):
            self.assertEqual(1, sdlc.command_next(None, self.config))
        self.assertIn("SKIPPED: issue #11", err.getvalue())
        self.assertIn("`needs-slicing`", err.getvalue())

    def test_next_says_why_it_skips_a_ready_issue_that_is_not_a_slice(self):
        import contextlib
        import io
        self._set_labels("status:ready")
        err = io.StringIO()
        with patch.object(sdlc, "fetch_status_data", return_value={"issues": [self.epic, self.slice], "pulls": []}), \
                patch.object(sdlc, "fetch_comments", return_value=[]), \
                contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(err):
            self.assertEqual(1, sdlc.command_next(None, self.config))
        self.assertIn("SKIPPED: issue #11: missing `type:slice` or `type:feedback` label", err.getvalue())

    def test_blocked_slice_is_refused_with_its_cause(self):
        self._set_labels("type:slice", "status:ready", "status:blocked")
        for mode in sdlc.ADMISSION_MODES:
            with self.subTest(mode=mode):
                errors = sdlc.validate_slice(
                    self.slice, {10: self.epic, 11: self.slice}, self.config, self.READ_OK, self.NO_BLOCKERS, mode=mode)
                self.assertTrue(any("`status:blocked`" in error for error in errors), errors)

    def test_closed_or_blocked_linked_issue_blocks_merge(self):
        for state in ("closed", "ready+blocked", "blocked in-progress", "needs-slicing"):
            with self.subTest(state=state):
                self.setUp()
                self.ADMISSION_STATES[state](self)
                self.assertFalse(self._command_admits("merge"))

    def test_next_names_a_ready_slice_it_skips_as_blocked(self):
        import contextlib
        import io
        self._set_labels("type:slice", "status:ready", "status:blocked")
        err = io.StringIO()
        with patch.object(sdlc, "fetch_status_data", return_value={"issues": [self.epic, self.slice], "pulls": []}), \
                patch.object(sdlc, "fetch_comments", return_value=[]), \
                contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(err):
            self.assertEqual(1, sdlc.command_next(None, self.config))
        self.assertIn("SKIPPED: issue #11", err.getvalue())
        self.assertIn("`status:blocked`", err.getvalue())

    # --- native prerequisites (R3b) ---

    def test_prerequisite_completion_matrix(self):
        cases = {
            "satisfied": (lambda: (self._prerequisite(5, prs=[7]), self._pull(7)), None),
            "open": (lambda: self._prerequisite(5, state="OPEN", reason=None), "blocked by #5, which is still open"),
            "reopened": (lambda: self._prerequisite(5, state="OPEN", reason="REOPENED", prs=[7]),
                         "blocked by #5, which is still open"),
            "not planned": (lambda: self._prerequisite(5, reason="NOT_PLANNED", prs=[7]), "closed as NOT_PLANNED"),
            "duplicate": (lambda: self._prerequisite(5, reason="DUPLICATE"), "closed as DUPLICATE"),
            "hand-closed, no PR": (lambda: self._prerequisite(5),
                                   "no closing pull request is verified merged into `main`"),
            "closing PR unmerged": (lambda: (self._prerequisite(5, prs=[7]), self._pull(7, state="OPEN", oid=None)),
                                    "PR #7 is OPEN into `main`"),
            "closing PR closed unmerged": (
                lambda: (self._prerequisite(5, prs=[7]), self._pull(7, state="CLOSED", oid=None)),
                "PR #7 is CLOSED into `main`"),
            "merged into another base": (
                lambda: (self._prerequisite(5, prs=[7]), self._pull(7, base="release")),
                "PR #7 is MERGED into `release`"),
            "merged without a full SHA": (
                lambda: (self._prerequisite(5, prs=[7]), self._pull(7, oid="abc")),
                "PR #7 is not a complete record"),
            "closing PR unreadable": (
                lambda: (self._prerequisite(5, prs=[7]), self.graph["pulls"].update({7: sdlc.SdlcError("HTTP 502")})),
                "PR #7 could not be read (HTTP 502)"),
            # R3b-1: an unreadable reference is unknown even beside a merged
            # one, in either order, live or bundled.
            "unreadable PR beside a merged one": (
                lambda: (self._prerequisite(5, prs=[6, 7]), self._pull(7),
                         self.graph["pulls"].update({6: sdlc.SdlcError("HTTP 403")})),
                "its closing evidence could not be read (PR #6 could not be read (HTTP 403))"),
            "merged PR beside an unreadable one": (
                lambda: (self._prerequisite(5, prs=[7, 6]), self._pull(7)),
                "its closing evidence could not be read (PR #6 could not be read (HTTP 404"),
            # R3b-2/R3b-3: malformed references and incomplete PR records are
            # unknown, even beside a merged reference.
            "malformed reference beside a merged one": (
                lambda: (self._prerequisite(5, prs=[7]), self._pull(7),
                         self.graph["issues"][5]["closedByPullRequestsReferences"].extend(
                             [{"number": 8}, {"repository": {"name": "cubism-rs", "owner": {"login": "jeromebanks"}}},
                              "PR_kwDO", {"number": True, "repository": {"name": "cubism-rs", "owner": {"login": "jeromebanks"}}}])),
                "a closing reference is malformed"),
            "incomplete PR record beside a merged one": (
                lambda: (self._prerequisite(5, prs=[6, 7]), self._pull(7), self.graph["pulls"].update({6: {}})),
                "PR #6 is not a complete record"),
            "PR record without mergeCommit": (
                lambda: (self._prerequisite(5, prs=[6, 7]), self._pull(7),
                         self.graph["pulls"].update({6: {"state": "MERGED", "baseRefName": "main"}})),
                "PR #6 is not a complete record"),
            # R3b-4: every PR record inconsistent with its state is unknown,
            # beside a merged reference, rather than read as unmerged.
            **{
                f"inconsistent PR record {record!r} beside a merged one": (
                    lambda record=record: (self._prerequisite(5, prs=[6, 7]), self._pull(7),
                                           self.graph["pulls"].update({6: record})),
                    "PR #6 is not a complete record")
                for record in (
                    {"state": "MERGED", "baseRefName": "main", "mergeCommit": {}},
                    {"state": "MERGED", "baseRefName": "main", "mergeCommit": {"oid": None}},
                    {"state": "MERGED", "baseRefName": "main", "mergeCommit": None},
                    {"state": "MERGED", "baseRefName": "", "mergeCommit": {"oid": "e" * 40}},
                    {"state": "OPEN", "baseRefName": "main", "mergeCommit": {}},
                    {"state": "CLOSED", "baseRefName": "main", "mergeCommit": {"oid": "e" * 40}},
                    {"state": "merged", "baseRefName": "main", "mergeCommit": {"oid": "e" * 40}},
                    {"state": None, "baseRefName": "main", "mergeCommit": None},
                )
            },
            "closing references not a list": (
                lambda: self._prerequisite(5, prs=[7]) or self.graph["issues"][5].update(
                    closedByPullRequestsReferences={"number": 7}),
                "its closing pull requests could not be read"),
            "one of two PRs merged": (
                lambda: (self._prerequisite(5, prs=[6, 7]), self._pull(6, state="CLOSED", oid=None), self._pull(7)),
                None),
            "missing prerequisite": (lambda: None, "blocked by #5, which could not be read (HTTP 404"),
            "prerequisite forbidden": (
                lambda: self.graph["issues"].update({5: sdlc.SdlcError("HTTP 403: Resource not accessible")}),
                "blocked by #5, which could not be read (HTTP 403"),
        }
        for name, (prepare, expected) in cases.items():
            with self.subTest(case=name):
                self.setUp()
                self._blocked_by(5)
                prepare()
                errors = self._prerequisite_errors()
                if expected is None:
                    self.assertEqual([], errors)
                else:
                    self.assertTrue(any(expected in error for error in errors), errors)

    def test_verification_only_prerequisite_names_the_missing_evidence(self):
        self._blocked_by(5)
        self._prerequisite(5)
        [error] = self._prerequisite_errors()
        self.assertIn("closed COMPLETED, but no closing pull request is verified merged", error)
        self.assertIn("verification-only prerequisite needs explicit completion evidence", error)

    def test_closing_pr_in_another_repository_does_not_count(self):
        self._blocked_by(5)
        self._prerequisite(5)
        self.graph["issues"][5]["closedByPullRequestsReferences"] = [
            {"number": 7, "repository": {"name": "fork", "owner": {"login": "someone"}}}]
        self._pull(7)  # Same number here, but it is not the PR that closed #5.
        [error] = self._prerequisite_errors()
        self.assertIn("someone/fork#7 is outside jeromebanks/cubism-rs", error)

    def test_unreadable_closing_references_are_unknown(self):
        self._blocked_by(5)
        self._prerequisite(5, prs=[7])
        del self.graph["issues"][5]["closedByPullRequestsReferences"]
        [error] = self._prerequisite_errors()
        self.assertIn("closing pull requests could not be read", error)

    def test_every_blocker_must_be_complete(self):
        self._blocked_by(5, 6)
        self._prerequisite(5, prs=[7])
        self._pull(7)
        self._prerequisite(6, state="OPEN", reason=None)
        self.assertEqual(["blocked by #6, which is still open"], self._prerequisite_errors())

    def test_blocker_in_another_repository_fails_closed(self):
        self.graph["blocked_by"][11] = [self._blocker(5, "https://api.github.com/repos/other/repo")]
        [error] = self._prerequisite_errors()
        self.assertIn("#11 is blocked by", error)
        self.assertIn("outside jeromebanks/cubism-rs; its completion cannot be verified", error)

    def test_malformed_dependency_records_fail_closed(self):
        for name, prepare, expected in (
            ("blocked_by not a list", lambda: self.graph["blocked_by"].update({11: {"number": 5}}),
             "prerequisites of #11 are not a list"),
            ("blocker not a dict", lambda: self.graph["blocked_by"].update({11: ["#5"]}),
             "a prerequisite entry of #11 is malformed"),
            ("boolean blocker number", lambda: self.graph["blocked_by"].update(
                {11: [dict(self._blocker(5), number=True)]}), "a prerequisite entry of #11 is malformed"),
            ("prerequisite not a dict", lambda: (self._blocked_by(5), self.graph["issues"].update({5: ["CLOSED"]})),
             "blocked by #5, whose record is malformed"),
        ):
            with self.subTest(case=name):
                self.setUp()
                prepare()
                errors = self._prerequisite_errors()
                self.assertTrue(any(expected in e for e in errors), errors)

    def test_round_four_input_shapes_fail_closed(self):
        # R3b-5..R3b-9 (Codex round 4). Each would otherwise admit the slice.
        def satisfied():
            self._blocked_by(5)
            self._prerequisite(5, prs=[7])
            self._pull(7)
        for name, prepare, expected in (
            # R3b-6: records must be the ones asked for.
            ("issue answered for another number",
             lambda: self.graph["issues"][5].update(number=6), "blocked by #5, whose record is malformed"),
            ("PR answered for another number",
             lambda: self.graph["pulls"][7].update(number=8), "PR #7 is not a complete record"),
            ("PR record without a number",
             lambda: self.graph["pulls"][7].pop("number"), "PR #7 is not a complete record"),
            # R3b-7: only api.github.com names a repository.
            ("blocker on another host", lambda: self.graph["blocked_by"].update({11: [dict(
                self._blocker(5), repository_url="https://evil.example/repos/jeromebanks/cubism-rs")]}),
             "outside jeromebanks/cubism-rs"),
            ("blocker URL with a suffix", lambda: self.graph["blocked_by"].update({11: [dict(
                self._blocker(5), repository_url=self.REPO_API + "/issues")]}),
             "outside jeromebanks/cubism-rs"),
            # R3b-8: numbers are positive.
            ("blocker number 0", lambda: self.graph["blocked_by"].update({11: [self._blocker(0)]}),
             "a prerequisite entry of #11 is malformed"),
            ("negative blocker number", lambda: self.graph["blocked_by"].update({11: [self._blocker(-5)]}),
             "a prerequisite entry of #11 is malformed"),
            ("closing reference number 0",
             lambda: self.graph["issues"][5]["closedByPullRequestsReferences"].append(
                 {"number": 0, "repository": {"name": "cubism-rs", "owner": {"login": "jeromebanks"}}}),
             "a closing reference is malformed"),
        ):
            with self.subTest(case=name):
                self.setUp()
                satisfied()
                self.assertEqual([], self._prerequisite_errors(), "the unmodified graph admits")
                prepare()
                errors = self._prerequisite_errors()
                self.assertTrue(any(expected in e for e in errors), errors)

    def test_live_answers_must_be_complete_and_for_the_asked_issue(self):
        # R3b-6 and R3b-9 on the live GraphQL reader.
        answer = self._graphql_prerequisite()
        issue = answer["data"]["repository"]["issue"]
        for name, response in (
            ("another issue", {"data": {"repository": {"issue": dict(issue, number=6)}}}),
            ("boolean count", {"data": {"repository": {"issue": dict(issue, closedByPullRequestsReferences=dict(
                issue["closedByPullRequestsReferences"], totalCount=True))}}}),
            ("string count", {"data": {"repository": {"issue": dict(issue, closedByPullRequestsReferences=dict(
                issue["closedByPullRequestsReferences"], totalCount="1"))}}}),
            ("page info not an object", {"data": {"repository": {"issue": dict(
                issue, closedByPullRequestsReferences=dict(issue["closedByPullRequestsReferences"], pageInfo=[]))}}}),
        ):
            with self.subTest(case=name):
                with patch.object(sdlc, "fetch_prerequisite", new=REAL_READERS["fetch_prerequisite"]), \
                        patch.object(sdlc, "run_json", return_value=response):
                    error = sdlc.completion_error(5, sdlc.live_dependencies(self.config), self.config)
                self.assertIn("blocked by #5, which could not be read", error)

    def test_conflicting_bundle_records_are_unknown(self):
        # R3b-5: two records for one prerequisite; neither is known current.
        closed = {"number": 5, "state": "CLOSED", "stateReason": "COMPLETED", "closedByPullRequestsReferences": []}
        source = sdlc.bundle_dependencies({"issues": [closed, dict(closed, state="OPEN", stateReason=None)]})
        with self.assertRaisesRegex(sdlc.SdlcError, "2 records for #5"):
            source.issue(5)

    def test_cycles_are_reported_with_their_path(self):
        # 11 <- 5 <- 6 <- 5: the cycle is upstream of the slice.
        self._blocked_by(5)
        self._prerequisite(5, prs=[7], blocked_by=[6])
        self._prerequisite(6, prs=[8], blocked_by=[5])
        self._pull(7)
        self._pull(8)
        self.assertEqual(
            ["dependency cycle: #5 → #6 → #5 (each is blocked by the next); remove one link"],
            self._prerequisite_errors())
        # A cycle through the slice itself names it.
        self.setUp()
        self._blocked_by(5)
        self._prerequisite(5, state="OPEN", reason=None, blocked_by=[11])
        errors = self._prerequisite_errors()
        self.assertIn("dependency cycle: #5 → #11 → #5 (each is blocked by the next); remove one link", errors)
        # A self-loop is a cycle too.
        self.setUp()
        self._blocked_by(11)
        self.assertTrue(any(e.startswith("dependency cycle: #11 → #11") for e in self._prerequisite_errors()))

    def test_a_diamond_is_not_a_cycle(self):
        self._blocked_by(5, 6)
        self._prerequisite(5, prs=[7], blocked_by=[4])
        self._prerequisite(6, prs=[7], blocked_by=[4])
        self._prerequisite(4, prs=[7])
        self._pull(7)
        self.assertEqual([], self._prerequisite_errors())

    def test_unreadable_upstream_links_block(self):
        # Only #11's direct prerequisite need be complete, but the graph above
        # it must be readable, or a cycle through it could hide.
        self._blocked_by(5)
        self._prerequisite(5, prs=[7])
        self._pull(7)
        self.graph["blocked_by"][5] = sdlc.SdlcError("HTTP 502 on page 2")
        [error] = self._prerequisite_errors()
        self.assertIn("prerequisites of #5 could not be read completely (HTTP 502 on page 2)", error)

    def test_no_dependency_source_blocks(self):
        errors = sdlc.validate_slice(self.slice, {10: self.epic, 11: self.slice}, self.config, self.READ_OK)
        self.assertTrue(any("prerequisites of #11 could not be read completely" in e for e in errors), errors)

    def test_admission_requires_prerequisites_explicitly(self):
        with self.assertRaises(TypeError):
            sdlc.admission_errors(self.slice, self._open_parent(), self.config, mode="check")
        with self.assertRaises(TypeError):
            sdlc.evaluate_merge_gate(
                self._passing_pr(), [self._receipt("fresh-codex")], self.slice, self.config,
                parent=self._open_parent())

    # The literal `gh api --paginate --slurp` output for codexsit#111 at
    # per_page=1, trimmed to the fields admission reads: one list per page.
    SLURPED_PAGES = [
        [{"number": 110, "state": "open", "state_reason": None,
          "repository_url": "https://api.github.com/repos/codexsit/codexsit.github.io"}],
        [{"number": 114, "state": "open", "state_reason": None,
          "repository_url": "https://api.github.com/repos/codexsit/codexsit.github.io"}],
    ]

    def test_blocked_by_reads_every_page(self):
        calls = []
        real = REAL_READERS["fetch_blocked_by"]
        with patch.object(sdlc, "run_json", side_effect=lambda c: calls.append(c) or self.SLURPED_PAGES):
            blockers = real(111, self.config)
        self.assertEqual([110, 114], [b["number"] for b in blockers])
        self.assertIn("--paginate", calls[-1])
        self.assertIn("--slurp", calls[-1])
        self.assertTrue(calls[-1][-1].endswith("/issues/111/dependencies/blocked_by?per_page=100"))
        with patch.object(sdlc, "run_json", return_value=[[]]):
            self.assertEqual([], real(98, self.config))

    def test_malformed_or_failed_pages_fail_closed(self):
        real = REAL_READERS["fetch_blocked_by"]
        for response in (
            sdlc.SdlcError("gh api: HTTP 502 (page 2)"),
            sdlc.SdlcError("gh: Not Found (HTTP 404)"),
            {"message": "Not Found"},
            [[{"number": 110}], {"message": "rate limited"}],
            [[{"number": 110}], ["not an issue"]],
            # Shapes that would flatten to "no prerequisites" and fail open.
            {},
            [{}],
            [],
        ):
            with self.subTest(response=response):
                kwargs = {"side_effect": response} if isinstance(response, Exception) else {"return_value": response}
                with patch.object(sdlc, "run_json", **kwargs), \
                        patch.object(sdlc, "fetch_blocked_by", side_effect=real):
                    errors = sdlc.prerequisite_errors(11, sdlc.live_dependencies(self.config), self.config)
                self.assertEqual(1, len(errors), errors)
                self.assertIn("prerequisites of #11 could not be read completely", errors[0])

    # The live GraphQL answer for #97 (recorded 2026-09-24), renumbered.
    @staticmethod
    def _graphql_prerequisite(nodes=None, total=None, has_next=False, state="CLOSED"):
        nodes = [{"number": 7, "repository": {"name": "cubism-rs", "owner": {"login": "jeromebanks"}}}] \
            if nodes is None else nodes
        return {"data": {"repository": {"issue": {
            "number": 5, "state": state, "stateReason": "COMPLETED", "url": "https://example.test/5",
            "closedByPullRequestsReferences": {
                "totalCount": len(nodes) if total is None else total,
                "pageInfo": {"hasNextPage": has_next}, "nodes": nodes,
            },
        }}}}

    def test_live_prerequisite_reads_use_the_probed_fields(self):
        commands = []
        responses = iter([
            self._graphql_prerequisite(),
            {"number": 7, "state": "MERGED", "baseRefName": "main", "mergeCommit": {"oid": "e" * 40}},
        ])
        with patch.object(sdlc, "fetch_prerequisite", new=REAL_READERS["fetch_prerequisite"]), \
                patch.object(sdlc, "fetch_pull_merge", new=REAL_READERS["fetch_pull_merge"]), \
                patch.object(sdlc, "run_json", side_effect=lambda c: commands.append(c) or next(responses)):
            self.assertIsNone(sdlc.completion_error(5, sdlc.live_dependencies(self.config), self.config))
        self.assertEqual(["gh", "api", "graphql"], commands[0][:3])
        query = commands[0][4]
        for field in ("stateReason", "closedByPullRequestsReferences", "totalCount", "hasNextPage"):
            self.assertIn(field, query)
        self.assertIn("number=5", commands[0])
        self.assertEqual(["gh", "pr", "view", "7"], commands[1][:4])
        self.assertIn("mergeCommit", commands[1][-1])

    def test_a_closing_reference_list_not_shown_complete_is_unknown(self):
        # `gh issue view` asks for only the first 100 closing references and
        # never pages. A list that may be cut hides unread references, and every
        # reference must be read.
        for name, response in (
            ("more pages", self._graphql_prerequisite(has_next=True)),
            ("count exceeds nodes", self._graphql_prerequisite(total=101)),
            ("no page info", {"data": {"repository": {"issue": dict(
                self._graphql_prerequisite()["data"]["repository"]["issue"],
                closedByPullRequestsReferences={"totalCount": 1, "nodes": []})}}}),
            ("no issue", {"data": {"repository": {"issue": None}}}),
            ("graphql error", {"errors": [{"message": "Something went wrong"}]}),
        ):
            with self.subTest(case=name):
                with patch.object(sdlc, "fetch_prerequisite", new=REAL_READERS["fetch_prerequisite"]), \
                        patch.object(sdlc, "run_json", return_value=response):
                    error = sdlc.completion_error(5, sdlc.live_dependencies(self.config), self.config)
                self.assertIn("blocked by #5, which could not be read", error)

    def test_a_prerequisite_state_must_be_known(self):
        for state in (None, "closed", "MERGED", ""):
            with self.subTest(state=state):
                self.setUp()
                self._blocked_by(5)
                self._prerequisite(5, prs=[7])
                self._pull(7)
                self.graph["issues"][5]["state"] = state
                self.assertEqual(
                    ["blocked by #5, whose record is malformed; its completion is unknown"],
                    self._prerequisite_errors())

    def test_offline_bundle_must_supply_dependency_data(self):
        import argparse
        import contextlib
        import io

        def check(bundle):
            with tempfile.TemporaryDirectory() as tmp:
                path = Path(tmp) / "bundle.json"
                path.write_text(json.dumps(bundle))
                out = io.StringIO()
                with contextlib.redirect_stdout(out):
                    code = sdlc.command_check_slice(argparse.Namespace(issue=11, input=path), self.config)
            return code, out.getvalue()

        prerequisite = {
            "number": 5, "state": "CLOSED", "stateReason": "COMPLETED",
            "closedByPullRequestsReferences": [
                {"number": 7, "repository": {"name": "cubism-rs", "owner": {"login": "jeromebanks"}}}],
        }
        merged = {"number": 7, "state": "MERGED", "baseRefName": "main", "mergeCommit": {"oid": "e" * 40}}
        base = {"issues": [self.epic, self.slice, prerequisite], "pulls": [], "gate_comments": {"10": []},
                "dependencies": {"11": [self._blocker(5)], "5": []}}
        self.assertEqual(0, check(dict(base, pull_requests={"7": merged}))[0])
        # The demo: a prerequisite closed by hand does not count.
        code, output = check(dict(base, pull_requests={"7": dict(merged, state="CLOSED", mergeCommit=None)}))
        self.assertEqual(1, code)
        self.assertIn("BLOCKED: blocked by #5: closed COMPLETED, but no closing pull request is verified merged", output)
        code, output = check(dict(base))
        self.assertEqual(1, code)
        self.assertIn("no `pull_requests` entry for pull request #7", output)
        code, output = check(dict(base, dependencies={"11": [self._blocker(5)]}, pull_requests={"7": merged}))
        self.assertEqual(1, code)
        self.assertIn("no `dependencies` entry for issue #5", output)
        # R3b-1: a closing PR the bundle omits is unknown beside a merged one.
        two_refs = dict(prerequisite, closedByPullRequestsReferences=[
            {"number": n, "repository": {"name": "cubism-rs", "owner": {"login": "jeromebanks"}}} for n in (6, 7)])
        code, output = check(dict(base, issues=[self.epic, self.slice, two_refs], pull_requests={"7": merged}))
        self.assertEqual(1, code)
        self.assertIn("no `pull_requests` entry for pull request #6", output)
        # R3b-3: an incomplete bundled PR record is unknown beside a merged one.
        code, output = check(dict(base, issues=[self.epic, self.slice, two_refs], pull_requests={"6": {}, "7": merged}))
        self.assertEqual(1, code)
        self.assertIn("PR #6 is not a complete record", output)
        # R3b-4: `mergeCommit: {}` is not evidence of either outcome.
        code, output = check(dict(base, issues=[self.epic, self.slice, two_refs], pull_requests={
            "6": dict(merged, number=6, mergeCommit={}), "7": merged}))
        self.assertEqual(1, code)
        self.assertIn("PR #6 is not a complete record", output)
        # R3b-2: a malformed bundled reference is unknown beside a merged one.
        malformed = dict(prerequisite, closedByPullRequestsReferences=[
            *prerequisite["closedByPullRequestsReferences"], {"number": 8}])
        code, output = check(dict(base, issues=[self.epic, self.slice, malformed], pull_requests={"7": merged}))
        self.assertEqual(1, code)
        self.assertIn("a closing reference is malformed", output)
        without_reason = {k: v for k, v in prerequisite.items() if k != "stateReason"}
        code, output = check(dict(base, issues=[self.epic, self.slice, without_reason], pull_requests={"7": merged}))
        self.assertEqual(1, code)
        self.assertIn("no prerequisite record with `stateReason` for #5", output)

    def test_prerequisite_reopened_after_claim_blocks_the_real_merge(self):
        # Drive the real `load_gate_inputs` wiring, patching only GitHub reads.
        import argparse
        import contextlib
        import io
        pr = dict(self._passing_pr(head=self.CANDIDATE), number=99, headRefName="issue/11")
        issues = {10: self.epic, 11: self.slice}
        comments = {99: [self._receipt("fresh-codex", head=self.CANDIDATE)], 10: []}
        self._blocked_by(5)
        self._prerequisite(5, prs=[7])
        self._pull(7)
        self.assertEqual([], self._prerequisite_errors(), "admitted at claim time")
        # Reopened after the claim.
        self.graph["issues"][5].update(state="OPEN", stateReason="REOPENED")
        writes = []
        out = io.StringIO()
        with patch.object(sdlc, "fetch_pr", return_value=pr), \
                patch.object(sdlc, "fetch_issue", side_effect=lambda n, c: issues[n]), \
                patch.object(sdlc, "fetch_comments", side_effect=lambda n, c: comments[n]), \
                patch.object(sdlc, "fetch_commit_message", return_value=self.HEAD_COMMIT), \
                patch.object(sdlc, "fetch_commit_parent_count", return_value=1), \
                patch.object(sdlc, "run_text", side_effect=lambda command, **k: writes.append(command) or ""), \
                contextlib.redirect_stdout(out):
            code = sdlc.command_merge(argparse.Namespace(pr=99, apply=True), self.config)
        self.assertEqual(1, code)
        self.assertEqual([], writes)
        self.assertIn("BLOCKED: blocked by #5, which is still open", out.getvalue())
        # And the same wiring admits it once #5 is complete again.
        self.graph["issues"][5].update(state="CLOSED", stateReason="COMPLETED")
        with patch.object(sdlc, "fetch_pr", return_value=pr), \
                patch.object(sdlc, "fetch_issue", side_effect=lambda n, c: issues[n]), \
                patch.object(sdlc, "fetch_comments", side_effect=lambda n, c: comments[n]), \
                patch.object(sdlc, "fetch_commit_message", return_value=self.HEAD_COMMIT), \
                patch.object(sdlc, "fetch_commit_parent_count", return_value=1):
            self.assertEqual((), sdlc.merge_eligibility(99, self.config).errors)

    def test_next_and_claim_explain_an_unmet_prerequisite(self):
        import argparse
        import contextlib
        import io
        self._blocked_by(5)
        self._prerequisite(5, reason="NOT_PLANNED")
        data = {"issues": [self.epic, self.slice], "pulls": []}
        err, out = io.StringIO(), io.StringIO()
        with patch.object(sdlc, "fetch_status_data", return_value=data), \
                patch.object(sdlc, "fetch_comments", return_value=[]), \
                contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            self.assertEqual(1, sdlc.command_next(None, self.config))
        self.assertIn("SKIPPED: issue #11: blocked by #5, closed as NOT_PLANNED", err.getvalue())
        out = io.StringIO()
        with patch.object(sdlc, "fetch_status_data", return_value=data), \
                patch.object(sdlc, "fetch_comments", return_value=[]), \
                patch.object(sdlc, "run_process", return_value=subprocess.CompletedProcess([], 2, "", "")), \
                patch.object(sdlc, "run_text", side_effect=self._Admitted), \
                patch.object(sdlc, "create_remote_ref", side_effect=self._Admitted), \
                contextlib.redirect_stdout(out):
            self.assertEqual(1, sdlc.command_claim(argparse.Namespace(issue=11, resume=False), self.config))
        self.assertIn("BLOCKED: blocked by #5, closed as NOT_PLANNED", out.getvalue())

    def test_unknown_admission_mode_is_a_programming_error(self):
        with self.assertRaises(ValueError):
            sdlc.validate_slice(self.slice, {10: self.epic, 11: self.slice}, self.config, self.READ_OK, self.NO_BLOCKERS, mode="merge")

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

    def test_claim_names_the_remote_base_and_never_moves_local_main(self):
        # The default branch may be checked out in the primary checkout, so
        # claim only fetches it. The ref to diff and rebase against is the
        # remote-tracking one, and claim says so.
        import argparse
        import contextlib
        import io
        default = self.config["default_branch"]
        data = {"issues": [self.epic, self.slice], "pulls": []}
        git = []

        def run_process(args, **kwargs):
            git.append(args)
            # No remote claim yet; no local claim branch.
            return subprocess.CompletedProcess(args, 2 if args[1] == "ls-remote" else 1, "", "")

        def run_text(args):
            git.append(args)
            return "0" * 40 if args[1] == "rev-parse" else ""

        out = io.StringIO()
        with tempfile.TemporaryDirectory() as tmp, \
                patch.object(sdlc, "ROOT", Path(tmp)), \
                patch.object(sdlc, "fetch_status_data", return_value=data), \
                patch.object(sdlc, "fetch_comments", return_value=[]), \
                patch.object(sdlc, "run_process", side_effect=run_process), \
                patch.object(sdlc, "run_text", side_effect=run_text), \
                patch.object(sdlc, "create_remote_ref", return_value=True), \
                contextlib.redirect_stdout(out):
            self.assertEqual(0, sdlc.command_claim(argparse.Namespace(issue=11, resume=False), self.config))
        self.assertIn(f"BASE=origin/{default}\n", out.getvalue())
        self.assertIn(["git", "fetch", "origin", default], git)
        for args in git:
            if args[0] != "git":
                continue
            # An allowlist, not a denylist: claim runs only these read or
            # claim-branch commands, with no global option (`-C <dir>`, `-c`,
            # `--git-dir`) placed before them that could redirect one at the
            # primary checkout. `reset`, `branch`, `pull`, `update-ref` and
            # the rest are rejected by omission.
            self.assertIn(args[1], {"ls-remote", "fetch", "rev-parse", "show-ref", "worktree"}, args)
            if args[1] == "worktree":
                self.assertEqual("add", args[2], args)
            if args == ["git", "fetch", "origin", default]:
                continue
            # No other command names the local default branch in any form,
            # so none can move it (`fetch origin main:main`) or check it out
            # (`worktree add <path> main`).
            for token in args[1:]:
                self.assertNotEqual(default, token, args)
                self.assertFalse(token.endswith(f":{default}") or token.endswith(f"/heads/{default}"), args)

    def _cleanup(self, state="CLOSED", labels=("type:slice", "in-review"), worktree=False, local_branch=True,
                 merged=True):
        """Run `cleanup 11` with every effect recorded in order, none performed."""
        import argparse
        import contextlib
        import io
        issue = {"number": 11, "state": state, "labels": [{"name": name} for name in labels]}
        effects = []

        def run_text(args):
            effects.append(("run", args))
            return ""

        def run_process(args, **kwargs):
            effects.append(("run", args))
            if args[1] == "merge-base":
                return subprocess.CompletedProcess(args, 0 if merged else 1, "", "")
            return subprocess.CompletedProcess(args, 0 if local_branch else 1, "", "")

        def fetch_status_data(_config):
            effects.append(("status-read",))
            return {"repository": "example/repo", "generated_at": "now", "issues": [], "pulls": []}

        out = io.StringIO()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            if worktree:
                (root / ".worktrees" / "issue-11").mkdir(parents=True)
            with patch.object(sdlc, "ROOT", root), \
                    patch.object(sdlc, "fetch_issue", return_value=issue), \
                    patch.object(sdlc, "run_text", side_effect=run_text), \
                    patch.object(sdlc, "run_process", side_effect=run_process), \
                    patch.object(sdlc, "fetch_status_data", side_effect=fetch_status_data), \
                    patch.object(sdlc, "write_atomic", side_effect=lambda path, _c: effects.append(("write", path))), \
                    contextlib.redirect_stdout(out):
                code = sdlc.command_cleanup(argparse.Namespace(issue=11), self.config)
        return code, effects, out.getvalue()

    @staticmethod
    def _label_edits(effects):
        return [e[1] for e in effects if e[0] == "run" and e[1][:3] == ["gh", "issue", "edit"]]

    def test_cleanup_clears_delivery_labels_before_regenerating_status(self):
        code, effects, out = self._cleanup(labels=("type:slice", "in-review", "in-progress", "area:docs"), worktree=True)
        self.assertEqual(0, code, out)
        edits = self._label_edits(effects)
        self.assertEqual(1, len(edits))
        removed = [edits[0][i + 1] for i, arg in enumerate(edits[0]) if arg == "--remove-label"]
        self.assertEqual(sorted(["in-review", "in-progress"]), sorted(removed))
        self.assertNotIn("--add-label", edits[0])
        # Status is read and written only after the label edit, and last.
        kinds = [e[0] for e in effects]
        self.assertLess(effects.index(("run", edits[0])), kinds.index("status-read"))
        self.assertEqual(["status-read", "write"], kinds[-2:])

    def test_cleanup_removes_the_worktree_without_force(self):
        _code, effects, _out = self._cleanup(worktree=True)
        removes = [e[1] for e in effects if e[0] == "run" and e[1][:3] == ["git", "worktree", "remove"]]
        self.assertEqual(1, len(removes))
        self.assertNotIn("--force", removes[0])
        self.assertNotIn("-f", removes[0])

    def test_cleanup_rerun_edits_no_labels_and_still_renders_status(self):
        # A rerun after a partial failure: labels already cleared, worktree
        # and branch already gone. Nothing is edited; status is still rendered.
        code, effects, out = self._cleanup(labels=("type:slice", "area:docs"), local_branch=False)
        self.assertEqual(0, code, out)
        self.assertEqual([], self._label_edits(effects))
        self.assertNotIn("CLEARED", out)
        self.assertEqual(["status-read", "write"], [e[0] for e in effects][-2:])

    def test_cleanup_judges_the_branch_merged_against_the_fetched_remote_base(self):
        # A stale local default branch must not make a merged branch look
        # unmerged: the prune happens first, and containment is checked
        # against origin/<default>, never this checkout's HEAD.
        default = self.config["default_branch"]
        code, effects, out = self._cleanup(worktree=True)
        self.assertEqual(0, code, out)
        runs = [e[1] for e in effects if e[0] == "run"]
        prune = runs.index(["git", "fetch", "origin", "--prune"])
        check = runs.index(["git", "merge-base", "--is-ancestor", "refs/heads/issue/11",
                            f"refs/remotes/origin/{default}"])
        delete = runs.index(["git", "branch", "-D", "issue/11"])
        self.assertLess(prune, check)
        self.assertLess(check, delete)
        self.assertNotIn(["git", "branch", "-d", "issue/11"], runs)

    def test_cleanup_keeps_an_unmerged_branch_and_still_renders_status(self):
        code, effects, out = self._cleanup(merged=False)
        self.assertEqual(1, code)
        self.assertIn("BLOCKED: kept local branch `issue/11`", out)
        self.assertNotIn("CLEANED", out)
        runs = [e[1] for e in effects if e[0] == "run"]
        self.assertFalse([r for r in runs if r[:2] == ["git", "branch"]], "an unmerged branch must not be deleted")
        self.assertEqual(["status-read", "write"], [e[0] for e in effects][-2:])

    def test_cleanup_of_an_open_issue_has_no_effects(self):
        code, effects, out = self._cleanup(state="OPEN")
        self.assertEqual(1, code)
        self.assertIn("BLOCKED", out)
        self.assertEqual([], effects)


if __name__ == "__main__":
    unittest.main()
