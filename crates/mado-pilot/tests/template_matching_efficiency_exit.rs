//! Exercise the benchmark producer's exit predicate without native workloads.

#![cfg(feature = "template-matching-timing")]
#![recursion_limit = "256"]

use benchmark::{parse_arguments, report_failed};
use serde_json::{Value, json};

#[allow(dead_code, unreachable_pub, unused_imports)]
#[path = "../benches/template-matching-efficiency.rs"]
mod benchmark;

#[cfg(test)]
mod exit_tests {
    use super::*;

    fn report() -> Value {
        let threads = json!({"framework": "fixture", "threads": 1, "reason": null});
        json!({
            "kind": "observations",
            "windows_loader_observer_enabled": false,
            "workloads": [{
                "oracle_failures": 0,
                "windows_loader_observer": null,
                "metrics": {"cpu_ns": null, "cpu_reason": "CPU unavailable"},
                "retained_loop_cpu": {
                    "start_ns": null, "end_ns": null, "cpu_ns": null,
                    "reason": "retained-loop process CPU unavailable or nonmonotonic"
                }
            }],
            "opencv_parallel_threads": {"before": threads, "after": threads, "changed": false}
        })
    }

    #[test]
    fn optional_cpu_is_nonfatal_only_for_smoke() {
        let mut value = report();
        assert!(!report_failed(&value, true));
        assert!(report_failed(&value, false));
        value["workloads"][0]["retained_loop_cpu"] =
            json!({"start_ns": 20, "end_ns": 10, "cpu_ns": null, "reason": "nonmonotonic"});
        assert!(!report_failed(&value, true));
        assert!(report_failed(&value, false));
        value["workloads"][0]["retained_loop_cpu"] =
            json!({"start_ns": 10, "end_ns": 20, "cpu_ns": 10, "reason": null});
        assert!(!report_failed(&value, true));
        assert!(!report_failed(&value, false));
    }

    #[test]
    fn malformed_cpu_accounting_never_passes_smoke() {
        for cpu in [
            json!({"start_ns": null, "end_ns": null, "cpu_ns": null, "reason": null}),
            json!({"start_ns": "unknown", "end_ns": null, "cpu_ns": null, "reason": "missing"}),
            json!({"start_ns": 10, "end_ns": 20, "cpu_ns": null, "reason": "hidden observation"}),
            json!({"start_ns": 10, "end_ns": 20, "cpu_ns": "10", "reason": "invalid"}),
            json!({"start_ns": 10, "end_ns": 20, "cpu_ns": 11, "reason": null}),
            json!({"start_ns": 10, "end_ns": 20, "cpu_ns": 10}),
            json!({"start_ns": 10, "end_ns": 20, "cpu_ns": 10, "reason": "contradictory"}),
            json!({"cpu_ns": null, "reason": "endpoints absent"}),
        ] {
            let mut value = report();
            value["workloads"][0]["retained_loop_cpu"] = cpu;
            assert!(report_failed(&value, true));
            assert!(report_failed(&value, false));
        }
        let mut value = report();
        value["workloads"][0]["retained_loop_cpu"] =
            json!({"start_ns": 10, "end_ns": 20, "cpu_ns": 10, "reason": null});
        value["workloads"][0]["metrics"]["cpu_ns"] = json!(11);
        assert!(report_failed(&value, true));
    }

    #[test]
    fn optional_cpu_cannot_hide_oracle_owner_or_thread_failures() {
        for error in ["setup_error", "owner_error"] {
            let mut value = report();
            value["workloads"][0][error] = json!("failed");
            value["workloads"][0]["oracle_failures"] = json!(1);
            assert!(report_failed(&value, true));
        }
        let mut value = report();
        value["workloads"][0]["metrics"]["final_owner_correct"] = json!(false);
        value["workloads"][0]["oracle_failures"] = json!(1);
        assert!(report_failed(&value, true));
        let mut value = report();
        value["opencv_parallel_threads"]["changed"] = json!(true);
        assert!(report_failed(&value, true));
        let mut value = report();
        value["workloads"][0]["oracle_failures"] = Value::Null;
        assert!(report_failed(&value, true));
    }

    fn counter(sequence: u64) -> Value {
        json!({
            "first": sequence, "last": sequence, "incomplete_bits": 0,
            "pending_sequences": [], "clean": true
        })
    }

    fn observer_report(statistical: bool) -> Value {
        let id = if statistical {
            "single-small96x64"
        } else {
            "contract-different-frame"
        };
        let module = json!({"base": 4096, "size": 8192, "path_utf16_hex": "0063003a005c0061"});
        let census = json!({
            "before": counter(0), "after": counter(0), "needed_bytes": 8,
            "failure": null, "modules": [module]
        });
        let file = json!({
            "volume_serial": 1, "file_id_hex": "00000000000000000000000000000001",
            "final_nt_path_utf16_hex": "005c0061", "creation_time": 1, "last_access_time": 2,
            "last_write_time": 3, "change_time": 4, "attributes": 32, "allocation_size": 8192,
            "file_size": 8192, "links": 1, "delete_pending": 0, "directory": 0
        });
        let pe = json!({
            "machine": 34404, "timestamp": 1, "characteristics": 34, "magic": 523,
            "entry_rva": 4096, "preferred_base": 4096, "image_size": 8192,
            "headers_size": 512, "checksum": 0, "subsystem": 3, "dll_characteristics": 352,
            "sections": [{
                "virtual_size": 4096, "rva": 4096, "raw_size": 512,
                "raw_offset": 512, "characteristics": 1610612768
            }],
            "debug_entries": [], "codeview_status": "ABSENT", "codeview": []
        });
        let bridges: Vec<_> = [("initial", 1), ("begin", 3), ("end", 5)]
            .into_iter()
            .map(|(stage, index)| {
                json!({
                    "stage": stage, "census_index": index, "module_index": 0, "base": 4096,
                    "mapped_nt_path_utf16_hex": "005c0061",
                    "mapped_nt_path_after_utf16_hex": "005c0061",
                    "file_before": file, "file_after": file, "memory_pe": pe, "file_pe": pe,
                    "file_before_complete": true, "file_after_complete": true,
                    "memory_pe_complete": true, "file_pe_complete": true,
                    "failure": null, "close_success": true
                })
            })
            .collect();
        let cleanup: Vec<_> = (1..=3)
            .map(|handle| {
                json!({
                    "operation": "CloseHandle", "handle": handle, "success": true, "failure": null
                })
            })
            .collect();
        let observer = json!({
            "protocol": "mado-component-windows-loader-v1",
            "method": "windows-ldr-observer-on-warm-v1",
            "pid": 73, "workload_id": id, "statistical": statistical,
            "complete": true, "window_unchanged": true, "primary_failure": null,
            "registration": {
                "module": 4096, "export": 4352, "ntstatus": 0, "cookie": 8192,
                "unregister_called": false
            },
            "cuts": {"initial": [0, 1], "begin": [2, 3], "end": [4, 5]},
            "cut_counters": {"initial": counter(0), "begin": counter(0), "end": counter(0)},
            "censuses": vec![census; 6], "bridges": bridges, "cleanup": cleanup,
            "window_counter": {"begin": counter(0), "end": counter(0)},
            "final_counter": counter(0), "snapshot_n": 0, "events": [],
            "records_complete": true, "records_complete_scope": "snapshot_n_only",
            "final_owned_cleanup": {"ok": true, "remaining_owned_dll": 0, "failed_close_handles": []},
            "read_bytes": 3072, "read_calls": 6, "census_path_units": 24,
            "total_sections_reserved": 6, "total_debug_entries_reserved": 0,
            "limits": {
                "modules": 128, "events": 256, "name_utf16_units": 1024, "censuses": 20,
                "total_census_path_units": 131072, "image_file_bytes": 536870912,
                "header_bytes": 65536, "sections": 96, "debug_entries": 32,
                "total_sections": 4096, "total_debug_entries": 4096, "codeview_bytes": 4096,
                "read_bytes": 67108864, "read_calls": 32768, "bridges": 129, "stdout_bytes": 8388608
            },
            "claim": "conditional_observed_window_and_path_header_correlation_only",
            "qualification": false, "backing_object_proof": false, "code_bytes_proof": false,
            "callback_lifetime": "process_exit_no_unregister_no_drain",
            "owned_cleanup_scope": "file_handles_only", "outer_process_cleanup": "supervisor_only",
            "serialization_exit_observed": false
        });
        let mut value = report();
        value["target"] = json!("x86_64-pc-windows-msvc");
        value["process_id"] = json!(73);
        value["timing_boundaries_enabled"] = json!(true);
        value["windows_loader_observer_enabled"] = json!(true);
        value["windows_loader_observer"] = json!({
            "protocol": "mado-component-windows-loader-v1",
            "callback": {"rva": 4096, "statics": {
                "sequence": {"rva": 8192, "bytes": 8},
                "incomplete": {"rva": 8200, "bytes": 4},
                "records": {"rva": 8208, "bytes": 1058816}
            }}
        });
        value["workloads"][0]["id"] = json!(id);
        value["workloads"][0]["statistical"] = json!(statistical);
        value["workloads"][0]["timing_boundary_error"] = Value::Null;
        value["workloads"][0]["windows_loader_observer"] = observer;
        value
    }

    fn changed_window(value: &mut Value) {
        let observer = &mut value["workloads"][0]["windows_loader_observer"];
        observer["window_counter"]["end"] = counter(2);
        observer["cut_counters"]["end"] = counter(2);
        observer["final_counter"] = counter(2);
        observer["snapshot_n"] = json!(2);
        for index in [4, 5] {
            observer["censuses"][index]["before"] = counter(2);
            observer["censuses"][index]["after"] = counter(2);
        }
        observer["window_unchanged"] = json!(false);
        observer["events"] = json!([
            {"sequence": 1, "complete": true, "reason": 2, "flags": 0,
             "base": 4096, "size": 8192, "invalid_bits": 0,
             "full_name_utf16_hex": "0063003a005c0061", "base_name_utf16_hex": "0061"},
            {"sequence": 2, "complete": true, "reason": 1, "flags": 0,
             "base": 4096, "size": 8192, "invalid_bits": 0,
             "full_name_utf16_hex": "0063003a005c0061", "base_name_utf16_hex": "0061"}
        ]);
    }

    #[test]
    fn observer_cli_requires_explicit_supported_single_row_scope() {
        let parse =
            |args: &[&str]| parse_arguments(args.iter().map(|argument| (*argument).to_owned()));
        for args in [
            vec![],
            vec!["--list"],
            vec!["--timing-boundaries", "--workload", "single-small96x64"],
        ] {
            assert!(parse(&args).is_ok());
        }
        for args in [
            vec!["--windows-loader-observer"],
            vec!["--windows-loader-observer", "--timing-boundaries"],
            vec![
                "--windows-loader-observer",
                "--workload",
                "single-small96x64",
            ],
            vec![
                "--windows-loader-observer",
                "--timing-boundaries",
                "--workload",
                "unknown",
            ],
            vec![
                "--windows-loader-observer",
                "--timing-boundaries",
                "--workload",
                "single-small96x64",
                "--workload",
                "single-small96x64",
            ],
        ] {
            assert!(parse(&args).is_err());
        }
        let supported = cfg!(all(
            target_os = "windows",
            target_arch = "x86_64",
            target_env = "msvc"
        ));
        for args in [
            vec!["--windows-loader-observer", "--list"],
            vec![
                "--windows-loader-observer",
                "--timing-boundaries",
                "--workload",
                "single-small96x64",
            ],
        ] {
            let parsed = parse(&args);
            assert_eq!(parsed.is_ok(), supported);
            if !supported {
                assert_eq!(
                    parsed.err().as_deref(),
                    Some("--windows-loader-observer requires x86_64-pc-windows-msvc")
                );
            }
        }
        assert!(
            parse(&[
                "--workload",
                "contract-different-frame",
                "--warmups",
                "1",
                "--samples",
                "3"
            ])
            .is_err()
        );
        assert!(
            parse(&[
                "--workload",
                "single-small96x64",
                "--warmups",
                "1",
                "--samples",
                "3"
            ])
            .is_ok()
        );
    }

    #[test]
    fn observer_off_and_manifest_never_accept_observation_payloads() {
        let mut value = observer_report(true);
        value["windows_loader_observer_enabled"] = json!(false);
        assert!(report_failed(&value, true));
        value["workloads"][0]["windows_loader_observer"] = Value::Null;
        assert!(!report_failed(&value, true));
        value["target"] = json!("aarch64-apple-darwin");
        value["windows_loader_observer"] = Value::Null;
        assert!(!report_failed(&value, true));

        let mut value = observer_report(true);
        value["kind"] = json!("manifest");
        assert!(report_failed(&value, true));
        value["workloads"][0]["windows_loader_observer"] = Value::Null;
        value["timing_boundaries_enabled"] = json!(false);
        assert!(!report_failed(&value, true));
        value["target"] = json!("aarch64-apple-darwin");
        assert!(report_failed(&value, true));
    }

    #[test]
    fn observer_missing_or_malformed_never_passes_smoke_or_contract() {
        for statistical in [false, true] {
            let value = observer_report(statistical);
            assert!(!report_failed(&value, true));
            for replacement in [Value::Null, json!({}), json!(true), json!([])] {
                let mut invalid = value.clone();
                invalid["workloads"][0]["windows_loader_observer"] = replacement;
                assert!(report_failed(&invalid, true));
            }
            for field in [
                "protocol",
                "method",
                "pid",
                "workload_id",
                "statistical",
                "complete",
                "primary_failure",
                "records_complete",
                "registration",
                "cuts",
                "censuses",
                "bridges",
                "events",
                "window_counter",
                "final_counter",
                "snapshot_n",
                "cleanup",
                "final_owned_cleanup",
                "read_bytes",
                "read_calls",
                "limits",
                "claim",
                "qualification",
                "backing_object_proof",
                "code_bytes_proof",
                "records_complete_scope",
                "callback_lifetime",
                "serialization_exit_observed",
                "owned_cleanup_scope",
                "outer_process_cleanup",
                "cut_counters",
            ] {
                let mut invalid = value.clone();
                invalid["workloads"][0]["windows_loader_observer"]
                    .as_object_mut()
                    .unwrap()
                    .remove(field);
                assert!(report_failed(&invalid, true), "{field}");
            }
            let mut invalid = value.clone();
            invalid["workloads"][0]
                .as_object_mut()
                .unwrap()
                .remove("windows_loader_observer");
            assert!(report_failed(&invalid, true));
            let mut invalid = value.clone();
            invalid["workloads"][0]["windows_loader_observer"]["complete"] = json!("true");
            assert!(report_failed(&invalid, true));
        }
    }

    #[test]
    fn observer_binds_pid_row_policy_and_enabled_envelope() {
        for (pointer, replacement) in [
            ("/process_id", json!(74)),
            ("/windows_loader_observer_enabled", Value::Null),
            ("/target", json!("aarch64-apple-darwin")),
            ("/timing_boundaries_enabled", json!(false)),
            (
                "/windows_loader_observer/protocol",
                json!("mado-component-timing-v1"),
            ),
            (
                "/windows_loader_observer/callback/statics/records/bytes",
                json!(1058815),
            ),
            (
                "/windows_loader_observer/callback/rva",
                json!(u64::from(u32::MAX) + 1),
            ),
            ("/workloads/0/id", json!("contract-different-frame")),
            ("/workloads/0/statistical", json!(false)),
            ("/workloads/0/windows_loader_observer/pid", json!(74)),
            (
                "/workloads/0/windows_loader_observer/workload_id",
                json!("contract-different-frame"),
            ),
            (
                "/workloads/0/windows_loader_observer/statistical",
                json!(false),
            ),
        ] {
            let mut value = observer_report(true);
            *value.pointer_mut(pointer).unwrap() = replacement;
            assert!(report_failed(&value, true), "{pointer}");
        }
        let mut value = observer_report(true);
        value["workloads"][0]["statistical"] = json!(false);
        value["workloads"][0]["windows_loader_observer"]["statistical"] = json!(false);
        assert!(
            report_failed(&value, true),
            "authored statistical policy cannot be downgraded"
        );
        let mut value = observer_report(true);
        let duplicate = value["workloads"][0].clone();
        value["workloads"].as_array_mut().unwrap().push(duplicate);
        assert!(report_failed(&value, true));
        value["workloads"] = json!([]);
        assert!(report_failed(&value, true));
    }

    #[test]
    fn observer_refuses_torn_missing_mismatched_or_overflowed_cuts() {
        for (pointer, replacement) in [
            ("/cuts/initial", Value::Null),
            ("/cuts/begin", json!([0, 1])),
            ("/cuts/end", json!([5, 4])),
            ("/cut_counters/initial", Value::Null),
            ("/cut_counters/begin", counter(1)),
            ("/cut_counters/end/pending_sequences", json!([1])),
            ("/censuses/2/before/first", json!(1)),
            ("/censuses/3/after/pending_sequences", json!([1])),
            ("/censuses/4/after/incomplete_bits", json!(1)),
            ("/censuses/5/before/clean", json!(false)),
            ("/censuses/0/needed_bytes", json!(16)),
            ("/censuses/1/modules/0/base", json!(8192)),
            ("/window_counter/begin/last", json!(1)),
            ("/window_counter/end/first", json!(257)),
            ("/final_counter/last", json!(1)),
            ("/snapshot_n", json!(1)),
            ("/records_complete", json!(false)),
            ("/window_unchanged", json!(false)),
            ("/bridges/0/census_index", json!(0)),
            ("/bridges/1/module_index", json!(1)),
            ("/bridges/2/stage", json!("begin")),
            ("/bridges/0/close_success", json!(false)),
            ("/bridges/0/memory_pe", Value::Null),
            ("/cleanup/0/success", json!(false)),
            ("/final_owned_cleanup/ok", json!(false)),
            ("/read_bytes", json!(67108865)),
            ("/limits/bridges", json!(384)),
            ("/limits/censuses", json!(6)),
            ("/registration/unregister_called", json!(true)),
            ("/registration/ntstatus", json!(-1)),
            ("/registration/ntstatus", json!(1)),
        ] {
            for statistical in [false, true] {
                let mut value = observer_report(statistical);
                *value["workloads"][0]["windows_loader_observer"]
                    .pointer_mut(pointer)
                    .unwrap() = replacement.clone();
                assert!(report_failed(&value, true), "{pointer}");
            }
        }
        let mut value = observer_report(false);
        value["workloads"][0]["windows_loader_observer"]["bridges"]
            .as_array_mut()
            .unwrap()
            .pop();
        assert!(report_failed(&value, true));
    }

    #[test]
    fn observer_refuses_partial_bridge_objects_despite_complete_summary() {
        for key in [
            "file_before_complete",
            "file_after_complete",
            "memory_pe_complete",
            "file_pe_complete",
        ] {
            for replacement in [Value::Null, json!(false), json!("true")] {
                let mut value = observer_report(false);
                value["workloads"][0]["windows_loader_observer"]["bridges"][0][key] = replacement;
                assert!(report_failed(&value, true), "{key}");
            }
            let mut value = observer_report(false);
            value["workloads"][0]["windows_loader_observer"]["bridges"][0]
                .as_object_mut()
                .unwrap()
                .remove(key);
            assert!(report_failed(&value, true), "{key}");
        }
    }

    #[test]
    fn observer_distinguishes_census_terminator_capacity_from_counted_native_names() {
        for (units, valid) in [(1023, true), (1024, false)] {
            let mut value = observer_report(true);
            for census in value["workloads"][0]["windows_loader_observer"]["censuses"]
                .as_array_mut()
                .unwrap()
            {
                census["modules"][0]["path_utf16_hex"] = json!("0061".repeat(units));
            }
            assert_eq!(!report_failed(&value, true), valid);
        }
        for (units, valid) in [(1024, true), (1025, false)] {
            for key in ["full_name_utf16_hex", "base_name_utf16_hex"] {
                let mut value = observer_report(false);
                changed_window(&mut value);
                value["workloads"][0]["windows_loader_observer"]["events"][0][key] =
                    json!("0061".repeat(units));
                assert_eq!(!report_failed(&value, true), valid, "{key}");
            }
            let mut value = observer_report(true);
            let bridge = &mut value["workloads"][0]["windows_loader_observer"]["bridges"][0];
            bridge["mapped_nt_path_utf16_hex"] = json!("0061".repeat(units));
            bridge["mapped_nt_path_after_utf16_hex"] = bridge["mapped_nt_path_utf16_hex"].clone();
            assert_eq!(!report_failed(&value, true), valid);
        }
    }

    #[test]
    fn changed_contract_window_is_not_a_qualification_or_identity_claim() {
        let mut contract = observer_report(false);
        changed_window(&mut contract);
        assert!(!report_failed(&contract, true));
        let mut statistical = observer_report(true);
        changed_window(&mut statistical);
        assert!(report_failed(&statistical, true));
        for field in [
            "qualification",
            "backing_object_proof",
            "code_bytes_proof",
            "serialization_exit_observed",
        ] {
            let mut value = contract.clone();
            value["workloads"][0]["windows_loader_observer"][field] = json!(true);
            assert!(report_failed(&value, true), "{field}");
        }
        for (field, replacement) in [
            ("claim", "backing_object_identity"),
            ("records_complete_scope", "final_drain"),
            ("callback_lifetime", "unregistered"),
            ("owned_cleanup_scope", "all_process_resources"),
            ("outer_process_cleanup", "observer_drained"),
        ] {
            let mut value = contract.clone();
            value["workloads"][0]["windows_loader_observer"][field] = json!(replacement);
            assert!(report_failed(&value, true), "{field}");
        }
        for (field, replacement) in [
            ("complete", json!(false)),
            ("sequence", json!(3)),
            ("invalid_bits", json!(1)),
            ("flags", json!(1)),
            ("full_name_utf16_hex", json!("torn")),
        ] {
            let mut value = contract.clone();
            value["workloads"][0]["windows_loader_observer"]["events"][0][field] = replacement;
            assert!(report_failed(&value, true), "{field}");
        }
    }

    #[test]
    fn observer_failures_retain_raw_details_without_waiving_cpu_or_oracles() {
        for operation in ["register", "begin_census", "end_census"] {
            let mut value = observer_report(false);
            let failure = json!({"operation": operation, "raw_code": 5, "detail": 129});
            value["workloads"][0]["windows_loader_observer"]["primary_failure"] = failure.clone();
            value["workloads"][0]["windows_loader_observer"]["complete"] = json!(false);
            assert!(report_failed(&value, true));
            assert_eq!(
                value["workloads"][0]["windows_loader_observer"]["primary_failure"],
                failure
            );
        }
        let mut value = observer_report(true);
        assert!(!report_failed(&value, true));
        assert!(report_failed(&value, false));
        value["workloads"][0]["retained_loop_cpu"] =
            json!({"start_ns": 10, "end_ns": 20, "cpu_ns": 10, "reason": null});
        assert!(!report_failed(&value, false));
        value["workloads"][0]["timing_boundary_error"] = json!("short marker write");
        assert!(report_failed(&value, true));
        value["workloads"][0]["timing_boundary_error"] = Value::Null;
        value["workloads"][0]["oracle_failures"] = json!(1);
        assert!(report_failed(&value, true));
    }

    #[test]
    fn late_runtime_diagnostic_failure_cannot_hide_retained_observation() {
        let mut value = observer_report(false);
        let retained = value["workloads"][0]["windows_loader_observer"].clone();
        value["loaded_native_runtime"] = Value::Null;
        value["error"] = json!("loaded module path unavailable or truncated");
        assert!(report_failed(&value, true));
        assert_eq!(value["workloads"][0]["windows_loader_observer"], retained);
    }
}
