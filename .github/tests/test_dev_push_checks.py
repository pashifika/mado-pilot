"""Exercise the push-check selector without GitHub access or credentials."""

import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "scripts/select-dev-push-checks.sh"
REPOSITORY = "owner/project"
BRANCH = "dev/0.5.0"
SHA = "a" * 40


def release_pr():
    return {
        "state": "open",
        "base": {"ref": "main", "repo": {"full_name": REPOSITORY}},
        "head": {
            "ref": BRANCH,
            "sha": SHA,
            "repo": {"full_name": REPOSITORY},
        },
    }


class DevPushChecks(unittest.TestCase):
    def select(self, pages, *, status=0, branch=BRANCH):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            gh = root / "gh"
            gh.write_text(
                "#!/usr/bin/env python3\n"
                "import os, sys\n"
                "print(os.environ['TEST_RESPONSE'])\n"
                "sys.exit(int(os.environ['TEST_STATUS']))\n",
                encoding="utf-8",
            )
            gh.chmod(0o755)
            output = root / "output"
            environment = {
                **os.environ,
                "PATH": f"{root}{os.pathsep}{os.environ['PATH']}",
                "GITHUB_REPOSITORY": REPOSITORY,
                "GITHUB_REPOSITORY_OWNER": "owner",
                "GITHUB_REF_NAME": branch,
                "GITHUB_SHA": SHA,
                "GITHUB_OUTPUT": str(output),
                "TEST_RESPONSE": pages if isinstance(pages, str) else json.dumps(pages),
                "TEST_STATUS": str(status),
            }
            subprocess.run(
                ["bash", str(SCRIPT)],
                env=environment,
                check=True,
                capture_output=True,
                text=True,
                timeout=10,
            )
            return output.read_text(encoding="utf-8").strip()

    def test_open_release_pr_replaces_only_its_exact_push(self):
        self.assertEqual(self.select([[release_pr()]]), "skip-checks=true")
        self.assertEqual(self.select([[]]), "skip-checks=false")
        closed = release_pr()
        closed["state"] = "closed"
        self.assertEqual(self.select([[closed]]), "skip-checks=false")

    def test_unrelated_or_uncovered_heads_keep_post_merge_verification(self):
        for field, value in [
            (("base", "ref"), "dev/0.5.0"),
            (("base", "repo", "full_name"), "owner/another-project"),
            (("head", "repo", "full_name"), "fork/project"),
            (("head", "ref"), "dev/0.6.0"),
            (("head", "sha"), "b" * 40),
            (("head", "repo"), None),
        ]:
            with self.subTest(field=field, value=value):
                unrelated = release_pr()
                target = unrelated
                for component in field[:-1]:
                    target = target[component]
                target[field[-1]] = value
                self.assertEqual(self.select([[unrelated]]), "skip-checks=false")

    def test_pagination_and_drafts_still_find_a_covering_release_pr(self):
        draft = release_pr()
        draft["draft"] = True
        unrelated = release_pr()
        unrelated["head"]["ref"] = "dev/0.6.0"
        self.assertEqual(self.select([[unrelated], [draft]]), "skip-checks=true")

    def test_failed_or_unreadable_lookup_never_suppresses_checks(self):
        # Even plausible stdout from a failed API invocation is not authority.
        self.assertEqual(self.select([[release_pr()]], status=1), "skip-checks=false")
        for response in ["not json", {"message": "API failure"}, None]:
            with self.subTest(response=response):
                self.assertEqual(self.select(response), "skip-checks=false")

    def test_non_release_development_branch_keeps_its_push_checks(self):
        candidate = release_pr()
        candidate["head"]["ref"] = "dev/experiment"
        self.assertEqual(
            self.select([[candidate]], branch="dev/experiment"), "skip-checks=false"
        )


if __name__ == "__main__":
    unittest.main()
