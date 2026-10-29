#!/usr/bin/env python3
"""Require successful native jobs from GitHub's needs object."""

import json
import os
import sys


def main():
    required = sys.argv[1:]
    if not required:
        print("No required native job names supplied", file=sys.stderr)
        return 1

    raw = os.environ.get("NEEDS_JSON")
    if raw is None:
        print("NEEDS_JSON is missing", file=sys.stderr)
        return 1
    try:
        needs = json.loads(raw)
    except json.JSONDecodeError as error:
        print(f"NEEDS_JSON is invalid JSON: {error.msg}", file=sys.stderr)
        return 1
    if not isinstance(needs, dict):
        print("NEEDS_JSON must be a job object", file=sys.stderr)
        return 1

    failed = False
    for name in required:
        if name not in needs:
            state = "missing job"
            success = False
        elif not isinstance(needs[name], dict):
            state = f"invalid job: {needs[name]!r}"
            success = False
        else:
            result = needs[name].get("result")
            state = f"result={result!r}"
            success = result == "success"
        print(f"{name}: {state}", file=sys.stdout if success else sys.stderr)
        if not success:
            failed = True
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
