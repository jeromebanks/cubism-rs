import importlib.util
import json
from pathlib import Path
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
        self.assertIn("parent epic #10 is paused at a human gate", errors)

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
