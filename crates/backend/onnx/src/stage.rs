//! Monotonic stage clocks compiled only into the qualification feature.

/// One stage measurement; a zero-sized no-op without `benchmark-instrumentation`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct StageTimer {
    #[cfg(feature = "benchmark-instrumentation")]
    started: std::time::Instant,
}

impl StageTimer {
    #[inline]
    pub(crate) fn start() -> Self {
        Self {
            #[cfg(feature = "benchmark-instrumentation")]
            started: std::time::Instant::now(),
        }
    }

    /// Returns saturated elapsed nanoseconds; always zero without the feature.
    #[inline]
    pub(crate) fn elapsed_nanos(self) -> u64 {
        #[cfg(feature = "benchmark-instrumentation")]
        {
            u64::try_from(self.started.elapsed().as_nanos()).unwrap_or(u64::MAX)
        }
        #[cfg(not(feature = "benchmark-instrumentation"))]
        {
            0
        }
    }
}

/// Native-run and postprocessing time of one completed inference call.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct RunNanos {
    pub(crate) inference: u64,
    pub(crate) postprocess: u64,
}
