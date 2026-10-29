"""Bounded native metadata inspection; imports alone do not prove actual loads."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import sys
import time

from process_runner import run_process
from qualify import digest_file, write_record


def inspect_file(path: Path, tool: Path) -> dict:
    path = path.resolve(strict=True)
    tool = tool.resolve(strict=True)
    if not path.is_file() or not tool.is_file():
        raise ValueError("native artifact and inspection tool must be regular files")
    record = {"file": path.name, "bytes": path.stat().st_size, "sha256": digest_file(path),
              "tool_sha256": digest_file(tool), "imports": [], "rpaths": [],
              "architecture": None, "cpu_subtype": None, "platform": None,
              "minimum_os": None, "sdk": None,
              "install_name": None, "actual_loads": "not-observed"}
    if sys.platform == "darwin":
        args = [str(tool), "-l", str(path)]
    elif sys.platform == "win32":
        args = [str(tool), "/headers", "/dependents", str(path)]
    else:
        raise ValueError("native metadata requires a release target")
    environment = {key: os.environ[key] for key in ("SystemRoot", "WINDIR") if key in os.environ}
    result = run_process(args, cwd=path.parent, env=environment, timeout_seconds=30,
                         output_limit_bytes=1048576)
    if result["exit_code"] != 0 or result["timed_out"] or result["output_limited"] or not result["cleanup_ok"] or result["launch_error"]:
        raise ValueError("native inspection failed")
    text = result["stdout"]
    if sys.platform == "darwin":
        record["format"] = "Mach-O"
        # Architecture comes from a separate header read, not the host process.
        header = run_process([str(tool), "-hv", str(path)], cwd=path.parent, env=environment,
                             timeout_seconds=30, output_limit_bytes=1048576)
        if header["exit_code"] != 0 or header["timed_out"] or header["output_limited"] or not header["cleanup_ok"] or header["launch_error"]:
            raise ValueError("Mach-O header inspection failed")
        arch = re.findall(r"(?m)^MH_MAGIC_64\s+(\S+)\s+(\S+)\s+", header["stdout"])
        if len(arch) != 1 or arch[0][0] not in ("ARM64", "X86_64"):
            raise ValueError("expected one supported native architecture")
        record["architecture"], record["cpu_subtype"] = arch[0]
        for command in re.split(r"(?m)^Load command \d+\s*$", text)[1:]:
            kind = re.search(r"(?m)^\s*cmd (LC_\w+)\s*$", command)
            if not kind:
                raise ValueError("incomplete load command")
            kind = kind[1]
            if kind in ("LC_LOAD_DYLIB", "LC_LOAD_WEAK_DYLIB", "LC_REEXPORT_DYLIB", "LC_LOAD_UPWARD_DYLIB", "LC_ID_DYLIB"):
                name = re.search(r"(?m)^\s*name (.*?) \(offset \d+\)\s*$", command)
                if not name:
                    raise ValueError("incomplete dylib identity")
                if kind == "LC_ID_DYLIB":
                    record["install_name"] = name[1]
                else:
                    record["imports"].append({"name": name[1], "kind": kind})
            elif kind == "LC_RPATH":
                name = re.search(r"(?m)^\s*path (.*?) \(offset \d+\)\s*$", command)
                if not name:
                    raise ValueError("incomplete rpath")
                record["rpaths"].append(name[1])
            elif kind == "LC_BUILD_VERSION":
                platform = re.search(r"(?m)^\s*platform (\S+)\s*$", command)
                if not platform or platform[1] not in ("1", "MACOS"):
                    raise ValueError("expected native macOS platform")
                record["platform"] = "macOS"
                minimum = re.search(r"(?m)^\s*minos ([\d.]+)\s*$", command)
                sdk = re.search(r"(?m)^\s*sdk ([\d.]+)\s*$", command)
                if not minimum or not sdk:
                    raise ValueError("incomplete deployment metadata")
                record["minimum_os"], record["sdk"] = minimum[1], sdk[1]
            elif kind == "LC_VERSION_MIN_MACOSX":
                record["platform"] = "macOS"
                minimum = re.search(r"(?m)^\s*version ([\d.]+)\s*$", command)
                if not minimum:
                    raise ValueError("incomplete minimum version")
                record["minimum_os"] = minimum[1]
        if record["minimum_os"] is None:
            raise ValueError("missing deployment minimum")
    else:
        record["format"] = "PE"
        machine = re.findall(r"(?mi)^\s*([0-9a-f]+) machine \(([^)]+)\)", text)
        if len(machine) != 1:
            raise ValueError("expected one PE machine")
        record["architecture"] = machine[0][1]
        record["imports"] = [{"name": name, "kind": "PE-import"} for name in
                             re.findall(r"(?mi)^\s+([^\s]+\.dll)\s*$", text)]
        record["dynamic_imports"] = "requires-provider-manifest-and-runtime-observation"
    return record


def inspect_macos_closure(path: Path, tool: Path) -> dict:
    """Inventory canonical non-system files; sibling rpath resolution is not a runtime oracle."""
    if sys.platform != "darwin":
        raise ValueError("Mach-O closure inspection requires macOS")
    origin = path.resolve(strict=True)
    queue, seen, files, unresolved = [origin], set(), [], []
    deadline = time.monotonic() + 120
    total = 0
    origin_metadata = None
    while queue:
        current = queue.pop()
        if current in seen:
            continue
        if len(seen) >= 512 or time.monotonic() >= deadline:
            raise ValueError("native closure inspection limit")
        seen.add(current)
        observed = inspect_file(current, tool)
        if current == origin:
            origin_metadata = {key: observed[key] for key in
                               ("file", "bytes", "sha256", "tool_sha256", "architecture",
                                "cpu_subtype", "platform", "minimum_os", "sdk")}
        if current != origin:
            total += observed["bytes"]
            if total > 4294967296:
                raise ValueError("native closure byte limit")
            files.append({key: observed[key] for key in
                          ("file", "bytes", "sha256", "architecture", "cpu_subtype", "platform", "minimum_os", "sdk")})
            files[-1]["redistribution"] = "unresolved"
        for dependency in observed["imports"]:
            token = dependency["name"]
            if token.startswith(("/usr/lib/", "/System/Library/")):
                continue
            if token.startswith(("@rpath/", "@loader_path/")):
                candidate = current.parent / token.split("/", 1)[1]
            elif token.startswith("/"):
                candidate = Path(token)
            else:
                unresolved.append({"parent": current.name, "dependency": Path(token).name})
                continue
            if candidate.is_file():
                queue.append(candidate.resolve(strict=True))
            else:
                unresolved.append({"parent": current.name, "dependency": candidate.name})
    return {"schema_version": 1, "origin_sha256": origin_metadata["sha256"],
            "origin": origin_metadata, "tool_sha256": origin_metadata["tool_sha256"], "native_bytes": total,
            "files": sorted(files, key=lambda row: row["file"]), "unresolved_imports": unresolved,
            "resolution": "static-absolute-or-sibling-rpath", "actual_loads": "not-observed"}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--file", type=Path, required=True)
    parser.add_argument("--tool", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--closure", action="store_true")
    args = parser.parse_args()
    try:
        record = inspect_macos_closure(args.file, args.tool) if args.closure else inspect_file(args.file, args.tool)
        write_record(args.output, record)
        print(json.dumps({"actual_loads": record["actual_loads"], "output": str(args.output)}))
        return 0
    except (OSError, ValueError) as error:
        print(f"native inspection refused: {type(error).__name__}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
