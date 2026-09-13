#!/usr/bin/env python3
"""Collect one candidate's dyld diagnostics without limiting unrelated file writes."""

from __future__ import annotations

import os
import select
import signal
import subprocess
import sys
import time
from pathlib import Path

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
    if any(
        key.startswith("DYLD_PRINT_") and value for key, value in os.environ.items()
    ):
        raise ValueError("ambient dyld diagnostic settings are refused")
    os.umask(0o077)
    descriptor = os.open(report, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    interrupted = False

    def interrupt(_signum, _frame):
        nonlocal interrupted
        interrupted = True

    handlers = {
        number: signal.getsignal(number) for number in (signal.SIGTERM, signal.SIGINT)
    }
    process = None
    code = 1
    try:
        for number in handlers:
            signal.signal(number, interrupt)
        with os.fdopen(descriptor, "wb", buffering=0) as output:
            reader, writer = os.pipe()
            try:
                os.set_blocking(reader, False)
                environment = dict(os.environ)
                channel = f"/dev/fd/{writer}"
                environment[REPORT_ENV] = channel
                environment["DYLD_PRINT_LIBRARIES"] = "1"
                environment["DYLD_PRINT_TO_FILE"] = channel
                # Stay in the supervisor-owned process group. Only the bounded
                # diagnostic pipe crosses exec; stdout/stderr remain inherited.
                process = subprocess.Popen(command, env=environment, pass_fds=(writer,))
                os.close(writer)
                writer = None
                count = 0
                while True:
                    if interrupted:
                        raise InterruptedError("image collection interrupted")
                    # An exit concurrent with select must get another drain.
                    code = process.poll()
                    ready, _, _ = select.select(
                        [reader], [], [], 0 if code is not None else 0.05
                    )
                    if ready:
                        chunk = os.read(reader, 65536)
                        if chunk:
                            remaining = MAX_REPORT_BYTES - count
                            piece = chunk[:remaining]
                            if output.write(piece) != len(piece):
                                raise OSError("short image report write")
                            count += len(piece)
                            if len(chunk) >= remaining:
                                raise ValueError("native image report saturated")
                            continue
                    if code is not None:
                        break
                    if ready:
                        time.sleep(0.05)
            finally:
                if writer is not None:
                    os.close(writer)
                os.close(reader)
                if process is not None and process.poll() is None:
                    process.kill()
                    process.wait(timeout=1)
    finally:
        for number, handler in handlers.items():
            signal.signal(number, handler)
    if code < 0:
        number = -code
        if number not in (signal.SIGKILL, signal.SIGSTOP):
            signal.signal(number, signal.SIG_DFL)
        os.kill(os.getpid(), number)
    return code


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, KeyError, subprocess.SubprocessError):
        print("bounded dyld report launch failed", file=sys.stderr)
        raise SystemExit(1)
