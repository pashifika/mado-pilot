"""Protect recursive evidence privacy and report-specific failure diagnostics."""

import importlib.util
from pathlib import Path
import re
import subprocess
import unittest
from urllib.parse import unquote, urlsplit


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "g004_evidence_validation", ROOT / "docs/evidence/g-004/validate.py"
)
validator = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(validator)


class EvidencePrivacyTests(unittest.TestCase):
    def test_recognized_payload_is_rejected_through_nested_containers(self):
        cases = (
            ("Apple report", {"metadata": {"text": "private fixture payload"}}),
            ("Windows report", [{"metadata": {"observed": "private fixture payload"}}]),
            ("Apple v5 report", {"metadata": [[{"raw": "private fixture payload"}]]}),
        )
        for label, value in cases:
            with self.subTest(label=label):
                with self.assertRaises(SystemExit) as failure:
                    validator.reject_unapproved_payload(value, label)
                self.assertEqual(
                    str(failure.exception),
                    f"G-004 evidence invalid: {label} contains unapproved recognized payload fields",
                )

    def test_host_paths_are_rejected_inside_lists_and_dictionaries(self):
        cases = (
            ("Apple v5 report", {"metadata": ["path=/Users/fixture/private"]}),
            ("Windows v5 report", [{"metadata": r"path=C:\Users\fixture\private"}]),
        )
        for label, value in cases:
            with self.subTest(label=label):
                with self.assertRaises(SystemExit) as failure:
                    validator.reject_unapproved_payload(value, label)
                self.assertEqual(
                    str(failure.exception),
                    f"G-004 evidence invalid: {label} contains a host-local path",
                )

    def test_allowed_metadata_becomes_invalid_when_raw_payload_is_added(self):
        value = {"metadata": [None, False, 7, 0.25, "fixture", {"language": "ja"}]}
        validator.reject_unapproved_payload(value, "Windows v5 report")
        value["metadata"][-1]["raw"] = "private fixture payload"
        with self.assertRaises(SystemExit) as failure:
            validator.reject_unapproved_payload(value, "Windows v5 report")
        self.assertEqual(
            str(failure.exception),
            "G-004 evidence invalid: Windows v5 report contains unapproved recognized payload fields",
        )


class PublicEvidenceLinksTests(unittest.TestCase):
    def test_qualification_links_resolve_without_separate_planning_storage(self):
        # The product Git manifest is the public-checkout view; ignored Rasen
        # storage cannot satisfy a reference even when present on this machine.
        tracked = set(subprocess.check_output(
            ["git", "ls-files", "-z"], cwd=ROOT, text=True
        ).split("\0"))
        documents = (
            "docs/performance.md",
            "docs/macos-input-verification.md",
            "docs/evidence/phase-2-native/macos-owning-process-qualification.md",
        )
        missing = []
        for document in documents:
            source = ROOT / document
            for target in re.findall(r"\[[^\]\n]+\]\(([^)\n]+)\)", source.read_text(encoding="utf-8")):
                parsed = urlsplit(target)
                if parsed.scheme or parsed.netloc or not parsed.path:
                    continue
                path = (source.parent / unquote(parsed.path)).resolve()
                try:
                    relative = path.relative_to(ROOT).as_posix()
                except ValueError:
                    missing.append(f"{document}: {target}")
                    continue
                if relative in tracked and path.is_file():
                    continue
                if path.is_dir() and any(entry.startswith(relative + "/") for entry in tracked):
                    continue
                missing.append(f"{document}: {target}")
        self.assertEqual(missing, [], "public evidence depends on missing or untracked files")


if __name__ == "__main__":
    unittest.main()
