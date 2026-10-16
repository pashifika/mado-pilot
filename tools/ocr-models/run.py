#!/usr/bin/env python3
"""Run real Rust OCR in fresh bounded serial processes, or evaluate retained reports per rule."""
from __future__ import annotations

import argparse
import base64
import contextlib
import hashlib
import json
import os
import platform
import shutil
import sys
import tempfile
from pathlib import Path

from report import (BUILD_FIELDS, BUILD_SOURCE_FIELDS, COHORT_FIELDS, CRITERIA, document_bytes, evaluate,
                    evaluate_reports, evaluation_bundle, parse_process, sha256, strict_json, validate_binding,
                    validate_budgets)

ROOT = Path(__file__).resolve().parents[2]
RUNNER_DIR = ROOT / "tools/native-release-profile"
sys.path.insert(0, str(RUNNER_DIR))
from process_runner import run_process  # noqa: E402

MANIFESTS = (ROOT / "fixtures/ocr/g-004/fixture-manifest.json",
             ROOT / "fixtures/ocr/compact-ui-v1/fixture-manifest.json")
CRITERIA_PATH = Path(__file__).with_name("criteria-v1.json")
CATALOG_PATH = ROOT / "crates/automation/ocr/resources/model-catalog.json"


def load(path):
    return strict_json(Path(path).read_text(encoding="utf-8"))


def save(path, value):
    with Path(path).open("xb") as stream:
        stream.write(document_bytes(value))


def environment():
    names = ("PATH", "HOME", "USERPROFILE", "HOMEDRIVE", "HOMEPATH", "LOCALAPPDATA", "APPDATA",
             "CARGO_HOME", "RUSTUP_HOME", "TMPDIR", "TEMP", "TMP", "SystemRoot", "SYSTEMROOT", "WINDIR")
    result = {key: os.environ[key] for key in names if key in os.environ}
    result.update({"OMP_NUM_THREADS": "1", "OPENBLAS_NUM_THREADS": "1", "MKL_NUM_THREADS": "1", "VECLIB_MAXIMUM_THREADS": "1"})
    return result


def bounded(command, *, seconds=30, output=4 * 1024 * 1024, cleanup=5):
    return run_process(command, cwd=ROOT, env=environment(), timeout_seconds=seconds,
                       output_limit_bytes=output, cleanup_seconds=cleanup)


def successful(record):
    return (record["exit_code"] == 0 and not record["timed_out"] and not record["output_limited"]
            and record["cleanup_ok"] and record["launch_error"] is None)


def command_output(command):
    record = bounded(command)
    if not successful(record):
        raise ValueError(f"preflight command failed: {command!r}: {record!r}")
    return record["stdout"].strip()


def git(*arguments):
    executable = shutil.which("git")
    if not executable:
        raise ValueError("git required to prove frozen source/criteria/corpus inputs")
    return command_output([executable, "--no-replace-objects", *arguments])


def frozen(path, revision):
    relative = path.resolve().relative_to(ROOT).as_posix()
    committed = git("rev-parse", f"{revision}:{relative}")
    actual = git("hash-object", "--", str(path.resolve()))
    if committed != actual:
        raise ValueError(f"input is not frozen at {revision}: {relative}")
    return committed


def policy_input(path, revision):
    blob = frozen(path, revision)
    content = path.read_bytes()
    return {"base64": base64.b64encode(content).decode("ascii"), "sha256": hashlib.sha256(content).hexdigest(),
            "git_blob": blob, "revision": revision, "path": path.resolve().relative_to(ROOT).as_posix()}


def fixtures(revision):
    result, identities = [], []
    for manifest_path in MANIFESTS:
        frozen(manifest_path, revision)
        manifest = load(manifest_path)
        if manifest.get("fixture_profile_id") not in CRITERIA["corpora"] or manifest.get("license") != "Apache-2.0":
            raise ValueError("wrong licensed corpus profile")
        identities.append({"profile": manifest["fixture_profile_id"], "sha256": sha256(manifest_path)})
        for image in manifest["images"]:
            relative = Path(image["file"])
            if relative.is_absolute() or ".." in relative.parts:
                raise ValueError("unsafe fixture path")
            source = (manifest_path.parent / relative).resolve(strict=True)
            source.relative_to(manifest_path.parent.resolve())
            frozen(source, revision)
            if sha256(source) != image["sha256"]:
                raise ValueError(f"fixture digest mismatch: {source}")
            if not all(isinstance(image[key], int) and 0 < image[key] <= 16384 for key in ("width", "height")):
                raise ValueError("invalid fixture dimensions")
            for order, region in enumerate(image["regions"]):
                if region.get("order") != order or not region.get("text_nfc") or not region.get("id"):
                    raise ValueError("invalid frozen region oracle/order")
            result.append({"id": f"{manifest['fixture_profile_id']}/{relative.stem}",
                           "file": str(source), "sha256": image["sha256"], "byte_len": source.stat().st_size,
                           "width": image["width"], "height": image["height"], "regions": image["regions"]})
    frozen(ROOT / "fixtures/ocr/compact-ui-v1/generate.py", revision)
    frozen(CRITERIA_PATH, revision)
    return result, identities


def actual_host():
    if sys.platform == "darwin":
        cpu = command_output(["/usr/sbin/sysctl", "-n", "machdep.cpu.brand_string"])
        return "aarch64-apple-darwin" if platform.machine() == "arm64" else "unsupported-apple-target", f"macOS {platform.mac_ver()[0]}", cpu
    if sys.platform == "win32":
        return "x86_64-pc-windows-msvc" if platform.machine().upper() in ("AMD64", "X86_64") else "unsupported-windows-target", platform.platform(), platform.processor()
    return platform.machine() + "-unsupported-" + sys.platform, platform.platform(), platform.processor() or platform.machine()


def actual_binding(args, fixture_path, revision, build):
    target, os_name, cpu = actual_host()
    rustc = shutil.which("rustc")
    if not rustc:
        raise ValueError("rustc is required to record the explicitly selected build toolchain")
    return {"source_revision": git("rev-parse", "HEAD"), "source_tree": git("rev-parse", "HEAD^{tree}"),
            "executable_sha256": sha256(args.executable), "runtime_sha256": sha256(args.runtime),
            "catalog_sha256": sha256(CATALOG_PATH), "criteria_sha256": sha256(CRITERIA_PATH),
            "fixtures_sha256": sha256(fixture_path), "compiler": command_output([rustc, "-Vv"]),
            "opencv": build.get("opencv"), **{key: build.get(key) for key in BUILD_SOURCE_FIELDS},
            "target": target, "os": os_name, "cpu": cpu, "host_id": platform.node(),
            "cohort_id": args.cohort_id or "diagnostic-unqualified", "runtime": "1.29.0-api17", "provider": "cpu",
            "intra_op": 1, "inter_op": 1, "qualified_host": False, "host_approval": None,
            "frozen_revision": revision, "serial_execution": True}


def verify_resources(identity, root):
    for resource in identity["resources"]:
        relative = Path(resource["relative_path"])
        if relative.is_absolute() or ".." in relative.parts:
            raise ValueError("unsafe catalog resource path")
        path = (root / relative).resolve(strict=True)
        path.relative_to(root)
        if path.stat().st_size != resource["bytes"] or sha256(path) != resource["sha256"]:
            raise ValueError(f"resource identity mismatch: {resource['relative_path']}")


@contextlib.contextmanager
def serial_lock():
    name = hashlib.sha256(str(ROOT).encode()).hexdigest()[:24]
    with (Path(tempfile.gettempdir()) / f"mado-ocr-evaluation-{name}.lock").open("a+b") as stream:
        if sys.platform == "win32":
            import msvcrt
            stream.seek(0); stream.write(b"0"); stream.flush(); stream.seek(0)
            try:
                msvcrt.locking(stream.fileno(), msvcrt.LK_NBLCK, 1)
            except OSError as error:
                raise ValueError("another OCR evaluation campaign owns the serial lock") from error
            try:
                yield
            finally:
                stream.seek(0); msvcrt.locking(stream.fileno(), msvcrt.LK_UNLCK, 1)
        else:
            import fcntl
            try:
                fcntl.flock(stream.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
            except OSError as error:
                raise ValueError("another OCR evaluation campaign owns the serial lock") from error
            try:
                yield
            finally:
                fcntl.flock(stream.fileno(), fcntl.LOCK_UN)


def campaign(args):
    args.report.mkdir(parents=True, exist_ok=False)
    report = {"schema_version": 2, "criteria_id": CRITERIA["criteria_id"], "diagnostic": args.diagnostic,
              "reference_profile": args.reference_profile, "candidate_profile": args.profile,
              "profiles": {}, "identities": {}, "preflight_failures": [], "commands": []}
    report["reproduction_command"] = [sys.executable, *sys.argv]
    report["process_limits"] = {"timeout_seconds": args.timeout_seconds, "output_limit_bytes": args.output_limit_bytes,
                                "cleanup_seconds": args.cleanup_seconds, "lifecycle_bound_ms": args.lifecycle_bound_ms}
    destination = args.report / "report.json"
    fatal = []
    try:
        revision = git("rev-parse", f"{args.frozen_revision}^{{commit}}")
        rows, manifests = fixtures(revision)
        report["fixtures"], report["corpora"] = rows, manifests
        fixture_path = args.report / "fixtures.json"
        save(fixture_path, {"fixtures": rows})
        described_build = None
        for profile in (args.reference_profile, args.profile):
            command = [str(args.executable), "--model-evaluation", "--describe-profile", "--profile", profile]
            raw = bounded(command)
            report["commands"].append({"command": command, "process": raw})
            if not successful(raw):
                raise ValueError(f"compiled descriptor unavailable: {profile}")
            description = strict_json(raw["stdout"])
            build = description["build"]
            if build["executable_sha256"] != sha256(args.executable) or build["catalog_sha256"] != sha256(CATALOG_PATH):
                raise ValueError("compiled descriptor/source catalog binding mismatch")
            if described_build is not None and any(described_build.get(key) != build.get(key) for key in BUILD_FIELDS):
                raise ValueError("inconsistent compiled build provenance between profiles")
            described_build = build
            report["identities"][profile] = description["identity"]
        observed = actual_binding(args, fixture_path, revision, described_build)
        binding = load(args.binding) if args.binding else observed
        if not args.binding and not args.diagnostic:
            fatal.append("a complete evaluation requires an exact pre-execution --binding")
        for key, value in observed.items():
            if key not in ("qualified_host", "host_approval", "cohort_id") and binding.get(key) != value:
                fatal.append(f"actual host/artifact binding mismatch: {key}")
        if args.cohort_id is not None and binding.get("cohort_id") != args.cohort_id:
            fatal.append("--cohort-id differs from recorded binding")
        report["binding"] = binding
        budgets = load(args.budgets) if args.budgets else None
        if budgets is not None:
            # The commit is external to the file it identifies; no self-referential hash.
            budgets["frozen_revision"] = revision
        report["budgets"] = budgets
        if report["process_limits"] != (budgets or {}).get("process_limits"):
            report["preflight_failures"].append("process deadline/output/cleanup bounds differ from frozen policy")
        report["policy_inputs"] = {}
        workload_ids = [f"{row['id']}/{mode}" for row in rows for mode in ("singular", "grouped")]
        report["preflight_failures"].extend(validate_binding(binding))
        report["preflight_failures"].extend(validate_budgets(budgets, binding, workload_ids))
        dirty = git("status", "--porcelain", "--untracked-files=all", "--ignore-submodules=none")
        if dirty:
            report["preflight_failures"].append("implementation source has uncommitted changes")
            report["source_changes"] = dirty
        if args.budgets:
            report["policy_inputs"]["budgets"] = policy_input(args.budgets, revision)
            if not successful(bounded([shutil.which("git"), "merge-base", "--is-ancestor", revision, binding["source_revision"]])):
                fatal.append("budget/corpus freeze is not an ancestor of implementation source")
            report["budget_sha256"] = sha256(args.budgets)
        if args.host_policy:
            report["policy_inputs"]["host_policy"] = policy_input(args.host_policy, revision)
            policy = load(args.host_policy)
            report["host_policy"] = policy
            expected_cohort = {key: binding.get(key) for key in COHORT_FIELDS}
            if not any(row.get("cohort") == expected_cohort and row.get("approval") == binding.get("host_approval") for row in policy.get("approved_cohorts", [])):
                report["preflight_failures"].append("host cohort not in frozen release allowlist")
            report["host_policy_sha256"] = sha256(args.host_policy)
        else:
            report["preflight_failures"].append("missing frozen --host-policy release allowlist")
        for profile, identity in report["identities"].items():
            root = args.reference_root if profile == args.reference_profile else args.model_root
            verify_resources(identity, root)
        report["preflight_failures"].extend(fatal)
        if args.preflight_only or fatal or (report["preflight_failures"] and not args.diagnostic):
            report["execution"] = "refused" if report["preflight_failures"] else "preflight-only"
        else:
            binding_path = args.report / "binding.json"
            save(binding_path, binding)
            # A diagnostic run keeps every unmet release prerequisite and is never a complete evaluation.
            for profile in (args.reference_profile, args.profile):
                report["profiles"][profile] = []
                root = args.reference_root if profile == args.reference_profile else args.model_root
                for index in range(3):
                    raw_path = args.report / f"{profile}-{index}.jsonl"
                    command = [str(args.executable), "--model-evaluation", "--profile", profile,
                               "--model-root", str(root), "--runtime", str(args.runtime),
                               "--fixtures", str(fixture_path), "--binding", str(binding_path),
                               "--report", str(raw_path), "--process-index", str(index),
                               "--output-limit-bytes", str(args.output_limit_bytes),
                               "--lifecycle-bound-ms", str(args.lifecycle_bound_ms)]
                    operational = bounded(command, seconds=args.timeout_seconds, output=args.output_limit_bytes, cleanup=args.cleanup_seconds)
                    # Preserve the parent's independent outcome before parsing child observations.
                    receipt_path = args.report / f"{profile}-{index}.process.json"
                    save(receipt_path, {"index": index, "process": operational, "command": command})
                    process = parse_process(raw_path, operational, command, index, evidence_root=args.report)
                    process.update(receipt_path=receipt_path.name, receipt_sha256=sha256(receipt_path))
                    report["profiles"][profile].append(process)
                    if operational["cleanup_ok"] is not True:
                        report["preflight_failures"].append("owned child cleanup failed; later launches withheld")
                        break
                if report["profiles"][profile] and report["profiles"][profile][-1]["process"]["cleanup_ok"] is not True:
                    break
            report["execution"] = "diagnostic" if args.diagnostic else "evaluation"
            for name, path in (("executable_sha256", args.executable), ("runtime_sha256", args.runtime),
                               ("criteria_sha256", CRITERIA_PATH), ("catalog_sha256", CATALOG_PATH),
                               ("fixtures_sha256", fixture_path)):
                if sha256(path) != binding.get(name):
                    report["preflight_failures"].append(f"input changed during campaign: {name}")
            for profile, identity in report["identities"].items():
                verify_resources(identity, args.reference_root if profile == args.reference_profile else args.model_root)
            for row in rows:
                if sha256(Path(row["file"])) != row["sha256"] or Path(row["file"]).stat().st_size != row["byte_len"]:
                    report["preflight_failures"].append(f"fixture changed during campaign: {row['id']}")
            for identity, manifest in zip(manifests, MANIFESTS):
                if sha256(manifest) != identity["sha256"]:
                    report["preflight_failures"].append(f"corpus manifest changed during campaign: {identity['profile']}")
            if git("rev-parse", "HEAD") != binding["source_revision"] or git("rev-parse", "HEAD^{tree}") != binding["source_tree"]:
                report["preflight_failures"].append("implementation revision changed during campaign")
            if git("status", "--porcelain", "--untracked-files=all", "--ignore-submodules=none") != dirty:
                report["preflight_failures"].append("implementation source changed during campaign")
    except (OSError, ValueError, KeyError, TypeError) as error:
        report["preflight_failures"].append(f"preflight/apparatus failure: {error}")
        report["execution"] = "refused"
    if args.diagnostic:
        report["preflight_failures"].append("explicit diagnostic execution is never a complete evaluation")
    report["evaluation"] = evaluation_bundle([evaluate(report, evidence_root=args.report, repository_root=ROOT)])
    save(destination, report)
    outcomes = {profile: {rule["rule_id"]: rule["outcome"] for rule in entry["rules"]}
                for scope in report["evaluation"]["evaluations"] for profile, entry in scope["profiles"].items()}
    print(json.dumps({"report": str(destination), "execution": report["execution"],
                      "complete": report["evaluation"]["complete"],
                      "validation_errors": sum(len(scope["validation_errors"]) for scope in report["evaluation"]["evaluations"]),
                      "outcomes": outcomes}, ensure_ascii=False))
    return 0 if report["evaluation"]["complete"] else 2


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    subcommands = parser.add_subparsers(dest="command", required=True)
    run = subcommands.add_parser("run", help="preflight and run three fresh Rust processes per profile")
    run.add_argument("--executable", type=Path, required=True, help="isolated release onnx-cpu binary, benchmark-instrumentation enabled")
    run.add_argument("--model-root", type=Path, required=True)
    run.add_argument("--reference-root", type=Path, help="defaults to --model-root")
    run.add_argument("--runtime", type=Path, required=True)
    run.add_argument("--reference-profile", required=True, help="declared profile in the comparison reference role")
    run.add_argument("--profile", required=True, help="declared profile compared against the reference")
    run.add_argument("--frozen-revision", required=True, help="commit containing frozen corpus, criteria, budgets and host policy")
    run.add_argument("--binding", type=Path, help="exact source/artifact/cohort identities recorded before execution; required for a complete evaluation")
    run.add_argument("--budgets", type=Path, help="approved precommitted cohort-specific absolute ceilings; no defaults")
    run.add_argument("--host-policy", type=Path, help="precommitted approved_cohorts allowlist; no inferred host approval")
    run.add_argument("--cohort-id", help="must match a supplied binding; diagnostic default is explicitly unqualified")
    run.add_argument("--report", type=Path, required=True, help="new evidence directory; never overwritten")
    run.add_argument("--diagnostic", action="store_true", help="permit known missing host/budget prerequisites, retain failures, never a complete evaluation")
    run.add_argument("--preflight-only", action="store_true", help="do not initialize models or run inference")
    run.add_argument("--timeout-seconds", type=float, required=True, help="predeclared per-process external hard deadline")
    run.add_argument("--output-limit-bytes", type=int, required=True, help="predeclared combined stdout/stderr byte bound")
    run.add_argument("--cleanup-seconds", type=float, required=True, help="predeclared owned-child cleanup deadline")
    run.add_argument("--lifecycle-bound-ms", type=int, required=True, help="predeclared admitted cancellation/deadline return bound")
    evaluation = subcommands.add_parser("evaluate", help="recompute per-rule outcomes from retained reports; never changes catalog or model availability")
    evaluation.add_argument("reports", nargs="+", type=Path)
    evaluation.add_argument("--evidence-roots", nargs="+", type=Path, help="original JSONL directories in report order; defaults to report parents")
    evaluation.add_argument("--repository", type=Path, default=ROOT, help="repository retaining original source/policy Git objects")
    args = parser.parse_args()
    if args.command == "evaluate":
        roots = args.evidence_roots or [path.parent for path in args.reports]
        if len(roots) != len(args.reports):
            parser.error("--evidence-roots needs exactly one directory per report")
        bundle = evaluate_reports(args.reports, evidence_roots=roots, repository_root=args.repository)
        sys.stdout.buffer.write(document_bytes(bundle))
        sys.stdout.flush()
        # Failed quality/numerical rules are results; only incomplete, invalid or technical failures fail.
        return 0 if bundle["complete"] else 2
    if args.reference_profile == args.profile:
        parser.error("--reference-profile and --profile must differ")
    for key in ("executable", "model_root", "runtime"):
        setattr(args, key, getattr(args, key).resolve())
    args.reference_root = (args.reference_root or args.model_root).resolve()
    args.report = args.report.resolve()
    if not all(value > 0 and value < float("inf") for value in (args.timeout_seconds, args.output_limit_bytes, args.cleanup_seconds, args.lifecycle_bound_ms)):
        parser.error("deadline, output and cleanup bounds must be positive finite numbers")
    with serial_lock():
        return campaign(args)


if __name__ == "__main__":
    raise SystemExit(main())
