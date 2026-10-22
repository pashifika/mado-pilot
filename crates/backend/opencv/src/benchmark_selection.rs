//! Qualification-only entry to the production score selector, without counters,
//! plus the effective OpenCV parallel thread setting observed and never changed.

use mado_pilot_core::{Interruption, OperationContext};
use mado_pilot_vision::Suppression;

use crate::candidates::{Peak, PeakSearch, peaks};

/// Effective matching or mapping instrumentation, including unified features.
pub const INSTRUMENTATION_ENABLED: bool = cfg!(feature = "benchmark-instrumentation")
    || mado_pilot_capture::BENCHMARK_INSTRUMENTATION_ENABLED;

/// Selection-only benchmark input; the supplied score slice is disposable.
#[derive(Debug, Clone, Copy)]
pub struct SelectionRequest {
    /// Score-map columns.
    pub width: usize,
    /// Score-map rows.
    pub height: usize,
    /// Template columns.
    pub template_width: usize,
    /// Template rows.
    pub template_height: usize,
    /// Inclusive public-score threshold.
    pub min_score: f64,
    /// Maximum returned candidates.
    pub candidate_budget: usize,
    /// Requested overlap policy.
    pub suppression: Suppression,
}

/// Selected positions and public scores, without an extra result-vector copy.
#[derive(Debug)]
pub struct SelectedScores(Vec<Peak>);

impl SelectedScores {
    /// Iterates in canonical result order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (usize, usize, f64)> + '_ {
        self.0.iter().map(|peak| (peak.left, peak.top, peak.score))
    }
}

/// Runs the production selector independently of OpenCV score generation.
///
/// Score work is counted only when `benchmark-instrumentation` is also enabled.
///
/// # Errors
/// Returns the caller's cancellation or deadline interruption.
pub fn select_scores(
    scores: &mut [f32],
    request: SelectionRequest,
    operation: &OperationContext,
) -> Result<SelectedScores, Interruption> {
    peaks(
        scores,
        PeakSearch {
            width: request.width,
            height: request.height,
            template_width: request.template_width,
            template_height: request.template_height,
            min_score: request.min_score,
            candidate_budget: request.candidate_budget,
            suppression: request.suppression,
        },
        operation,
    )
    .map(SelectedScores)
}

/// Effective OpenCV parallel-region thread setting as the library reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelThreads {
    /// Runtime backend named by `cv::currentParallelFramework`.
    pub framework: String,
    /// `cv::getNumThreads` result for that backend.
    pub threads: i32,
}

/// Observes the runtime backend and its OpenCV parallel-region thread setting.
///
/// # Errors
/// Returns unavailable for a native exception, missing/overlong backend name,
/// unsupported GCD semantics, or a non-positive count. Nothing is set or tuned.
pub fn parallel_threads() -> Result<ParallelThreads, String> {
    unsafe extern "C" {
        fn madopilot_benchmark_parallel_threads(
            framework: *mut u8,
            capacity: usize,
            threads: *mut i32,
        ) -> i32;
    }
    let mut name = [0_u8; 128];
    let mut threads = 0;
    // SAFETY: the shim writes only within this buffer and scalar, retains no
    // pointers, and contains every C++ exception. The benchmark never reconfigures
    // OpenCV concurrently with this observation.
    let status = unsafe {
        madopilot_benchmark_parallel_threads(name.as_mut_ptr(), name.len(), &mut threads)
    };
    match status {
        0 => {}
        1 => return Err("OpenCV runtime parallel backend is unavailable".into()),
        2 => return Err("OpenCV runtime backend name exceeds 127 bytes".into()),
        _ => return Err("OpenCV parallel backend observation raised a native exception".into()),
    }
    let length = name
        .iter()
        .position(|&byte| byte == 0)
        .ok_or("unterminated backend name")?;
    let framework = std::str::from_utf8(&name[..length]).map_err(|error| error.to_string())?;
    if framework.eq_ignore_ascii_case("gcd") {
        return Err("cv::getNumThreads is documented as unsupported for the GCD backend".into());
    }
    if threads < 1 {
        return Err(format!(
            "cv::getNumThreads reported a non-positive count {threads}"
        ));
    }
    Ok(ParallelThreads {
        framework: framework.to_owned(),
        threads,
    })
}
