#!/usr/bin/env python3
"""Keep trigger work in the mandatory cross-scale regression gate."""
from copy import deepcopy
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


class TriggerWorkBudgetTest(unittest.TestCase):
    def setUp(self):
        names = [
            "read_list", "read_single", "slack_wake_empty", "slack_due_empty",
            "slack_wake_team_empty", "slack_due_team_empty",
            "display_row_empty", "display_scope_empty", "slack_wake_ready",
            "slack_due_ready", "display_row_ready", "display_scope_ready",
            "message_insert", "message_delete_latest",
        ]
        # Parent row counts stay constant even when a trigger scans history.
        self.small = {
            name: {"relations": {"messages": 1}, "intermediate": 1,
                   "sort_input": 0, "logical_blocks": 100}
            for name in names
        }
        for name in ("message_insert", "message_delete_latest"):
            self.small[name]["nested"] = dict(
                relations={"messages": 1}, intermediate=1, sort_input=0,
                logical_blocks=100, statements=5,
            )
        self.large = deepcopy(self.small)

    def compare(self):
        with tempfile.TemporaryDirectory() as directory:
            paths = [Path(directory, name) for name in ("300000", "1000000")]
            for path, work in zip(paths, (self.small, self.large)):
                path.mkdir()
                (path / "work.json").write_text(json.dumps(work))
            return subprocess.run(
                [sys.executable, str(Path(__file__).with_name(
                    "compare-read-state-worker-work.py")), *map(str, paths)],
                capture_output=True, text=True, check=False,
            )

    def test_bounded_trigger_work_passes(self):
        # Allow fixed overhead and B-tree height changes at the documented edge.
        for name in ("message_insert", "message_delete_latest"):
            self.large[name]["nested"]["logical_blocks"] = 456
        result = self.compare()
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_history_scan_inside_either_write_trigger_fails(self):
        for name in ("message_insert", "message_delete_latest"):
            with self.subTest(query=name):
                self.large = deepcopy(self.small)
                self.large[name]["nested"]["logical_blocks"] = 457
                result = self.compare()
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(name + "/nested: logical I/O grows with history", result.stderr)

    def test_nested_history_rows_fail_even_with_constant_parent_plan(self):
        self.large["message_insert"]["nested"]["relations"]["messages"] = 67
        result = self.compare()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("message_insert/nested: messages grows with history", result.stderr)

    def test_team_scoped_claim_history_scan_fails(self):
        self.large["slack_due_team_empty"]["relations"]["slack_channel_syncs"] = 67
        result = self.compare()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("slack_due_team_empty: slack_channel_syncs grows with history", result.stderr)

    def test_missing_nested_capture_fails(self):
        del self.large["message_delete_latest"]["nested"]
        result = self.compare()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("message_delete_latest: missing trigger plans", result.stderr)


if __name__ == "__main__":
    unittest.main()
