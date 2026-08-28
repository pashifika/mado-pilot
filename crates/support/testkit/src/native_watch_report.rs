//! Strict tracked-output schema for native template-watch qualification.
//!
//! Raw harness output remains ignored ephemera. Only a profile whose free-form
//! fields equal this module's fixed public text and whose host/provenance tokens
//! pass the bounded validators may be copied into tracked evidence.

use crate::bench_harness::Profile;

/// Exact fixture description accepted in a tracked native watcher profile.
pub const FIXTURE_DESCRIPTION: &str =
    "repository native watch fixture plus generated watch-marker-v1";
/// Exact build command accepted in a tracked native watcher profile.
pub const BUILD_PROFILE: &str = "cargo build --locked --release --package mado-pilot --bench native-template-watch --features native-template-watch-qualification";
/// Exact semantic oracle description accepted in a tracked profile.
pub const CORRECTNESS_ORACLE: &str =
    "24 ordered source/match/geometry/work/lifecycle/ownership/resource/privacy hard gates";
/// Exact finite queue description accepted in a tracked profile.
pub const QUEUE_POLICY: &str = "production finite latest-wins capture and template-watch scheduler";

/// Exact ordered workload registry accepted by native watcher profiles.
pub const WORKLOADS: [&str; 24] = [
    "environment_identity",
    "window_absent_current",
    "window_transient_appearance",
    "window_persistent_appearance",
    "window_disappearance_reset",
    "window_strictly_newer",
    "window_move",
    "window_resize",
    "window_topology_scale",
    "display_current_newer",
    "permission_availability",
    "native_high_rate_slow_backend",
    "two_query_fairness",
    "two_session_fairness",
    "exact_coalescing",
    "unequal_no_coalescing",
    "queue_expiry_overload",
    "stale_generation",
    "wait_cancel_deadline",
    "native_stop_target_loss",
    "session_engine_close",
    "retained_result_mapping",
    "fresh_session",
    "producer_progress_cleanup_privacy",
];

/// Workloads with the accepted three-warmup, twenty-sample latency profile.
pub const SAMPLED_WORKLOADS: [&str; 16] = [
    "window_absent_current",
    "window_transient_appearance",
    "window_persistent_appearance",
    "window_disappearance_reset",
    "window_strictly_newer",
    "window_move",
    "window_resize",
    "native_high_rate_slow_backend",
    "two_query_fairness",
    "two_session_fairness",
    "exact_coalescing",
    "unequal_no_coalescing",
    "stale_generation",
    "wait_cancel_deadline",
    "retained_result_mapping",
    "fresh_session",
];

/// Revision-bound non-sensitive provenance for one process aggregate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Provenance<'a> {
    /// Exact 40-character source revision.
    pub source: &'a str,
    /// Exact 40-character source tree.
    pub tree: &'a str,
    /// Exact SHA-256 over the frozen fixture source inventory.
    pub fixture_source: &'a str,
    /// Exact allowlisted backend identity and runtime version.
    pub backend: &'a str,
    /// Exact allowlisted Rust/native toolchain identity.
    pub toolchain: &'a str,
    /// Allowlisted approved host class.
    pub host: &'a str,
    /// `precursor` or `final`.
    pub cohort: &'a str,
    /// One-based process index represented without a native PID.
    pub process_index: &'a str,
}

/// Why a candidate tracked aggregate is not safe to retain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivacyFault {
    /// A fixed schema field was changed or an identity was malformed.
    Schema,
    /// A bounded host string contained a payload-like or path-like value.
    Payload,
}

/// Validates the complete string-bearing portion of one tracked process profile.
///
/// Numeric workload metrics are emitted by the typed benchmark harness and need
/// no string scanner. This function closes every string-bearing field before the
/// report can be retained.
pub fn validate(profile: &Profile, provenance: Provenance<'_>) -> Result<(), PrivacyFault> {
    if profile.fixture != FIXTURE_DESCRIPTION
        || profile.build_profile != BUILD_PROFILE
        || profile.correctness_oracle != CORRECTNESS_ORACLE
        || profile.queue_policy != QUEUE_POLICY
        || !is_hex(provenance.source, 40)
        || !is_hex(provenance.tree, 40)
        || !is_hex(provenance.fixture_source, 64)
        || provenance.backend != "opencv-4.14.0"
        || !matches!(
            provenance.toolchain,
            "rust-1.97.1-8bab26f4-llvm-22.1.6" | "rust-1.97.1-msvc-19.44.35228"
        )
        || !matches!(
            provenance.host,
            "apple-m1-pro-10c-32g" | "windows-i7-12700kf-32g"
        )
        || !matches!(provenance.cohort, "precursor" | "final")
        || provenance.process_index.is_empty()
        || provenance.process_index.len() > 2
        || !provenance
            .process_index
            .bytes()
            .all(|byte| byte.is_ascii_digit())
    {
        return Err(PrivacyFault::Schema);
    }
    let expected_notes = format!(
        "source {}; tree {}; fixture-source {}; backend {}; toolchain {}; host {}; cohort {}; process {}; control native-watch-control-v1",
        provenance.source,
        provenance.tree,
        provenance.fixture_source,
        provenance.backend,
        provenance.toolchain,
        provenance.host,
        provenance.cohort,
        provenance.process_index,
    );
    if profile.notes.as_deref() != Some(expected_notes.as_str())
        || !safe_host_field(&profile.hardware)
        || !safe_host_field(&profile.os_version)
        || profile.deployment_target.as_deref().is_none_or(|target| {
            !matches!(target, "aarch64-apple-darwin" | "x86_64-pc-windows-msvc")
        })
        || !is_hex(&profile.fixture_sha256, 64)
        || profile
            .benchmark_executable_sha256
            .as_deref()
            .is_none_or(|digest| !is_hex(digest, 64))
    {
        return Err(PrivacyFault::Payload);
    }
    Ok(())
}

fn is_hex(value: &str, length: usize) -> bool {
    value.len() == length && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn safe_host_field(value: &str) -> bool {
    const FORBIDDEN: [&str; 15] = [
        "captured_pixel",
        "pixel_hash",
        "template_bytes",
        "template_id",
        "window_id",
        "display_id",
        "process_id",
        "window_title",
        "local_path",
        "credential",
        "password",
        "ocr_text",
        "input_text",
        "process_inventory",
        "environment=",
    ];
    let lowercase = value.to_ascii_lowercase();
    !value.is_empty()
        && value.len() <= 160
        && !value.contains('\n')
        && !value.contains('\r')
        && !value.contains('/')
        && !value.contains('\\')
        && !value.contains("..")
        && FORBIDDEN.iter().all(|term| !lowercase.contains(term))
}

#[cfg(test)]
mod tests {
    use super::{
        BUILD_PROFILE, CORRECTNESS_ORACLE, FIXTURE_DESCRIPTION, PrivacyFault, Provenance,
        QUEUE_POLICY, validate,
    };
    use crate::bench_harness::Profile;

    const SOURCE: &str = "0123456789abcdef0123456789abcdef01234567";
    const TREE: &str = "89abcdef0123456789abcdef0123456789abcdef";
    const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn provenance() -> Provenance<'static> {
        Provenance {
            source: SOURCE,
            tree: TREE,
            fixture_source: DIGEST,
            backend: "opencv-4.14.0",
            toolchain: "rust-1.97.1-8bab26f4-llvm-22.1.6",
            host: "apple-m1-pro-10c-32g",
            cohort: "precursor",
            process_index: "1",
        }
    }

    fn profile() -> Profile {
        Profile {
            fixture: FIXTURE_DESCRIPTION.to_owned(),
            fixture_sha256: DIGEST.to_owned(),
            benchmark_executable_sha256: Some(DIGEST.to_owned()),
            hardware: "Apple M1 Pro 10 cores 34359738368 bytes".to_owned(),
            os_version: "macOS 26.5.2 build 25F84 SDK 26.5 OpenCV 4.14.0".to_owned(),
            deployment_target: Some("aarch64-apple-darwin".to_owned()),
            build_profile: BUILD_PROFILE.to_owned(),
            correctness_oracle: CORRECTNESS_ORACLE,
            queue_policy: QUEUE_POLICY,
            notes: Some(format!(
                "source {SOURCE}; tree {TREE}; fixture-source {DIGEST}; backend opencv-4.14.0; toolchain rust-1.97.1-8bab26f4-llvm-22.1.6; host apple-m1-pro-10c-32g; cohort precursor; process 1; control native-watch-control-v1"
            )),
        }
    }

    #[test]
    fn the_exact_allowlisted_profile_is_accepted() {
        assert_eq!(validate(&profile(), provenance()), Ok(()));
    }

    #[test]
    fn payload_categories_cannot_enter_host_or_os_fields() {
        for payload in [
            "captured_pixel_hash=abc",
            "template_bytes=abc",
            "template_id=secret",
            "window_title=private",
            "window_id=42 display_id=7 process_id=9",
            "/Users/person/project/model.onnx",
            r"C:\\Users\\person\\fixture.exe",
            "credential=password",
            "ocr_text=private input_text=private",
            "process_inventory=all",
            "environment=TOKEN",
        ] {
            let mut candidate = profile();
            candidate.hardware = payload.to_owned();
            assert_eq!(
                validate(&candidate, provenance()),
                Err(PrivacyFault::Payload)
            );
        }
    }

    #[test]
    fn notes_and_fixed_fields_cannot_be_extended_with_free_form_diagnostics() {
        let mut candidate = profile();
        candidate
            .notes
            .as_mut()
            .expect("notes")
            .push_str("; native_error=private");
        assert_eq!(
            validate(&candidate, provenance()),
            Err(PrivacyFault::Payload)
        );

        let mut candidate = profile();
        candidate.fixture.push_str(" at /tmp/private");
        assert_eq!(
            validate(&candidate, provenance()),
            Err(PrivacyFault::Schema)
        );
    }
}
