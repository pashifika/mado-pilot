"""Offline current-profile contracts and read-only historical provenance; no child executes."""

import io
import json
from pathlib import Path
import sys
import tempfile
import tomllib
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import workloads

ROOT = workloads.ROOT
TARGETS = ("aarch64-apple-darwin", "x86_64-pc-windows-msvc")


class CurrentProfiles(unittest.TestCase):
    def test_each_target_loads_from_a_product_only_tree_without_prose_authority(self):
        for target in TARGETS:
            with self.subTest(target=target), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                current = ROOT / "docs/benchmarks" / f"ocr-text-watch-v3-{target}.toml"
                destination = root / current.relative_to(ROOT)
                destination.parent.mkdir(parents=True)
                destination.write_bytes(current.read_bytes())
                with patch.object(workloads, "ROOT", root), \
                        patch.object(workloads.run_replay, "selected_target", return_value=target):
                    selected, identity = workloads.profile(destination)
                self.assertEqual(selected["profile"]["release_target"], target)
                self.assertEqual(identity, workloads.run_replay.identity(destination))
                self.assertFalse((root / "rasen").exists())
                self.assertFalse((root / "docs/adr").exists())
                self.assertFalse((root / "docs/performance").exists())

    def test_original_historical_profiles_inspect_without_rewriting_or_execution(self):
        for target in TARGETS:
            with self.subTest(target=target):
                historical = ROOT / "docs/benchmarks" / f"ocr-text-watch-{target}.toml"
                before = historical.read_bytes()
                output = io.StringIO()
                with patch.object(sys, "argv", ["workloads.py", "--inspect-profile", str(historical)]), \
                        patch.object(sys, "stdout", output), \
                        patch.object(workloads.run_replay, "run_process") as launch:
                    self.assertEqual(workloads.main(), 0)
                    launch.assert_not_called()
                record = json.loads(output.getvalue())
                self.assertFalse(record["execution_requested"])
                self.assertEqual(record["profile"], tomllib.loads(before.decode("utf-8")))
                self.assertEqual(historical.read_bytes(), before)
                with patch.object(workloads.run_replay, "selected_target", return_value=target):
                    with self.assertRaisesRegex(ValueError, "read-only.*format_version = 3.*--inspect-profile"):
                        workloads.profile(historical)

    def test_current_profile_cannot_execute_under_a_foreign_target(self):
        selected = ROOT / "docs/benchmarks" / f"ocr-text-watch-v3-{TARGETS[0]}.toml"
        with patch.object(workloads.run_replay, "selected_target", return_value=TARGETS[1]):
            with self.assertRaisesRegex(ValueError, "profile and execution target differ"):
                workloads.profile(selected)

    def test_inspection_cannot_be_combined_with_execution(self):
        selected = ROOT / "docs/benchmarks" / f"ocr-text-watch-v3-{TARGETS[0]}.toml"
        with patch.object(sys, "argv", ["workloads.py", "--inspect-profile", str(selected), "--execute"]), \
                patch.object(sys, "stderr", io.StringIO()), \
                patch.object(workloads.run_replay, "run_process") as launch:
            with self.assertRaises(SystemExit) as error:
                workloads.main()
            self.assertEqual(error.exception.code, 2)
            launch.assert_not_called()


if __name__ == "__main__":
    unittest.main()
