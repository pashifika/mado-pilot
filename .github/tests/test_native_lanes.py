"""Exercise native job aggregation without GitHub access or credentials."""

import json
import os
from pathlib import Path
import subprocess
import sys
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "scripts/check-native-lanes.py"


class NativeLanes(unittest.TestCase):
    def check(self, needs, *required):
        environment = dict(os.environ)
        if needs is None:
            environment.pop("NEEDS_JSON", None)
        else:
            environment["NEEDS_JSON"] = (
                needs if isinstance(needs, str) else json.dumps(needs)
            )
        return subprocess.run(
            [sys.executable, str(SCRIPT), *required],
            env=environment,
            capture_output=True,
            text=True,
            timeout=10,
        )

    def test_all_required_jobs_succeed(self):
        result = self.check(
            {
                "windows-lanes": {"result": "success"},
                "macos-lanes": {"result": "success"},
            },
            "windows-lanes",
            "macos-lanes",
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("windows-lanes: result='success'", result.stdout)
        self.assertIn("macos-lanes: result='success'", result.stdout)

    def test_one_failed_job_among_multiple_fails_with_identity(self):
        result = self.check(
            {
                "windows-lanes": {"result": "failure"},
                "macos-lanes": {"result": "success"},
            },
            "windows-lanes",
            "macos-lanes",
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("windows-lanes: result='failure'", result.stderr)
        self.assertIn("macos-lanes: result='success'", result.stdout)

    def test_cancelled_skipped_and_invalid_results_fail(self):
        for state in ["cancelled", "skipped", "unknown", "", None, True, 1, [], {}]:
            with self.subTest(state=state):
                result = self.check(
                    {"windows-lanes": {"result": state}}, "windows-lanes"
                )
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(f"windows-lanes: result={state!r}", result.stderr)

    def test_missing_job_fails_with_identity(self):
        result = self.check(
            {"macos-lanes": {"result": "success"}}, "windows-lanes"
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("windows-lanes: missing job", result.stderr)

    def test_missing_result_fails_with_identity(self):
        result = self.check({"windows-lanes": {}}, "windows-lanes")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("windows-lanes: result=None", result.stderr)

    def test_invalid_job_structure_fails_with_identity(self):
        for job in [None, [], "success", True, 1]:
            with self.subTest(job=job):
                result = self.check({"windows-lanes": job}, "windows-lanes")
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("windows-lanes: invalid job", result.stderr)

    def test_missing_or_malformed_needs_fails(self):
        for needs in [None, "", "not json", "{"]:
            with self.subTest(needs=needs):
                result = self.check(needs, "windows-lanes")
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("NEEDS_JSON", result.stderr)

    def test_invalid_needs_structure_fails(self):
        for needs in ["null", "[]", '"success"', "true", "42"]:
            with self.subTest(needs=needs):
                result = self.check(needs, "windows-lanes")
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("NEEDS_JSON must be a job object", result.stderr)

    def test_no_required_names_fails(self):
        result = self.check({"windows-lanes": {"result": "success"}})
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("No required native job names", result.stderr)

    def test_selector_outcome_does_not_override_native_result(self):
        for selector in ["skipped", "failure"]:
            for native in ["success", "failure", "cancelled", "skipped"]:
                with self.subTest(selector=selector, native=native):
                    result = self.check(
                        {
                            "dev-push-policy": {"result": selector},
                            "windows-lanes": {"result": native},
                        },
                        "windows-lanes",
                    )
                    self.assertEqual(result.returncode, 0 if native == "success" else 1)
                    self.assertIn(
                        f"windows-lanes: result={native!r}",
                        result.stdout + result.stderr,
                    )
                    self.assertNotIn(
                        "dev-push-policy", result.stdout + result.stderr
                    )


if __name__ == "__main__":
    unittest.main()
