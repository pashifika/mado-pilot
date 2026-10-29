"""Prepare a private relocated host-root experiment; grant no redistribution or clean-host claim."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import sys
import time

from inspect_native import inspect_file
from process_runner import run_process
from qualify import digest_file, private_directory, write_record


def prepare(library: Path, output: Path, *, otool: Path, install_name_tool: Path, codesign: Path) -> dict:
    if sys.platform != "darwin":
        raise ValueError("native macOS builder required")
    library = library.resolve(strict=True)
    private_directory(output)
    output = output.resolve(strict=True)
    record = {"schema_version": 1, "status": "failed", "qualification": "not-selected",
              "ownership": "private-host-projection", "redistribution": "unresolved",
              "source_library_sha256": digest_file(library), "files": [], "commands": []}
    commands = record["commands"]
    environment = {"PATH": "/usr/bin:/bin", "HOME": str(output), "TMPDIR": str(output)}
    deadline = time.monotonic() + 120
    native = output / "native"
    native.mkdir()

    def checked(args: list[str]):
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise ValueError("preparation deadline")
        result = run_process(args, cwd=output, env=environment, timeout_seconds=min(30, remaining),
                             output_limit_bytes=1048576)
        commands.append({"argv": args, "exit_code": result["exit_code"], "timed_out": result["timed_out"],
                         "output_limited": result["output_limited"], "cleanup_ok": result["cleanup_ok"]})
        if result["exit_code"] != 0 or result["timed_out"] or result["output_limited"] or not result["cleanup_ok"] or result["launch_error"]:
            raise ValueError("native preparation command failed")

    def dependency_path(source: Path, token: str) -> Path:
        if token.startswith(("@rpath/", "@loader_path/")):
            path = source.parent / token.split("/", 1)[1]
        elif token.startswith("/"):
            path = Path(token)
        else:
            raise ValueError("unresolved native import")
        return path.resolve(strict=True)

    try:
        queue, inventory, names = [library], {}, {}
        total = 0
        while queue:
            source = queue.pop()
            if source in inventory:
                continue
            if len(inventory) >= 512 or time.monotonic() >= deadline:
                raise ValueError("native closure limit")
            inspected = inspect_file(source, otool)
            if source.name in names:
                raise ValueError("ambiguous native destination name")
            names[source.name] = source
            total += inspected["bytes"]
            if total > 4294967296:
                raise ValueError("native byte limit")
            dependencies = {}
            for entry in inspected["imports"]:
                token = entry["name"]
                if token.startswith(("/usr/lib/", "/System/Library/")):
                    continue
                resolved = dependency_path(source, token)
                if not resolved.is_file():
                    raise ValueError("native dependency is not a file")
                dependencies[token] = resolved
                queue.append(resolved)
            inventory[source] = (inspected, dependencies)
        for source, (inspected, _) in inventory.items():
            destination = native / source.name
            with source.open("rb") as reader, destination.open("xb") as writer:
                remaining = inspected["bytes"]
                while remaining:
                    chunk = reader.read(min(remaining, 1048576))
                    if not chunk or time.monotonic() >= deadline:
                        raise ValueError("native copy interrupted")
                    writer.write(chunk)
                    remaining -= len(chunk)
                if reader.read(1):
                    raise ValueError("native source grew")
            if digest_file(destination) != inspected["sha256"]:
                raise ValueError("native source identity changed")
            os.chmod(destination, 0o755)
        for source, (inspected, dependencies) in inventory.items():
            destination = native / source.name
            args = [str(install_name_tool), "-id", "@loader_path/" + source.name]
            for token, dependency in dependencies.items():
                args += ["-change", token, "@loader_path/" + dependency.name]
            # Remove all builder rpaths; every non-system import is now file-relative.
            for rpath in inspected["rpaths"]:
                args += ["-delete_rpath", rpath]
            checked(args + [str(destination)])
            checked([str(codesign), "--force", "--sign", "-", str(destination)])
            verified = inspect_file(destination, otool)
            if verified["rpaths"] or any(not entry["name"].startswith(("/usr/lib/", "/System/Library/", "@loader_path/")) for entry in verified["imports"]):
                raise ValueError("developer import survived relocation")
            record["files"].append({"file": source.name, "source_sha256": inspected["sha256"],
                                    "source_bytes": inspected["bytes"], "sha256": verified["sha256"],
                                    "bytes": verified["bytes"], "minimum_os": verified["minimum_os"],
                                    "architecture": verified["architecture"], "signing": "local-ad-hoc"})
        record.update(status="passed", native_bytes=sum(row["bytes"] for row in record["files"]),
                      root_library=library.name)
    finally:
        write_record(output / "preparation.json", record)
    return record


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--library", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--otool", type=Path, default=Path("/usr/bin/otool"))
    parser.add_argument("--install-name-tool", type=Path, default=Path("/usr/bin/install_name_tool"))
    parser.add_argument("--codesign", type=Path, default=Path("/usr/bin/codesign"))
    args = parser.parse_args()
    try:
        record = prepare(args.library, args.output, otool=args.otool.resolve(strict=True),
                         install_name_tool=args.install_name_tool.resolve(strict=True), codesign=args.codesign.resolve(strict=True))
        print(json.dumps({key: record[key] for key in ("status", "qualification", "native_bytes")}))
        return 0
    except (OSError, ValueError) as error:
        print(f"private root preparation failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
