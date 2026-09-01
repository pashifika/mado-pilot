//! Strict tracked-output schema and environment policy for native template-watch qualification.
//!
//! Raw harness output remains ignored ephemera. Only a profile whose free-form
//! fields satisfy the bounded privacy schema and whose typed environment matches
//! an approved target may be copied into tracked evidence.

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

const APPLE_HARDWARE: &str = "Apple M1 Pro, 10 CPU cores, 34359738368 bytes memory";
const APPLE_PRECURSOR_OS: &str = "macOS 26.5.2 build 25F84, SDK 26.5";
const APPLE_FINAL_OS: &str = "macOS 26.6.2 build 25G83, SDK 26.5";
const WINDOWS_HARDWARE_LEGACY: &str = "Intel Core i7-12700KF, 32 GiB";
const WINDOWS_HARDWARE_PREFIX: &str = "Intel Core i7-12700KF, ";
const WINDOWS_HARDWARE_BYTES_SUFFIX: &str = " bytes memory";
const WINDOWS_SUPPORTED_SDK: [u32; 4] = [10, 0, 26100, 0];
const WINDOWS_VERSION_MAX_BYTES: usize = 128;

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
    /// Canonical process index; `0` is reserved for disposable pre-formal runs.
    pub process_index: &'a str,
}

/// A privacy-safe environment that is not approved for qualification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvironmentFault {
    /// The bounded host class is not the approved class.
    HostClass,
    /// The bounded backend identity is not approved.
    Backend,
    /// The bounded toolchain identity is not approved for the host.
    Toolchain,
    /// The bounded hardware description disagrees with the host family.
    Hardware,
    /// The bounded operating-system family is not approved.
    OperatingSystem,
    /// The bounded SDK identity is not approved.
    Sdk,
    /// The bounded deployment target is not approved.
    Target,
}

/// Why a candidate tracked aggregate cannot be retained as green evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationError {
    /// A fixed report field or canonical identity violates the schema.
    Schema,
    /// A string-bearing field is not bounded privacy-safe provenance.
    Privacy,
    /// Every field is privacy-safe, but the environment is unsupported.
    Environment(EnvironmentFault),
}

impl ValidationError {
    /// Fixed bounded token retained by the qualification driver.
    pub const fn token(self) -> &'static str {
        match self {
            Self::Schema => "schema_violation",
            Self::Privacy => "privacy_violation",
            Self::Environment(EnvironmentFault::HostClass) => "environment_incompatible:host_class",
            Self::Environment(EnvironmentFault::Backend) => "environment_incompatible:backend",
            Self::Environment(EnvironmentFault::Toolchain) => "environment_incompatible:toolchain",
            Self::Environment(EnvironmentFault::Hardware) => "environment_incompatible:hardware",
            Self::Environment(EnvironmentFault::OperatingSystem) => {
                "environment_incompatible:operating_system"
            }
            Self::Environment(EnvironmentFault::Sdk) => "environment_incompatible:sdk",
            Self::Environment(EnvironmentFault::Target) => "environment_incompatible:target",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HostClass {
    Apple { cores: u16, memory_gib: u16 },
    Windows { memory_gib: u16 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HardwareDescription {
    Apple,
    WindowsLegacy,
    WindowsBytes(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlatformVersion<'a> {
    ApplePrecursor,
    AppleFinal,
    Windows(WindowsVersion<'a>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WindowsVersion<'a> {
    version: u32,
    edition: &'a str,
    release: &'a str,
    kernel: [u32; 3],
    ubr: u32,
    sdk: [u32; 4],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeploymentTarget {
    AppleAarch64,
    AppleX86_64,
    WindowsX86,
    WindowsX86_64,
    WindowsArm64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BoundedEnvironment<'a> {
    host: HostClass,
    hardware: HardwareDescription,
    os: PlatformVersion<'a>,
    target: Option<DeploymentTarget>,
}

/// Validates every string-bearing field before one process profile is retained.
///
/// Schema and privacy admission run before environment compatibility. A safe
/// but unsupported environment therefore cannot be mislabeled as a privacy
/// payload, and neither rejected class can reach report publication.
pub fn validate(profile: &Profile, provenance: Provenance<'_>) -> Result<(), ValidationError> {
    validate_schema(profile, provenance)?;
    let bounded = validate_privacy(profile, provenance)?;
    validate_environment(bounded, provenance)
}

fn validate_schema(profile: &Profile, provenance: Provenance<'_>) -> Result<(), ValidationError> {
    if profile.fixture != FIXTURE_DESCRIPTION
        || profile.build_profile != BUILD_PROFILE
        || profile.correctness_oracle != CORRECTNESS_ORACLE
        || profile.queue_policy != QUEUE_POLICY
        || !is_hex(provenance.source, 40)
        || !is_hex(provenance.tree, 40)
        || !is_hex(provenance.fixture_source, 64)
        || !matches!(provenance.cohort, "precursor" | "final")
        || parse_canonical_u32(provenance.process_index).is_none_or(|index| index > 99)
    {
        return Err(ValidationError::Schema);
    }
    Ok(())
}

fn validate_privacy<'a>(
    profile: &'a Profile,
    provenance: Provenance<'_>,
) -> Result<BoundedEnvironment<'a>, ValidationError> {
    let host = parse_host_class(provenance.host).ok_or(ValidationError::Privacy)?;
    if !bounded_backend(provenance.backend) || !bounded_toolchain(provenance.toolchain) {
        return Err(ValidationError::Privacy);
    }
    let hardware = parse_hardware(&profile.hardware).ok_or(ValidationError::Privacy)?;
    let os = parse_platform_version(&profile.os_version).ok_or(ValidationError::Privacy)?;
    let target = match profile.deployment_target.as_deref() {
        Some(target) => Some(parse_deployment_target(target).ok_or(ValidationError::Privacy)?),
        None => None,
    };
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
        || !is_hex(&profile.fixture_sha256, 64)
        || profile
            .benchmark_executable_sha256
            .as_deref()
            .is_none_or(|digest| !is_hex(digest, 64))
    {
        return Err(ValidationError::Privacy);
    }
    Ok(BoundedEnvironment {
        host,
        hardware,
        os,
        target,
    })
}

fn validate_environment(
    bounded: BoundedEnvironment<'_>,
    provenance: Provenance<'_>,
) -> Result<(), ValidationError> {
    match bounded.host {
        HostClass::Apple {
            cores: 10,
            memory_gib: 32,
        } => {
            if provenance.backend != "opencv-4.14.0" {
                return Err(ValidationError::Environment(EnvironmentFault::Backend));
            }
            if provenance.toolchain != "rust-1.97.1-8bab26f4-llvm-22.1.6" {
                return Err(ValidationError::Environment(EnvironmentFault::Toolchain));
            }
            if bounded.hardware != HardwareDescription::Apple {
                return Err(ValidationError::Environment(EnvironmentFault::Hardware));
            }
            if !matches!(
                bounded.os,
                PlatformVersion::ApplePrecursor | PlatformVersion::AppleFinal
            ) {
                return Err(ValidationError::Environment(
                    EnvironmentFault::OperatingSystem,
                ));
            }
            if bounded.target != Some(DeploymentTarget::AppleAarch64) {
                return Err(ValidationError::Environment(EnvironmentFault::Target));
            }
        }
        HostClass::Windows { memory_gib: 32 } => {
            if provenance.backend != "opencv-4.14.0" {
                return Err(ValidationError::Environment(EnvironmentFault::Backend));
            }
            if provenance.toolchain != "rust-1.97.1-msvc-19.44.35228" {
                return Err(ValidationError::Environment(EnvironmentFault::Toolchain));
            }
            if !matches!(
                bounded.hardware,
                HardwareDescription::WindowsLegacy
                    | HardwareDescription::WindowsBytes(1..=u64::MAX)
            ) {
                return Err(ValidationError::Environment(EnvironmentFault::Hardware));
            }
            let PlatformVersion::Windows(windows) = bounded.os else {
                return Err(ValidationError::Environment(
                    EnvironmentFault::OperatingSystem,
                ));
            };
            if windows.version != 11
                || windows.edition != "Pro"
                || windows.release != "25H2"
                || windows.kernel != [10, 0, 26200]
            {
                return Err(ValidationError::Environment(
                    EnvironmentFault::OperatingSystem,
                ));
            }
            if windows.sdk != WINDOWS_SUPPORTED_SDK {
                return Err(ValidationError::Environment(EnvironmentFault::Sdk));
            }
            if bounded.target != Some(DeploymentTarget::WindowsX86_64) {
                return Err(ValidationError::Environment(EnvironmentFault::Target));
            }
        }
        _ => {
            return Err(ValidationError::Environment(EnvironmentFault::HostClass));
        }
    }
    Ok(())
}

fn is_hex(value: &str, length: usize) -> bool {
    value.len() == length && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn parse_canonical_u32(value: &str) -> Option<u32> {
    if value.is_empty()
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    value.parse().ok()
}

fn parse_canonical_u64(value: &str) -> Option<u64> {
    if value.is_empty()
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    value.parse().ok()
}

fn parse_canonical_u16(value: &str) -> Option<u16> {
    parse_canonical_u32(value)?.try_into().ok()
}

fn parse_host_class(value: &str) -> Option<HostClass> {
    if let Some(parts) = value
        .strip_prefix("apple-m1-pro-")
        .and_then(|rest| rest.split_once("c-"))
    {
        let memory = parts.1.strip_suffix('g')?;
        return Some(HostClass::Apple {
            cores: parse_canonical_u16(parts.0)?,
            memory_gib: parse_canonical_u16(memory)?,
        });
    }
    let memory = value
        .strip_prefix("windows-i7-12700kf-")?
        .strip_suffix('g')?;
    Some(HostClass::Windows {
        memory_gib: parse_canonical_u16(memory)?,
    })
}

fn bounded_backend(value: &str) -> bool {
    value
        .strip_prefix("opencv-")
        .and_then(parse_dotted_3)
        .is_some()
}

fn bounded_toolchain(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("rust-") else {
        return false;
    };
    if let Some((rust, msvc)) = rest.split_once("-msvc-") {
        return !msvc.contains('-')
            && parse_dotted_3(rust).is_some()
            && parse_dotted_3(msvc).is_some();
    }
    let Some((rust_and_revision, llvm)) = rest.split_once("-llvm-") else {
        return false;
    };
    let Some((rust, revision)) = rust_and_revision.rsplit_once('-') else {
        return false;
    };
    parse_dotted_3(rust).is_some() && is_hex(revision, 8) && parse_dotted_3(llvm).is_some()
}

fn parse_hardware(value: &str) -> Option<HardwareDescription> {
    if value == APPLE_HARDWARE {
        return Some(HardwareDescription::Apple);
    }
    if value == WINDOWS_HARDWARE_LEGACY {
        return Some(HardwareDescription::WindowsLegacy);
    }
    let bytes = value
        .strip_prefix(WINDOWS_HARDWARE_PREFIX)?
        .strip_suffix(WINDOWS_HARDWARE_BYTES_SUFFIX)?;
    Some(HardwareDescription::WindowsBytes(parse_canonical_u64(
        bytes,
    )?))
}

fn parse_platform_version(value: &str) -> Option<PlatformVersion<'_>> {
    match value {
        APPLE_PRECURSOR_OS => Some(PlatformVersion::ApplePrecursor),
        APPLE_FINAL_OS => Some(PlatformVersion::AppleFinal),
        _ => parse_windows_version(value).map(PlatformVersion::Windows),
    }
}

fn parse_windows_version(value: &str) -> Option<WindowsVersion<'_>> {
    if value.len() > WINDOWS_VERSION_MAX_BYTES {
        return None;
    }
    let mut parts = value.split(' ');
    if parts.next()? != "Windows" {
        return None;
    }
    let version = parse_canonical_u32(parts.next()?)?;
    let edition = parts.next()?;
    if !matches!(edition, "Home" | "Pro" | "Enterprise") {
        return None;
    }
    let release = parts.next()?;
    if !windows_release_is_bounded(release) {
        return None;
    }
    let kernel = parse_dotted_3(parts.next()?)?;
    if parts.next()? != "UBR" {
        return None;
    }
    let ubr = parse_canonical_u32(parts.next()?.strip_suffix(';')?)?;
    if parts.next()? != "Windows" || parts.next()? != "SDK" {
        return None;
    }
    let sdk = parse_dotted_4(parts.next()?)?;
    if parts.next().is_some() {
        return None;
    }
    Some(WindowsVersion {
        version,
        edition,
        release,
        kernel,
        ubr,
        sdk,
    })
}

fn windows_release_is_bounded(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 4
        && bytes[0].is_ascii_digit()
        && bytes[1].is_ascii_digit()
        && bytes[2] == b'H'
        && matches!(bytes[3], b'1' | b'2')
}

fn parse_dotted_3(value: &str) -> Option<[u32; 3]> {
    let mut parts = value.split('.');
    let parsed = [
        parse_canonical_u32(parts.next()?)?,
        parse_canonical_u32(parts.next()?)?,
        parse_canonical_u32(parts.next()?)?,
    ];
    (parts.next().is_none()).then_some(parsed)
}

fn parse_dotted_4(value: &str) -> Option<[u32; 4]> {
    let mut parts = value.split('.');
    let parsed = [
        parse_canonical_u32(parts.next()?)?,
        parse_canonical_u32(parts.next()?)?,
        parse_canonical_u32(parts.next()?)?,
        parse_canonical_u32(parts.next()?)?,
    ];
    (parts.next().is_none()).then_some(parsed)
}

fn parse_deployment_target(value: &str) -> Option<DeploymentTarget> {
    match value {
        "aarch64-apple-darwin" => Some(DeploymentTarget::AppleAarch64),
        "x86_64-apple-darwin" => Some(DeploymentTarget::AppleX86_64),
        "i686-pc-windows-msvc" => Some(DeploymentTarget::WindowsX86),
        "x86_64-pc-windows-msvc" => Some(DeploymentTarget::WindowsX86_64),
        "aarch64-pc-windows-msvc" => Some(DeploymentTarget::WindowsArm64),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        APPLE_FINAL_OS, APPLE_HARDWARE, APPLE_PRECURSOR_OS, BUILD_PROFILE, CORRECTNESS_ORACLE,
        EnvironmentFault, FIXTURE_DESCRIPTION, Provenance, QUEUE_POLICY, ValidationError,
        WINDOWS_HARDWARE_LEGACY, validate,
    };
    use crate::bench_harness::Profile;

    const SOURCE: &str = "0123456789abcdef0123456789abcdef01234567";
    const TREE: &str = "89abcdef0123456789abcdef0123456789abcdef";
    const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const WINDOWS_OBSERVED_HARDWARE: &str = "Intel Core i7-12700KF, 34197635072 bytes memory";
    const WINDOWS_CANONICAL_OS: &str =
        "Windows 11 Pro 25H2 10.0.26200 UBR 9168; Windows SDK 10.0.26100.0";
    const WINDOWS_SERVICED_OS: &str =
        "Windows 11 Pro 25H2 10.0.26200 UBR 9278; Windows SDK 10.0.26100.0";

    fn apple_provenance() -> Provenance<'static> {
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

    fn windows_provenance() -> Provenance<'static> {
        Provenance {
            backend: "opencv-4.14.0",
            toolchain: "rust-1.97.1-msvc-19.44.35228",
            host: "windows-i7-12700kf-32g",
            cohort: "final",
            ..apple_provenance()
        }
    }

    fn notes(provenance: Provenance<'_>) -> String {
        format!(
            "source {}; tree {}; fixture-source {}; backend {}; toolchain {}; host {}; cohort {}; process {}; control native-watch-control-v1",
            provenance.source,
            provenance.tree,
            provenance.fixture_source,
            provenance.backend,
            provenance.toolchain,
            provenance.host,
            provenance.cohort,
            provenance.process_index,
        )
    }

    fn profile(
        provenance: Provenance<'_>,
        hardware: &str,
        os_version: &str,
        deployment_target: Option<&str>,
    ) -> Profile {
        Profile {
            fixture: FIXTURE_DESCRIPTION.to_owned(),
            fixture_sha256: DIGEST.to_owned(),
            benchmark_executable_sha256: Some(DIGEST.to_owned()),
            hardware: hardware.to_owned(),
            os_version: os_version.to_owned(),
            deployment_target: deployment_target.map(str::to_owned),
            build_profile: BUILD_PROFILE.to_owned(),
            correctness_oracle: CORRECTNESS_ORACLE,
            queue_policy: QUEUE_POLICY,
            notes: Some(notes(provenance)),
        }
    }

    fn apple_profile() -> Profile {
        profile(
            apple_provenance(),
            APPLE_HARDWARE,
            APPLE_PRECURSOR_OS,
            Some("aarch64-apple-darwin"),
        )
    }

    fn windows_profile(provenance: Provenance<'_>, hardware: &str, os_version: &str) -> Profile {
        profile(
            provenance,
            hardware,
            os_version,
            Some("x86_64-pc-windows-msvc"),
        )
    }

    #[test]
    fn existing_apple_and_legacy_windows_profiles_are_accepted() {
        assert_eq!(validate(&apple_profile(), apple_provenance()), Ok(()));

        let mut apple_final = apple_profile();
        apple_final.os_version = APPLE_FINAL_OS.to_owned();
        assert_eq!(validate(&apple_final, apple_provenance()), Ok(()));

        let provenance = windows_provenance();
        let windows = windows_profile(provenance, WINDOWS_HARDWARE_LEGACY, WINDOWS_CANONICAL_OS);
        assert_eq!(validate(&windows, provenance), Ok(()));
    }

    #[test]
    fn supported_windows_serviced_update_is_accepted() {
        let provenance = windows_provenance();
        let candidate = windows_profile(provenance, WINDOWS_OBSERVED_HARDWARE, WINDOWS_SERVICED_OS);

        assert_eq!(validate(&candidate, provenance), Ok(()));
    }

    #[test]
    fn windows_numeric_provenance_requires_bounded_canonical_decimal() {
        let provenance = windows_provenance();
        for ubr in ["0", "9278", "4294967295"] {
            let os = format!("Windows 11 Pro 25H2 10.0.26200 UBR {ubr}; Windows SDK 10.0.26100.0");
            let candidate = windows_profile(provenance, WINDOWS_OBSERVED_HARDWARE, &os);
            assert_eq!(validate(&candidate, provenance), Ok(()), "UBR {ubr}");
        }

        for ubr in ["00", "09278", "4294967296", "+1", "1x"] {
            let os = format!("Windows 11 Pro 25H2 10.0.26200 UBR {ubr}; Windows SDK 10.0.26100.0");
            let candidate = windows_profile(provenance, WINDOWS_OBSERVED_HARDWARE, &os);
            assert_eq!(
                validate(&candidate, provenance),
                Err(ValidationError::Privacy),
                "UBR {ubr}"
            );
        }

        let max_memory = windows_profile(
            provenance,
            "Intel Core i7-12700KF, 18446744073709551615 bytes memory",
            WINDOWS_SERVICED_OS,
        );
        assert_eq!(validate(&max_memory, provenance), Ok(()));

        let zero_memory = windows_profile(
            provenance,
            "Intel Core i7-12700KF, 0 bytes memory",
            WINDOWS_SERVICED_OS,
        );
        assert_eq!(
            validate(&zero_memory, provenance),
            Err(ValidationError::Environment(EnvironmentFault::Hardware))
        );

        for hardware in [
            "Intel Core i7-12700KF, 034197635072 bytes memory",
            "Intel Core i7-12700KF, 18446744073709551616 bytes memory",
        ] {
            let candidate = windows_profile(provenance, hardware, WINDOWS_SERVICED_OS);
            assert_eq!(
                validate(&candidate, provenance),
                Err(ValidationError::Privacy),
                "hardware {hardware}"
            );
        }
    }

    #[test]
    fn windows_os_provenance_requires_bounded_single_space_grammar() {
        let provenance = windows_provenance();
        for os in [
            format!(" {WINDOWS_SERVICED_OS}"),
            format!("{WINDOWS_SERVICED_OS} "),
            WINDOWS_SERVICED_OS.replacen("Windows 11", "Windows  11", 1),
            WINDOWS_SERVICED_OS.replacen("Windows 11", "Windows\t11", 1),
            WINDOWS_SERVICED_OS.replacen("Windows 11", "Windows\n11", 1),
        ] {
            let candidate = windows_profile(provenance, WINDOWS_OBSERVED_HARDWARE, os.as_str());
            assert_eq!(
                validate(&candidate, provenance),
                Err(ValidationError::Privacy),
                "OS whitespace variant"
            );
        }

        let overlong = "Windows 4294967295 Enterprise 99H2 4294967295.4294967295.4294967295 UBR 4294967295; Windows SDK 4294967295.4294967295.4294967295.4294967295";
        assert!(overlong.len() > 128);
        let candidate = windows_profile(provenance, WINDOWS_OBSERVED_HARDWARE, overlong);
        assert_eq!(
            validate(&candidate, provenance),
            Err(ValidationError::Privacy)
        );
    }

    #[test]
    fn bounded_but_unsupported_windows_environment_is_typed() {
        let base = windows_provenance();

        let host = Provenance {
            host: "windows-i7-12700kf-64g",
            ..base
        };
        assert_eq!(
            validate(
                &windows_profile(host, WINDOWS_OBSERVED_HARDWARE, WINDOWS_SERVICED_OS),
                host,
            ),
            Err(ValidationError::Environment(EnvironmentFault::HostClass))
        );

        let backend = Provenance {
            backend: "opencv-4.15.0",
            ..base
        };
        assert_eq!(
            validate(
                &windows_profile(backend, WINDOWS_OBSERVED_HARDWARE, WINDOWS_SERVICED_OS),
                backend,
            ),
            Err(ValidationError::Environment(EnvironmentFault::Backend))
        );

        let toolchain = Provenance {
            toolchain: "rust-1.97.2-msvc-19.44.35228",
            ..base
        };
        assert_eq!(
            validate(
                &windows_profile(toolchain, WINDOWS_OBSERVED_HARDWARE, WINDOWS_SERVICED_OS),
                toolchain,
            ),
            Err(ValidationError::Environment(EnvironmentFault::Toolchain))
        );

        assert_eq!(
            validate(
                &windows_profile(base, APPLE_HARDWARE, WINDOWS_SERVICED_OS),
                base,
            ),
            Err(ValidationError::Environment(EnvironmentFault::Hardware))
        );

        let unsupported_os = "Windows 11 Home 25H2 10.0.26200 UBR 9278; Windows SDK 10.0.26100.0";
        assert_eq!(
            validate(
                &windows_profile(base, WINDOWS_OBSERVED_HARDWARE, unsupported_os),
                base,
            ),
            Err(ValidationError::Environment(
                EnvironmentFault::OperatingSystem
            ))
        );

        let unsupported_sdk = "Windows 11 Pro 25H2 10.0.26200 UBR 9278; Windows SDK 10.0.22621.0";
        assert_eq!(
            validate(
                &windows_profile(base, WINDOWS_OBSERVED_HARDWARE, unsupported_sdk),
                base,
            ),
            Err(ValidationError::Environment(EnvironmentFault::Sdk))
        );

        let unsupported_target = profile(
            base,
            WINDOWS_OBSERVED_HARDWARE,
            WINDOWS_SERVICED_OS,
            Some("i686-pc-windows-msvc"),
        );
        assert_eq!(
            validate(&unsupported_target, base),
            Err(ValidationError::Environment(EnvironmentFault::Target))
        );
    }

    #[test]
    fn arbitrary_or_sensitive_payload_fields_are_rejected() {
        let provenance = windows_provenance();
        for payload in [
            "Alice MacBook Pro serial C02SECRET",
            "window title Secret Project",
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
            let mut candidate =
                windows_profile(provenance, WINDOWS_OBSERVED_HARDWARE, WINDOWS_SERVICED_OS);
            candidate.hardware = payload.to_owned();
            assert_eq!(
                validate(&candidate, provenance),
                Err(ValidationError::Privacy),
                "hardware payload {payload}"
            );

            let mut candidate =
                windows_profile(provenance, WINDOWS_OBSERVED_HARDWARE, WINDOWS_SERVICED_OS);
            candidate.os_version = payload.to_owned();
            assert_eq!(
                validate(&candidate, provenance),
                Err(ValidationError::Privacy),
                "OS payload {payload}"
            );
        }

        for os in [
            "Windows 11 Pro 25H2 10.0.26200 UBR 9278;; Windows SDK 10.0.26100.0",
            "Windows 11 Pro 25H2 10.0.26200 UBR 9278; Windows SDK 10.0.26100.0 trailing",
            "Windows 11 Pro 25H2 10.0.26200 UBR 9278; Windows SDK 10.0.4294967296.0",
        ] {
            let candidate = windows_profile(provenance, WINDOWS_OBSERVED_HARDWARE, os);
            assert_eq!(
                validate(&candidate, provenance),
                Err(ValidationError::Privacy),
                "OS {os}"
            );
        }

        for (field, candidate_provenance) in [
            (
                "backend",
                Provenance {
                    backend: r"opencv-C:\\private",
                    ..provenance
                },
            ),
            (
                "toolchain",
                Provenance {
                    toolchain: "rust-1.97.1-msvc-window_title",
                    ..provenance
                },
            ),
            (
                "host",
                Provenance {
                    host: "windows-window-title-secret",
                    ..provenance
                },
            ),
        ] {
            let candidate = windows_profile(
                candidate_provenance,
                WINDOWS_OBSERVED_HARDWARE,
                WINDOWS_SERVICED_OS,
            );
            assert_eq!(
                validate(&candidate, candidate_provenance),
                Err(ValidationError::Privacy),
                "{field}"
            );
        }

        let target_payload = profile(
            provenance,
            WINDOWS_OBSERVED_HARDWARE,
            WINDOWS_SERVICED_OS,
            Some(r"C:\\private\\target"),
        );
        assert_eq!(
            validate(&target_payload, provenance),
            Err(ValidationError::Privacy)
        );
    }

    #[test]
    fn fixed_schema_and_notes_reject_extension() {
        let provenance = apple_provenance();

        let mut candidate = apple_profile();
        candidate.fixture.push_str(" at /tmp/private");
        assert_eq!(
            validate(&candidate, provenance),
            Err(ValidationError::Schema)
        );

        let malformed_source = Provenance {
            source: "not-a-source-revision",
            ..provenance
        };
        assert_eq!(
            validate(
                &profile(
                    malformed_source,
                    APPLE_HARDWARE,
                    APPLE_PRECURSOR_OS,
                    Some("aarch64-apple-darwin"),
                ),
                malformed_source,
            ),
            Err(ValidationError::Schema)
        );

        let noncanonical_index = Provenance {
            process_index: "01",
            ..provenance
        };
        assert_eq!(
            validate(
                &profile(
                    noncanonical_index,
                    APPLE_HARDWARE,
                    APPLE_PRECURSOR_OS,
                    Some("aarch64-apple-darwin"),
                ),
                noncanonical_index,
            ),
            Err(ValidationError::Schema)
        );

        let mut candidate = apple_profile();
        candidate
            .notes
            .as_mut()
            .expect("notes")
            .push_str("; native_error=private");
        assert_eq!(
            validate(&candidate, provenance),
            Err(ValidationError::Privacy)
        );

        let mut candidate = apple_profile();
        candidate.fixture_sha256 = "/tmp/private".to_owned();
        assert_eq!(
            validate(&candidate, provenance),
            Err(ValidationError::Privacy)
        );
    }

    #[test]
    fn missing_benchmark_executable_digest_is_rejected() {
        let provenance = apple_provenance();
        let mut candidate = apple_profile();
        candidate.benchmark_executable_sha256 = None;

        assert_eq!(
            validate(&candidate, provenance),
            Err(ValidationError::Privacy)
        );
    }

    #[test]
    fn windows_update_rules_do_not_broaden_apple_profiles() {
        let provenance = apple_provenance();
        let mut final_profile = apple_profile();
        final_profile.os_version = APPLE_FINAL_OS.to_owned();
        assert_eq!(validate(&final_profile, provenance), Ok(()));

        let mut changed_apple = apple_profile();
        changed_apple.os_version = "macOS 26.6.3 build 25G84, SDK 26.5".to_owned();
        assert_eq!(
            validate(&changed_apple, provenance),
            Err(ValidationError::Privacy)
        );

        let bounded_windows_os = profile(
            provenance,
            APPLE_HARDWARE,
            WINDOWS_SERVICED_OS,
            Some("aarch64-apple-darwin"),
        );
        assert_eq!(
            validate(&bounded_windows_os, provenance),
            Err(ValidationError::Environment(
                EnvironmentFault::OperatingSystem
            ))
        );

        let unsupported_target = profile(
            provenance,
            APPLE_HARDWARE,
            APPLE_PRECURSOR_OS,
            Some("x86_64-apple-darwin"),
        );
        assert_eq!(
            validate(&unsupported_target, provenance),
            Err(ValidationError::Environment(EnvironmentFault::Target))
        );

        let windows = windows_profile(provenance, WINDOWS_OBSERVED_HARDWARE, WINDOWS_SERVICED_OS);
        assert_eq!(
            validate(&windows, provenance),
            Err(ValidationError::Environment(EnvironmentFault::Hardware))
        );
    }

    #[test]
    fn validation_tokens_preserve_failure_taxonomy() {
        assert_eq!(ValidationError::Schema.token(), "schema_violation");
        assert_eq!(ValidationError::Privacy.token(), "privacy_violation");
        assert_eq!(
            ValidationError::Environment(EnvironmentFault::HostClass).token(),
            "environment_incompatible:host_class"
        );
        assert_eq!(
            ValidationError::Environment(EnvironmentFault::Backend).token(),
            "environment_incompatible:backend"
        );
        assert_eq!(
            ValidationError::Environment(EnvironmentFault::Toolchain).token(),
            "environment_incompatible:toolchain"
        );
        assert_eq!(
            ValidationError::Environment(EnvironmentFault::Hardware).token(),
            "environment_incompatible:hardware"
        );
        assert_eq!(
            ValidationError::Environment(EnvironmentFault::OperatingSystem).token(),
            "environment_incompatible:operating_system"
        );
        assert_eq!(
            ValidationError::Environment(EnvironmentFault::Sdk).token(),
            "environment_incompatible:sdk"
        );
        assert_eq!(
            ValidationError::Environment(EnvironmentFault::Target).token(),
            "environment_incompatible:target"
        );
    }
}
