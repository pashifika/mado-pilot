//! The evidence-backed ceilings, and the caller-configured limits below them.
//!
//! Every value here comes from the measurements in `docs/evidence/g-014` and is
//! fixed by [ADR 0001]. A caller may set any limit at or below its ceiling; a
//! limit above one is rejected as an invalid argument rather than silently
//! clamped, because a host that asked for a weaker guard needs to be told it did
//! not get one.
//!
//! Raising a ceiling is a superseding ADR with fresh measurements on both
//! release targets, not an edit to this file.
//!
//! [ADR 0001]: https://github.com/pashifika/mado-pilot/blob/main/docs/adr/0001-asset-archive-container-and-safety-ceilings.md

use crate::fault::{AssetFault, AssetFaultKind, LoadStage};

/// Resource limits applied to one package load.
///
/// The defaults are the implementation ceilings, so a caller that configures
/// nothing gets the strongest guards this build supports.
///
/// Three of the six describe archive structure and apply to archive sources
/// only: a directory has no trailer to record an entry count in and no
/// compressed representation to expand from. The other three bound allocation
/// the loader performs whatever the source is, so directory and memory sources
/// are held to them as well.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssetLimits {
    max_manifest_bytes: u64,
    max_entry_count: u32,
    max_entry_uncompressed_bytes: u64,
    max_total_compressed_bytes: u64,
    max_total_uncompressed_bytes: u64,
    max_compression_ratio: u32,
}

impl AssetLimits {
    /// The largest manifest this build will read: 4 MiB.
    pub const MAX_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;
    /// The largest number of entries a package may contain: 4,096.
    pub const MAX_ENTRY_COUNT: u32 = 4_096;
    /// The largest single entry this build will expand: 64 MiB.
    pub const MAX_ENTRY_UNCOMPRESSED_BYTES: u64 = 64 * 1024 * 1024;
    /// The largest archive this build will open: 256 MiB.
    pub const MAX_TOTAL_COMPRESSED_BYTES: u64 = 256 * 1024 * 1024;
    /// The largest total expansion this build will admit: 512 MiB.
    pub const MAX_TOTAL_UNCOMPRESSED_BYTES: u64 = 512 * 1024 * 1024;
    /// The largest ratio of declared expansion to compressed bytes: 64.
    pub const MAX_COMPRESSION_RATIO: u32 = 64;

    /// Returns the implementation ceilings.
    #[must_use]
    pub const fn ceiling() -> Self {
        Self {
            max_manifest_bytes: Self::MAX_MANIFEST_BYTES,
            max_entry_count: Self::MAX_ENTRY_COUNT,
            max_entry_uncompressed_bytes: Self::MAX_ENTRY_UNCOMPRESSED_BYTES,
            max_total_compressed_bytes: Self::MAX_TOTAL_COMPRESSED_BYTES,
            max_total_uncompressed_bytes: Self::MAX_TOTAL_UNCOMPRESSED_BYTES,
            max_compression_ratio: Self::MAX_COMPRESSION_RATIO,
        }
    }

    /// Lowers the manifest byte limit.
    ///
    /// # Errors
    ///
    /// Returns [`AssetFaultKind::LimitAboveCeiling`] when `value` exceeds
    /// [`AssetLimits::MAX_MANIFEST_BYTES`].
    pub const fn with_max_manifest_bytes(mut self, value: u64) -> Result<Self, AssetFault> {
        if value > Self::MAX_MANIFEST_BYTES {
            return Err(above_ceiling());
        }
        self.max_manifest_bytes = value;
        Ok(self)
    }

    /// Lowers the entry-count limit.
    ///
    /// # Errors
    ///
    /// Returns [`AssetFaultKind::LimitAboveCeiling`] when `value` exceeds
    /// [`AssetLimits::MAX_ENTRY_COUNT`].
    pub const fn with_max_entry_count(mut self, value: u32) -> Result<Self, AssetFault> {
        if value > Self::MAX_ENTRY_COUNT {
            return Err(above_ceiling());
        }
        self.max_entry_count = value;
        Ok(self)
    }

    /// Lowers the per-entry expansion limit.
    ///
    /// # Errors
    ///
    /// Returns [`AssetFaultKind::LimitAboveCeiling`] when `value` exceeds
    /// [`AssetLimits::MAX_ENTRY_UNCOMPRESSED_BYTES`].
    pub const fn with_max_entry_uncompressed_bytes(
        mut self,
        value: u64,
    ) -> Result<Self, AssetFault> {
        if value > Self::MAX_ENTRY_UNCOMPRESSED_BYTES {
            return Err(above_ceiling());
        }
        self.max_entry_uncompressed_bytes = value;
        Ok(self)
    }

    /// Lowers the total source-byte limit.
    ///
    /// # Errors
    ///
    /// Returns [`AssetFaultKind::LimitAboveCeiling`] when `value` exceeds
    /// [`AssetLimits::MAX_TOTAL_COMPRESSED_BYTES`].
    pub const fn with_max_total_compressed_bytes(mut self, value: u64) -> Result<Self, AssetFault> {
        if value > Self::MAX_TOTAL_COMPRESSED_BYTES {
            return Err(above_ceiling());
        }
        self.max_total_compressed_bytes = value;
        Ok(self)
    }

    /// Lowers the total expansion limit.
    ///
    /// # Errors
    ///
    /// Returns [`AssetFaultKind::LimitAboveCeiling`] when `value` exceeds
    /// [`AssetLimits::MAX_TOTAL_UNCOMPRESSED_BYTES`].
    pub const fn with_max_total_uncompressed_bytes(
        mut self,
        value: u64,
    ) -> Result<Self, AssetFault> {
        if value > Self::MAX_TOTAL_UNCOMPRESSED_BYTES {
            return Err(above_ceiling());
        }
        self.max_total_uncompressed_bytes = value;
        Ok(self)
    }

    /// Lowers the compression-ratio limit.
    ///
    /// # Errors
    ///
    /// Returns [`AssetFaultKind::LimitAboveCeiling`] when `value` exceeds
    /// [`AssetLimits::MAX_COMPRESSION_RATIO`].
    pub const fn with_max_compression_ratio(mut self, value: u32) -> Result<Self, AssetFault> {
        if value > Self::MAX_COMPRESSION_RATIO {
            return Err(above_ceiling());
        }
        self.max_compression_ratio = value;
        Ok(self)
    }

    /// Returns the manifest byte limit.
    #[must_use]
    pub const fn max_manifest_bytes(self) -> u64 {
        self.max_manifest_bytes
    }

    /// Returns the entry-count limit.
    #[must_use]
    pub const fn max_entry_count(self) -> u32 {
        self.max_entry_count
    }

    /// Returns the per-entry expansion limit.
    #[must_use]
    pub const fn max_entry_uncompressed_bytes(self) -> u64 {
        self.max_entry_uncompressed_bytes
    }

    /// Returns the total source-byte limit.
    #[must_use]
    pub const fn max_total_compressed_bytes(self) -> u64 {
        self.max_total_compressed_bytes
    }

    /// Returns the total expansion limit.
    #[must_use]
    pub const fn max_total_uncompressed_bytes(self) -> u64 {
        self.max_total_uncompressed_bytes
    }

    /// Returns the compression-ratio limit.
    #[must_use]
    pub const fn max_compression_ratio(self) -> u32 {
        self.max_compression_ratio
    }
}

impl Default for AssetLimits {
    fn default() -> Self {
        Self::ceiling()
    }
}

const fn above_ceiling() -> AssetFault {
    AssetFault::new(AssetFaultKind::LimitAboveCeiling, LoadStage::Configuration)
}

#[cfg(test)]
mod tests {
    use super::AssetLimits;
    use crate::fault::{AssetFaultKind, LoadStage};

    #[test]
    fn the_default_is_the_implementation_ceiling() {
        assert_eq!(AssetLimits::default(), AssetLimits::ceiling());
    }

    #[test]
    fn the_ceilings_are_the_values_adr_0001_accepted() {
        let ceiling = AssetLimits::ceiling();

        assert_eq!(ceiling.max_manifest_bytes(), 4_194_304);
        assert_eq!(ceiling.max_entry_count(), 4_096);
        assert_eq!(ceiling.max_entry_uncompressed_bytes(), 67_108_864);
        assert_eq!(ceiling.max_total_compressed_bytes(), 268_435_456);
        assert_eq!(ceiling.max_total_uncompressed_bytes(), 536_870_912);
        assert_eq!(ceiling.max_compression_ratio(), 64);
    }

    #[test]
    fn a_caller_may_lower_every_limit() {
        let limits = AssetLimits::ceiling()
            .with_max_manifest_bytes(1_024)
            .expect("below the ceiling")
            .with_max_entry_count(8)
            .expect("below the ceiling")
            .with_max_entry_uncompressed_bytes(2_048)
            .expect("below the ceiling")
            .with_max_total_compressed_bytes(4_096)
            .expect("below the ceiling")
            .with_max_total_uncompressed_bytes(8_192)
            .expect("below the ceiling")
            .with_max_compression_ratio(2)
            .expect("below the ceiling");

        assert_eq!(limits.max_manifest_bytes(), 1_024);
        assert_eq!(limits.max_entry_count(), 8);
        assert_eq!(limits.max_entry_uncompressed_bytes(), 2_048);
        assert_eq!(limits.max_total_compressed_bytes(), 4_096);
        assert_eq!(limits.max_total_uncompressed_bytes(), 8_192);
        assert_eq!(limits.max_compression_ratio(), 2);
    }

    #[test]
    fn a_caller_may_set_a_limit_exactly_at_its_ceiling() {
        assert!(
            AssetLimits::ceiling()
                .with_max_entry_count(AssetLimits::MAX_ENTRY_COUNT)
                .is_ok()
        );
    }

    #[test]
    fn no_caller_may_raise_a_limit_above_its_ceiling() {
        let ceiling = AssetLimits::ceiling();
        let attempts = [
            ceiling
                .with_max_manifest_bytes(AssetLimits::MAX_MANIFEST_BYTES + 1)
                .err(),
            ceiling
                .with_max_entry_count(AssetLimits::MAX_ENTRY_COUNT + 1)
                .err(),
            ceiling
                .with_max_entry_uncompressed_bytes(AssetLimits::MAX_ENTRY_UNCOMPRESSED_BYTES + 1)
                .err(),
            ceiling
                .with_max_total_compressed_bytes(AssetLimits::MAX_TOTAL_COMPRESSED_BYTES + 1)
                .err(),
            ceiling
                .with_max_total_uncompressed_bytes(AssetLimits::MAX_TOTAL_UNCOMPRESSED_BYTES + 1)
                .err(),
            ceiling
                .with_max_compression_ratio(AssetLimits::MAX_COMPRESSION_RATIO + 1)
                .err(),
        ];

        for attempt in attempts {
            let fault = attempt.expect("above the ceiling");
            assert_eq!(fault.kind(), AssetFaultKind::LimitAboveCeiling);
            assert_eq!(fault.stage(), LoadStage::Configuration);
        }
    }

    #[test]
    fn a_rejected_limit_leaves_the_original_untouched() {
        let ceiling = AssetLimits::ceiling();

        assert!(ceiling.with_max_entry_count(u32::MAX).is_err());
        assert_eq!(ceiling.max_entry_count(), AssetLimits::MAX_ENTRY_COUNT);
    }
}
