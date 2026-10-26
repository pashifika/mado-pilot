//! Exercise the benchmark producer's exit predicate without native workloads.

#![cfg(feature = "template-matching-timing")]

use benchmark::report_failed;
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
            "workloads": [{
                "oracle_failures": 0,
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
}
