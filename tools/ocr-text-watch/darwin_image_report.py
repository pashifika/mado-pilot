#!/usr/bin/env python3
"""Exec one candidate with a private, kernel-bounded dyld diagnostic file."""

from __future__ import annotations

import os
from pathlib import Path
import signal
import sys

REPORT_ENV = "MADO_PILOT_OCR_DEPENDENCY_REPORT"
MAX_REPORT_BYTES = 1 << 20


def main() -> int:
    if sys.platform != "darwin":
        raise ValueError("dyld report launcher requires Darwin")
    command = sys.argv[1:]
    if not command or not Path(command[0]).is_absolute():
        raise ValueError("one absolute candidate executable is required")
    report = Path(os.environ[REPORT_ENV])
    if not report.is_absolute() or not report.parent.is_dir():
        raise ValueError("an existing private report parent is required")
    if any(key.startswith("DYLD_PRINT_") and value for key, value in os.environ.items()):
        raise ValueError("ambient dyld diagnostic settings are refused")
    import resource

    # Limit all regular-file writes in this child; pipes retain the supervisor's
    # separate output bound. Never relax an inherited file-size limit.
    soft, hard = resource.getrlimit(resource.RLIMIT_FSIZE)
    limits = [MAX_REPORT_BYTES] + [value for value in (soft, hard) if value != resource.RLIM_INFINITY]
    limit = min(limits)
    resource.setrlimit(resource.RLIMIT_FSIZE, (limit, limit))
    signal.signal(signal.SIGXFSZ, signal.SIG_DFL)
    os.umask(0o077)
    descriptor = os.open(report, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    os.close(descriptor)
    environment = dict(os.environ)
    environment["DYLD_PRINT_LIBRARIES"] = "1"
    environment["DYLD_PRINT_TO_FILE"] = str(report)
    os.execve(command[0], command, environment)
    return 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, KeyError):
        print("bounded dyld report launch failed", file=sys.stderr)
        raise SystemExit(1)
