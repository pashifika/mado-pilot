"""Actual-consumer Apple dyld closure and source-bound process ownership.

This module never launches a process or upgrades process_runner's CPU record.
It reuses macos_image_observer's parser, Mach-O/file bindings and cache identity.
Discovery is data, not approval; every qualifying invocation supplies its own
root and two named child transports, plus pre/post bindings of the frozen union.

Review references are {path, sha256}. A source-owned-processes-v1 certificate
binds context_sha256/target/reviewer/rationale, source_files and
roles[baseline|candidate]. Each role binds case, executables, two spawn_sites
(source_path/line/parent_role/child_role/maximum_calls), terminal_source_files,
application_process_cap=3, witness_contract, native_dependency_evidence, and the
exact compiler_identity for replay. Source files bind their complete bytes plus
inclusive UTF-8 lines normalized to one trailing LF per line. Native proofs bind
case/context/artifacts/reachable source sections and must have no open residuals;
they do not assert anything about unreachable library functions or CPU rollup.
"""
from __future__ import annotations

import hashlib
import os
from pathlib import Path
import shutil
import stat
import time

import macos_image_observer as images

APPLE = "aarch64-apple-darwin"
WITNESS_TRIAL = "saved-rgba-terminal-reap-v1"
WITNESS_REPLAY = "host-finish-awaited-node-v1"
MIB = 1024 * 1024


class IdentityError(ValueError):
    category = "identity"


def require(condition, reason, *, identity=False):
    if not condition:
        raise (IdentityError if identity else ValueError)(reason)


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def execution_binding(plan):
    keys = ("case", "target", "host", "configuration", "root", "env", "inputs", "roles",
            "collector", "bounds", "fixture", "loader", "campaign")
    return images.canonical_sha256({key: plan[key] for key in keys})


def process_roles(plan):
    return ("root", "capability", "recognition") if plan["case"].startswith("trial-") else ("root", "engine", "node")


def executables(plan, role):
    selected = plan["roles"][role]
    if plan["case"].startswith("trial-"):
        return {"root": selected["root_binary"], "capability": selected["engine"], "recognition": selected["engine"]}
    require(isinstance(selected.get("node"), dict), "explicit actual Node executable pin required")
    node = selected["node"]
    require(plan["inputs"].get(node.get("path"), {}).get("sha256") == node.get("sha256"), "Node executable absent from guarded inputs")
    return {"root": selected["root_binary"], "engine": selected["engine"], "node": node}


def check_node_resolution(plan, role):
    if plan["case"].startswith("trial-"):
        return None
    node = executables(plan, role)["node"]
    resolved = shutil.which("node", path=plan["env"].get("PATH", ""))
    require(resolved is not None and os.path.samefile(resolved, node["path"]), "actual PATH-resolved Node differs from frozen executable")
    require(digest(resolved) == node["sha256"], "PATH-resolved Node digest changed")
    return {"resolved": resolved, "pinned": node["path"], "sha256": node["sha256"]}


def system_snapshot():
    return {"system": images._check_system(images._system_identity()),
            "dyld": images._check_dyld(images._dyld_identity())}


def parser_record(plan, output):
    report = output["runs"][0]
    if plan["case"].startswith("deadline-"):
        return report["observations"]["measurement_import_parser"]
    return report["metrics"]["runtime"]["source_diagnostic"]["import_parser"]


def streams(plan, role, record, output, sink_root):
    require(plan["target"] == APPLE and plan["loader"] is True, "Apple actual-invocation loader-on required")
    require(plan["env"].get("DYLD_PRINT_LIBRARIES") == "1", "root loader environment missing")
    expected = executables(plan, role)
    sources = {"root": {"pid": record["process_id"], "text": record["stderr"], "location": "record.stderr"}}
    if "recognition" in expected:
        view = output["operation"]["controller"]
        require(len(view["attempts"]) == 1, "one actual capability record required")
        reports = {"capability": (view["attempts"][0]["child_pid"], view["attempts"][0]["stderr_observation"]),
                   "recognition": (view["result"]["child_pid"], view["result"]["stderr_observation"])}
    else:
        report = output["runs"][0]
        starts = [row for row in report["milestones"] if row["event"] == "ChildStarted"]
        require(len(starts) == 1, "one actual engine PID required")
        node = parser_record(plan, output)["loader"]
        require(node["node_pid"] == node["child_pid"], "actual compiler PID mismatch")
        reports = {"engine": (starts[0]["pid"], report["metrics"]["stderr_observation"]),
                   "node": (node["node_pid"], node)}
    sink_root = Path(sink_root)
    paths = set()
    for name, (pid, report) in reports.items():
        require(report["loader_enabled"] is True and report["child_pid"] == pid, "named child transport PID mismatch")
        require(report["method"] == images.METHOD and report["complete"] is True
                and report["eof"] is True and report["read_error"] is None
                and report["read_limit_reached"] is False and report["truncated"] is False
                and report["unterminated_tail"] is False and report["zero_loader_lines"] is False,
                "incomplete named child transport")
        require(all(report[key] == 0 for key in ("foreign_lines", "interleaved", "malformed_prefixes", "oversized_lines",
                                                "foreign_pid_entries_dropped"))
                and report["foreign_pids"] == [], "lost/foreign child loader records")
        sink = report["sink"]
        path = Path(sink["path"])
        require(path.parent == sink_root and not path.is_symlink() and stat.S_ISREG(path.lstat().st_mode), "foreign/nonregular child sink")
        require(sink["truncated"] is False and sink["write_error"] is None
                and 0 < sink["limit"] <= plan["bounds"]["child_sink_bytes"]
                and path.stat().st_size == sink["bytes_written"] == report["bytes"] <= sink["limit"],
                "child sink byte retention incomplete")
        payload = path.read_bytes()
        require(report["bytes_total"] == len(payload) + report["application_bytes_total"]
                and report["application_bytes_total"] == report["application_bytes_retained"],
                "stderr byte accounting or retained application diagnostics incomplete")
        text = payload.decode("utf-8", "strict")
        require(text.endswith("\n") and len(text.splitlines()) == report["lines"], "child sink line retention mismatch")
        paths.add(path)
        sources[name] = {"pid": pid, "text": text, "location": str(path), "sha256": hashlib.sha256(payload).hexdigest(), "bytes": len(payload)}
    require(set(sink_root.iterdir()) == paths, "unreported child sink")
    require(len({source["pid"] for source in sources.values()}) == 3, "three directly witnessed process roles required", identity=True)
    for name, source in sources.items():
        require(type(source["pid"]) is int and source["pid"] > 0, "missing actual process PID")
        parsed = images._parse_log(source["text"])
        require(not parsed["foreign_pids"], "foreign PID in named role loader records", identity=True)
        require(parsed["reason"] is None and parsed["application_stderr_lines"] == 0,
                "loader records incomplete: " + str(parsed["reason"]))
        require(parsed["pid"] == source["pid"], "foreign PID in named role loader records", identity=True)
        require(parsed["images"] and all(row["uuid"] is not None for row in parsed["images"]), "missing loaded-image UUID")
        source["parsed"] = parsed
        source.setdefault("sha256", hashlib.sha256(source["text"].encode()).hexdigest())
        source.setdefault("bytes", len(source["text"].encode()))
    return sources


def discover(plan, role, record, output, sink_root, before, deadline):
    require(record["cleanup_ok"] is True and record["timed_out"] is False and record["output_limited"] is False
            and record["launch_error"] is None and record["exit_code"] == int(plan["case"].startswith("deadline-")), "discovery invocation failed")
    sources = streams(plan, role, record, output, sink_root)
    expected = executables(plan, role)
    budget = {"bytes": images.MAX_HASH_BYTES}
    processes = {}
    for name, source in sources.items():
        parsed = source["parsed"]
        require(os.path.samefile(parsed["main_executable"], expected[name]["path"]), "observed process executable differs from selected role")
        rows, membership = [], {}
        for item in parsed["images"]:
            klass = images._image_class(item["path"])
            if klass == "system":
                membership[item["path"]] = True  # _image_class consulted dyld shared-cache membership, not just the prefix.
                row = {**item, "class": klass, "bound": True}
            else:
                row = {**item, "class": klass, **images._bind_file(item["path"], item["uuid"], deadline, budget)}
            rows.append(images._check_image_binding(row))
        require(rows[0]["class"] == "file" and rows[0]["sha256"] == expected[name]["sha256"], "actual executable hash mismatch")
        processes[name] = {"pid": source["pid"], "main_executable": parsed["main_executable"], "images": rows,
                           "cache_membership": membership, "raw_sha256": source["sha256"], "raw_bytes": source["bytes"],
                           "events": len(parsed["events"])}
    after = system_snapshot()
    require(before == after, "system cache/dyld changed during discovery")
    return {"kind": "consumer-runtime-discovery-v1", "execution_binding": execution_binding(plan),
            "role": role, "observer": images.method_identity(), "system": before, "processes": processes,
            "node_resolution": check_node_resolution(plan, role), "qualification": False}


def validate_discovery(plan, discovery, record, output, sink_root):
    require(discovery["kind"] == "consumer-runtime-discovery-v1" and discovery["execution_binding"] == execution_binding(plan)
            and discovery["observer"] == images.method_identity(), "discovery source/execution binding mismatch")
    role = discovery["role"]
    sources = streams(plan, role, record, output, sink_root)
    expected = executables(plan, role)
    require(set(discovery["processes"]) == set(sources), "discovery role coverage mismatch")
    images._check_system(discovery["system"]["system"])
    images._check_dyld(discovery["system"]["dyld"])
    for name, source in sources.items():
        retained = discovery["processes"][name]
        require(retained["pid"] == source["pid"] and retained["raw_sha256"] == source["sha256"]
                and retained["raw_bytes"] == source["bytes"] and retained["events"] == len(source["parsed"]["events"]), "discovery raw transport changed")
        rows = [images._check_image_binding(dict(row, bound=True)) for row in retained["images"]]
        require([{key: row[key] for key in ("path", "uuid")} for row in rows] == source["parsed"]["images"], "discovery images differ from raw log")
        require(retained["main_executable"] == source["parsed"]["main_executable"] == rows[0]["path"]
                and rows[0]["class"] == "file" and rows[0]["sha256"] == expected[name]["sha256"], "discovery executable binding mismatch")
        require(retained["cache_membership"] == {row["path"]: True for row in rows if row["class"] == "system"}, "cache membership proof incomplete")
    return discovery


def closure_from(plan, discoveries):
    require(0 < len(discoveries) <= 12, "finite explicit discovery set required")
    require({row["role"] for row in discoveries} == set(plan["roles"]), "every selected source role needs actual discovery")
    common = discoveries[0]["system"]
    roles = {}
    for discovery in discoveries:
        require(discovery["execution_binding"] == execution_binding(plan) and discovery["system"] == common, "mixed discovery source/host context")
        processes = roles.setdefault(discovery["role"], {})
        for name, observed in discovery["processes"].items():
            process = processes.setdefault(name, {"main_executable": observed["main_executable"], "images": []})
            require(process["main_executable"] == observed["main_executable"], "process executable path changed")
            rows = {row["path"]: row for row in process["images"]}
            for row in observed["images"]:
                require(row["path"] not in rows or rows[row["path"]] == row, "discovery image binding drift")
                if row["path"] not in rows:
                    process["images"].append(row)
                    rows[row["path"]] = row
    return {"kind": "consumer-apple-closure-v1", "execution_binding": execution_binding(plan),
            "observer": images.method_identity(), "system": common, "roles": roles, "qualification": False}


def validate_discovery_raw(plan, discovery, raw):
    require(discovery.get("issues") == [], "failed discovery cannot define runtime closure")
    require(raw["execution_binding"] == execution_binding(plan) and raw["role"] == discovery["role"],
            "discovery raw execution binding mismatch")
    require(raw["execution_limits"] == {
        "timeout_seconds": plan["bounds"]["primary_seconds"],
        "cleanup_seconds": plan["bounds"]["cleanup_seconds"],
        "output_limit_bytes": plan["bounds"]["output_limit_bytes"]}, "discovery enforcing envelope mismatch")


def validate_bundle(plan, bundle, read_bound, validate_semantics):
    require(plan["target"] == APPLE and plan["loader"] is True, "Windows/loader-off closure is unavailable")
    require(bundle.get("kind") == "consumer-apple-closure-v1" and bundle.get("execution_binding") == execution_binding(plan), "runtime closure execution binding mismatch")
    origins = bundle.get("origins")
    require(isinstance(origins, list) and 0 < len(origins) <= 12, "actual discovery origins required")
    discoveries = []
    for ref in origins:
        discovery = read_bound(ref)
        raw = read_bound(discovery["raw"], 25 * MIB)
        validate_discovery_raw(plan, discovery, raw)
        output = validate_semantics(plan, discovery["role"], raw["record"])
        discoveries.append(validate_discovery(plan, discovery, raw["record"], output, discovery["sink_root"]))
    derived = closure_from(plan, discoveries)
    require(set(bundle) == set(derived) | {"origins", "freeze"} and all(bundle[key] == value for key, value in derived.items()), "closure is not the union of actual retained discovery roles")
    require(set(bundle["freeze"]) == set(plan["roles"]), "closure freeze role coverage incomplete")
    for role, value in bundle["freeze"].items():
        validate_snapshot(plan, bundle, role, value)
    return bundle


def all_rows(bundle, role):
    rows = {}
    for process in bundle["roles"][role].values():
        for row in process["images"]:
            require(row["path"] not in rows or rows[row["path"]] == row, "conflicting per-role image bindings")
            rows[row["path"]] = row
    return list(rows.values())


def snapshot(plan, bundle, role, deadline):
    if time.monotonic() > deadline:
        raise TimeoutError("runtime binding deadline")
    require(system_snapshot() == bundle["system"], "runtime system shared-cache/dyld drift", identity=True)
    rows = all_rows(bundle, role)
    membership = {row["path"]: images._cache_contains_path(row["path"]) for row in rows if row["class"] == "system"}
    require(all(membership.values()), "frozen system image is not in current verified shared cache")
    bindings, reason = images._recheck_files(rows, deadline, "consumer")
    if reason is not None:
        if "deadline" in reason:
            raise TimeoutError(reason)
        raise IdentityError(reason)
    node = check_node_resolution(plan, role)
    if time.monotonic() > deadline:
        raise TimeoutError("runtime binding deadline")
    return {"system": bundle["system"], "bindings": bindings, "cache_membership": membership,
            "node_resolution": node}


def validate_snapshot(plan, bundle, role, value):
    require(value["system"] == bundle["system"], "retained cache/dyld binding differs")
    rows = all_rows(bundle, role)
    images._check_bindings(value["bindings"], rows, "consumer")
    require(value["cache_membership"] == {row["path"]: True for row in rows if row["class"] == "system"}, "cache membership was not established")
    if "node" in process_roles(plan):
        node = executables(plan, role)["node"]
        require(value["node_resolution"]["pinned"] == node["path"] and value["node_resolution"]["sha256"] == node["sha256"], "actual Node resolution binding missing")


def observe(plan, bundle, role, record, output, sink_root, observation):
    require(observation["closure_sha256"] == images.canonical_sha256(bundle), "attempt closure binding mismatch")
    validate_snapshot(plan, bundle, role, observation["pre"])
    validate_snapshot(plan, bundle, role, observation["post"])
    sources = streams(plan, role, record, output, sink_root)
    processes = {}
    require(set(sources) == set(bundle["roles"][role]), "actual invocation role coverage differs")
    for name, source in sources.items():
        frozen = bundle["roles"][role][name]
        require(source["parsed"]["main_executable"] == frozen["main_executable"], "actual role executable changed", identity=True)
        rows = {(row["path"], row["uuid"]): row for row in frozen["images"]}
        observed, unknown = images._reconcile(source["parsed"]["images"], rows)
        require(not unknown, "unknown loaded image: " + str(unknown), identity=True)
        processes[name] = {"pid": source["pid"], "raw_sha256": source["sha256"], "raw_bytes": source["bytes"],
                           "image_indexes": [row["closure_index"] for row in observed], "events": len(source["parsed"]["events"])}
    return {"status": "complete", "method": "actual-consumer-dyld-on-v1", "processes": processes,
            "closure_sha256": observation["closure_sha256"], "scope": "three source-bound owned application roles; no inferred OS process identities"}


def source_files(rows, inputs=None):
    """Full files and inclusive UTF-8 LF section hashes are independently checked."""
    require(isinstance(rows, list) and rows, "complete source-file evidence required")
    seen = set()
    for row in rows:
        path = row["path"]
        require(path not in seen and Path(path).is_absolute(), "duplicate/relative source binding")
        seen.add(path)
        if inputs is not None:
            require(inputs.get(path, {}).get("sha256") == row["sha256"], "certificate source absent from frozen inputs: " + path)
        require(Path(path).stat().st_size <= 16 * MIB and digest(path) == row["sha256"], "certificate originating file changed: " + path)
        lines = Path(path).read_text(encoding="utf-8").splitlines()
        require(isinstance(row.get("sections"), list) and row["sections"], "source witness sections required")
        for section in row["sections"]:
            start, end = section["start_line"], section["end_line"]
            require(type(start) is int and type(end) is int and 1 <= start <= end <= len(lines), "source section bounds invalid")
            payload = "".join(line + "\n" for line in lines[start - 1:end]).encode()
            require(hashlib.sha256(payload).hexdigest() == section["sha256"]
                    and isinstance(section.get("purpose"), str) and section["purpose"].strip(), "source witness section mismatch")
    return seen


def reviewed(value, kind, plan, context):
    require(value.get("kind") == kind and value.get("accepted") is True, "accepted " + kind + " required")
    require(value.get("context_sha256") == context and value.get("target") == plan["target"], "certificate context/target drift")
    require(all(isinstance(value.get(key), str) and value[key].strip() for key in ("reviewer", "rationale")), "named reviewer and rationale required")


def validate_process_certificate(plan, certificate, bundle, context, read_bound):
    reviewed(certificate, "source-owned-processes-v1", plan, context)
    require(plan["target"] == APPLE, "source-bound process ownership available only for Apple")
    require(plan["configuration"]["benchmark_accounting"] is False, "native proof requires normal allocation configuration")
    require(not any((key.startswith("OPENCV_") or key.startswith("ORT_")) and key != "ORT_DISABLE_TELEMETRY"
                    for key in plan["env"]), "native source proof excludes OpenCV/ORT environment overrides")
    require(certificate.get("scope") == {
        "owned_tree_claim": "all descendants of the selected application root",
        "excluded": ["out-of-process system services", "manual injection"]},
        "owned-process scope must not exclude native-created descendants")
    bound_sources = source_files(certificate["source_files"], plan["inputs"])
    require(set(certificate["roles"]) == set(plan["roles"]), "every selected source role needs its own certificate")
    for role, selected in certificate["roles"].items():
        argv = plan["roles"][role]["argv"]
        path = argv[8] if plan["case"].startswith("trial-") else argv[2]
        configuration = read_bound({"path": path, "sha256": plan["inputs"][path]["sha256"]})
        provider = configuration["provider"] if plan["case"].startswith("trial-") else configuration["native_config"]["ocr"]["provider"]
        require(provider == "cpu", "native selected-path proof requires actual source-bound CPU provider")
        required_executables = {name: {key: value[key] for key in ("path", "sha256")}
                                for name, value in executables(plan, role).items()}
        require(selected["executables"] == required_executables, "source certificate executable mismatch")
        expected_witness = WITNESS_TRIAL if plan["case"].startswith("trial-") else WITNESS_REPLAY
        require(selected["case"] == plan["case"] and selected["witness_contract"] == expected_witness
                and selected["application_process_cap"] == 3, "case-local source witness contract mismatch")
        sites = selected["spawn_sites"]
        require(isinstance(sites, list) and len(sites) == 2
                and {site["child_role"] for site in sites} == set(process_roles(plan)) - {"root"}, "finite selected spawn inventory incomplete")
        for site in sites:
            parent = "engine" if site["child_role"] == "node" else "root"
            require(site["parent_role"] == parent and site["maximum_calls"] == 1
                    and site["source_path"] in bound_sources, "spawn/ancestor chain is not source bound")
            source = next(row for row in certificate["source_files"] if row["path"] == site["source_path"])
            require(type(site["line"]) is int and any(section["start_line"] <= site["line"] <= section["end_line"]
                    for section in source["sections"]), "spawn site is outside reviewed source sections")
        require(isinstance(selected.get("terminal_source_files"), list) and selected["terminal_source_files"]
                and set(selected["terminal_source_files"]) <= bound_sources, "reap/stream-join/exit source evidence missing")
        # Native-linked code is part of the owned-tree claim. Logging alone cannot
        # establish that an opaque library never forks a silent/unawaited child.
        artifacts = {(row["path"], row["sha256"]) for row in all_rows(bundle, role) if row["class"] == "file"}
        covered = set()
        proofs = selected.get("native_dependency_evidence")
        require(isinstance(proofs, list) and proofs, "native/producer process-closure evidence missing")
        for ref in proofs:
            proof = read_bound(ref)
            require(proof.get("kind") == "native-process-closure-evidence-v1"
                    and proof.get("case") == plan["case"] and proof.get("context_sha256") == context
                    and proof.get("claim") == "no-unwitnessed-owned-descendants"
                    and proof.get("residuals") == []
                    and all(isinstance(proof.get(key), str) and proof[key].strip() for key in ("reviewer", "rationale")),
                    "native process-closure evidence is not case/source reviewed")
            source_files(proof["source_files"])
            covered.update((row["path"], row["sha256"]) for row in proof["artifacts"])
        require(artifacts <= covered, "unproved native/producer artifact process scope: " + str(sorted(artifacts - covered)))
        if expected_witness == WITNESS_REPLAY:
            require(isinstance(selected.get("compiler_identity"), dict) and selected["compiler_identity"],
                    "explicit successful compiler inventory identity required")
    return certificate


def terminal_witness(plan, role, certificate, record, output, closure):
    require(closure.get("status") == "complete" and set(closure["processes"]) == set(process_roles(plan)), "actual role/image witnesses incomplete")
    require(record.get("cleanup_ok") is True and record.get("timed_out") is False
            and record.get("output_limited") is False and record.get("launch_error") is None
            and record.get("exit_code") == int(plan["case"].startswith("deadline-")), "root wait/terminal witness unavailable")
    require(closure["processes"]["root"]["pid"] == record["process_id"], "root wait PID differs from actual loader PID")
    if plan["case"].startswith("trial-"):
        operation = output["operation"]
        view = operation["controller"]
        require(output["success"] is True and output["status"] == "semantic_pass" and output["apparatus_failures"] == []
                and operation["accepted"] is True and operation["terminal_observed"] is True
                and operation["requires_parent_owned_tree_cleanup"] is False and operation["cancellation_requested"] is False
                and operation["shutdown"] == {"completed": True, "error": None} and view["state"] == "terminal"
                and len(view["attempts"]) == 1, "trial accepted terminal path not witnessed")
        for name, child, operation_name in (("capability", view["attempts"][0], "recognition_capabilities"),
                                             ("recognition", view["result"], "recognition_trial")):
            require(child["operation"] == operation_name and child["child_pid"] == closure["processes"][name]["pid"]
                    and child["forced"] is False and child["exit_code"] == 0 and child["cleanup"]["clean"] is True,
                    "trial child was not normally reaped")
            parent = child["supervision_observation"]
            require(parent["epoch"] == "parent-control.started" and parent["unit"] == "ns"
                    and all(type(parent.get(key)) is int and parent[key] >= 0 for key in
                            ("terminal_received_ns", "root_wait_returned_ns", "streams_joined_ns"))
                    and max(parent["terminal_received_ns"], parent["root_wait_returned_ns"]) <= parent["streams_joined_ns"],
                    "actual wait-return/stream-join witnesses missing")
    else:
        child = output["runs"][0]
        require(child["forced"] is False and child["exit_code"] == 0 and child["cleanup"]["clean"] is True
                and child["observations"]["terminal_accounted"] is True
                and child["status"] == ("FAIL" if plan["case"].startswith("deadline-") else "PASS"),
                "engine normal exit/settlement witness unavailable")
        # This is the actual retained successful compile result, not EOF or the
        # constant child_reaped field. Exact source review proves its awaited path.
        parser = parser_record(plan, output)
        require(parser["identity"] == certificate["roles"][role]["compiler_identity"]
                and parser["loader"]["node_pid"] == closure["processes"]["node"]["pid"],
                "awaited compiler return/source identity missing")
    return {"status": "complete", "application_process_cap": 3,
            "roles": {name: value["pid"] for name, value in closure["processes"].items()},
            "witness_contract": certificate["roles"][role]["witness_contract"]}
