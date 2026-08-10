//! Engine-scoped identities, stream epochs, frame sequences, and frame stamps.
//!
//! Identities are opaque typed values. A title, a process identifier, a file
//! path, and a native handle are all descriptive metadata that an operating
//! system reuses freely, so none of them establishes that two observations are
//! the same target. What establishes it is that the same engine issued the same
//! identity.

use std::fmt;
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::status::{Error, Status};

/// A rule about identity that could not be satisfied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum IdentityFault {
    /// The identity was issued by a different engine.
    ForeignEngine,
    /// The identity was issued by a different provider of the same engine.
    ForeignProvider,
    /// The identity space is exhausted; issuing another would alias one already
    /// handed out.
    Exhausted,
    /// The stream's per-epoch sequence is exhausted.
    SequenceExhausted,
    /// The stream's epoch counter is exhausted.
    EpochExhausted,
    /// Two frame identities that had to belong to one stream did not.
    StreamMismatch,
}

impl IdentityFault {
    /// Returns the public status this fault reports as.
    #[must_use]
    pub const fn status(self) -> Status {
        match self {
            IdentityFault::ForeignEngine
            | IdentityFault::ForeignProvider
            | IdentityFault::StreamMismatch => Status::InvalidArgument,
            IdentityFault::Exhausted
            | IdentityFault::SequenceExhausted
            | IdentityFault::EpochExhausted => Status::LimitExceeded,
        }
    }

    const fn detail(self) -> &'static str {
        match self {
            IdentityFault::ForeignEngine => "identity was issued by another engine",
            IdentityFault::ForeignProvider => "identity was issued by another provider",
            IdentityFault::Exhausted => "identity space is exhausted",
            IdentityFault::SequenceExhausted => "stream frame sequence is exhausted",
            IdentityFault::EpochExhausted => "stream epoch counter is exhausted",
            IdentityFault::StreamMismatch => "frame identities belong to different streams",
        }
    }
}

impl fmt::Display for IdentityFault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.detail())
    }
}

impl std::error::Error for IdentityFault {}

impl From<IdentityFault> for Error {
    fn from(fault: IdentityFault) -> Self {
        Error::new(fault.status(), fault.detail())
    }
}

/// Identifies one engine instance within the process.
///
/// Every identity an engine issues carries this, so submitting an identity to a
/// different engine is a detectable mistake rather than an operation on an
/// unrelated target that happens to share an ordinal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EngineId(NonZeroU64);

impl fmt::Display for EngineId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "engine#{}", self.0)
    }
}

/// Names the provider that discovered a target.
///
/// A provider is a capture source family such as replay, Windows, or macOS. The
/// name qualifies an identity; it is not itself the identity, and two providers
/// may hand out the same ordinal without collision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProviderId(&'static str);

impl ProviderId {
    /// Names a provider.
    #[must_use]
    pub const fn new(name: &'static str) -> Self {
        Self(name)
    }

    /// Returns the provider name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        self.0
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

/// An opaque handle to one discovered capture target.
///
/// Two targets that report the same title and application metadata still get
/// distinct identities. An identity is reused for the same target only while
/// that target continues to exist, and never for a replacement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TargetId {
    engine: EngineId,
    provider: ProviderId,
    ordinal: NonZeroU64,
}

impl TargetId {
    /// Returns the engine that issued this identity.
    #[must_use]
    pub const fn engine(self) -> EngineId {
        self.engine
    }

    /// Returns the provider that discovered the target.
    #[must_use]
    pub const fn provider(self) -> ProviderId {
        self.provider
    }

    /// Returns the checked nonzero ordinal within the issuing engine's target space.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.ordinal.get()
    }

    /// Confirms that `engine` issued this identity.
    ///
    /// # Errors
    ///
    /// Returns [`IdentityFault::ForeignEngine`] when it did not, so a caller
    /// cannot retarget one engine's session with another engine's identity.
    pub fn check_engine(self, engine: EngineId) -> Result<(), IdentityFault> {
        if self.engine == engine {
            Ok(())
        } else {
            Err(IdentityFault::ForeignEngine)
        }
    }

    /// Confirms that `provider` discovered this target.
    ///
    /// # Errors
    ///
    /// Returns [`IdentityFault::ForeignProvider`] when it did not. One engine can
    /// hand out identities from more than one provider, and two providers may
    /// reach the same ordinal, so the engine check alone does not establish that
    /// a provider issued the identity it is being asked to act on.
    pub fn check_provider(self, provider: ProviderId) -> Result<(), IdentityFault> {
        if self.provider == provider {
            Ok(())
        } else {
            Err(IdentityFault::ForeignProvider)
        }
    }

    /// Confirms that `engine` and `provider` together issued this identity.
    ///
    /// This is the check an Adapter performs before acting on a caller's target:
    /// both halves qualify the ordinal, and passing one of them is not enough.
    ///
    /// # Errors
    ///
    /// As [`TargetId::check_engine`] and [`TargetId::check_provider`]. The engine
    /// is checked first, because an identity from another engine says nothing
    /// about what its provider name means.
    pub fn check_issued_by(
        self,
        engine: EngineId,
        provider: ProviderId,
    ) -> Result<(), IdentityFault> {
        self.check_engine(engine)?;
        self.check_provider(provider)
    }
}

impl fmt::Display for TargetId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}/{}/target#{}",
            self.engine, self.provider, self.ordinal
        )
    }
}

/// An opaque handle to one capture stream.
///
/// A stream identity is never reused during an engine's lifetime, so a frame
/// identity from a closed stream can never be mistaken for one from a later
/// stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StreamId {
    engine: EngineId,
    ordinal: NonZeroU64,
}

impl StreamId {
    /// Returns the engine that issued this identity.
    #[must_use]
    pub const fn engine(self) -> EngineId {
        self.engine
    }

    /// Returns the checked nonzero ordinal within the issuing engine's stream space.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.ordinal.get()
    }

    /// Confirms that `engine` issued this identity.
    ///
    /// # Errors
    ///
    /// Returns [`IdentityFault::ForeignEngine`] when it did not.
    pub fn check_engine(self, engine: EngineId) -> Result<(), IdentityFault> {
        if self.engine == engine {
            Ok(())
        } else {
            Err(IdentityFault::ForeignEngine)
        }
    }
}

impl fmt::Display for StreamId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}/stream#{}", self.engine, self.ordinal)
    }
}

/// A stream continuity generation.
///
/// Epoch zero is a stream's first epoch. A discontinuity that makes comparison
/// with the previous frame invalid — a capture restart, or a change of extent or
/// pixel representation — begins a later epoch that is never reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StreamEpoch(u64);

impl StreamEpoch {
    /// A stream's first epoch.
    pub const FIRST: Self = Self(0);

    /// Returns the epoch as a number, for diagnostics and the C ABI.
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }
}

impl fmt::Display for StreamEpoch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// A frame's position within its epoch.
///
/// Sequence zero is an epoch's first published frame, and every frame published
/// after it increments the sequence exactly once — including a frame whose pixels
/// repeat the previous one, because a repeated frame is still a distinct
/// observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FrameSequence(u64);

impl FrameSequence {
    /// An epoch's first frame.
    pub const FIRST: Self = Self(0);

    /// Returns the sequence as a number, for diagnostics and the C ABI.
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }
}

impl fmt::Display for FrameSequence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// The generation of a stream's geometry and coordinate transforms.
///
/// This is correlation metadata: it tells a caller which transform snapshot a
/// result was computed against. It is deliberately **not** a frame-ordering key,
/// because geometry can stay unchanged across many frames and can change without
/// a new frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GeometryRevision(u64);

impl GeometryRevision {
    /// A stream's first geometry revision.
    pub const FIRST: Self = Self(0);

    /// Returns the next revision, or `None` when the counter is exhausted.
    #[must_use]
    pub fn next(self) -> Option<Self> {
        self.0.checked_add(1).map(Self)
    }

    /// Returns the revision as a number, for diagnostics and the C ABI.
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }
}

impl fmt::Display for GeometryRevision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// The relative order of two frames from one stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrameOrder {
    /// The first frame was published before the second.
    Before,
    /// Both identities name the same frame.
    Same,
    /// The first frame was published after the second.
    After,
}

/// The complete public identity of one published frame.
///
/// A frame is identified by its stream, its epoch, its sequence within that
/// epoch, and the geometry revision that was authoritative when it was captured.
/// All four travel with every visual result, so a caller can always say which
/// exact frame an answer came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameStamp {
    stream: StreamId,
    epoch: StreamEpoch,
    sequence: FrameSequence,
    geometry: GeometryRevision,
}

impl FrameStamp {
    /// Assembles a frame stamp.
    ///
    /// Stamps are produced by a [`StreamCursor`], which enforces the epoch and
    /// sequence rules and assembles the stamp here.
    ///
    /// Nothing outside this crate can call it. A stamp needs a [`StreamId`], and
    /// a `StreamId` is issued by an [`IdentityIssuer`] and carries no public
    /// constructor, so its ordinal alone cannot rebuild one after a boundary
    /// drops the engine-qualified type — the C ABI converts a stamp outward and
    /// never back. That is deliberate: an identity a caller could assemble is
    /// an identity that proves nothing about which engine issued it.
    /// A boundary that has to rebuild one needs a way to reconstruct the stream
    /// identity first, and that is the decision to take then rather than a use
    /// this constructor already serves.
    #[must_use]
    pub const fn new(
        stream: StreamId,
        epoch: StreamEpoch,
        sequence: FrameSequence,
        geometry: GeometryRevision,
    ) -> Self {
        Self {
            stream,
            epoch,
            sequence,
            geometry,
        }
    }

    /// Returns the stream that published the frame.
    #[must_use]
    pub const fn stream(&self) -> StreamId {
        self.stream
    }

    /// Returns the stream epoch.
    #[must_use]
    pub const fn epoch(&self) -> StreamEpoch {
        self.epoch
    }

    /// Returns the sequence within the epoch.
    #[must_use]
    pub const fn sequence(&self) -> FrameSequence {
        self.sequence
    }

    /// Returns the geometry revision that was authoritative for the frame.
    #[must_use]
    pub const fn geometry(&self) -> GeometryRevision {
        self.geometry
    }

    /// Reports whether both stamps come from the same stream.
    #[must_use]
    pub fn is_same_stream(&self, other: &Self) -> bool {
        self.stream == other.stream
    }

    /// Orders two frames from one stream by epoch and then sequence.
    ///
    /// # Errors
    ///
    /// Returns [`IdentityFault::StreamMismatch`] when the stamps come from
    /// different streams. Frames from different streams have no order: their
    /// sequences are independent counters and their timestamps come from
    /// different providers, so any answer would be invented.
    ///
    /// This is deliberately not a [`PartialOrd`] implementation. `PartialOrd`
    /// would let `a < b` silently evaluate to `false` for unorderable frames,
    /// which reads as "not before" and is the wrong conclusion.
    pub fn order(&self, other: &Self) -> Result<FrameOrder, IdentityFault> {
        if !self.is_same_stream(other) {
            return Err(IdentityFault::StreamMismatch);
        }
        Ok(
            match (self.epoch, self.sequence).cmp(&(other.epoch, other.sequence)) {
                std::cmp::Ordering::Less => FrameOrder::Before,
                std::cmp::Ordering::Equal => FrameOrder::Same,
                std::cmp::Ordering::Greater => FrameOrder::After,
            },
        )
    }
}

impl fmt::Display for FrameStamp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}@{}.{}~{}",
            self.stream, self.epoch, self.sequence, self.geometry
        )
    }
}

/// Issues the identities one engine hands out.
///
/// Ordinals only ever move forward, so an identity is never reused within an
/// engine's lifetime. Exhaustion is reported rather than wrapped: a wrapped
/// ordinal would alias an identity already given to a caller.
#[derive(Debug)]
pub struct IdentityIssuer {
    engine: EngineId,
    next_target: AtomicU64,
    next_stream: AtomicU64,
}

impl IdentityIssuer {
    /// Creates an issuer for a new engine.
    ///
    /// # Panics
    ///
    /// Panics when more than `u64::MAX` engines have been created in one
    /// process. That is unreachable in practice, and the alternative — handing
    /// out a duplicate [`EngineId`] — would silently break every foreign-engine
    /// check that depends on it.
    #[must_use]
    pub fn new() -> Self {
        static NEXT_ENGINE: AtomicU64 = AtomicU64::new(0);
        let previous = NEXT_ENGINE.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        });
        let ordinal = previous
            .ok()
            .and_then(|previous| NonZeroU64::new(previous.wrapping_add(1)))
            .expect("engine identity space is not exhausted");
        Self {
            engine: EngineId(ordinal),
            next_target: AtomicU64::new(0),
            next_stream: AtomicU64::new(0),
        }
    }

    /// Returns the engine these identities belong to.
    #[must_use]
    pub const fn engine(&self) -> EngineId {
        self.engine
    }

    /// Issues an identity for a target discovered by `provider`.
    ///
    /// # Errors
    ///
    /// Returns [`IdentityFault::Exhausted`] when the target identity space is
    /// exhausted.
    pub fn issue_target(&self, provider: ProviderId) -> Result<TargetId, IdentityFault> {
        Ok(TargetId {
            engine: self.engine,
            provider,
            ordinal: next_ordinal(&self.next_target)?,
        })
    }

    /// Issues an identity for a new capture stream.
    ///
    /// # Errors
    ///
    /// Returns [`IdentityFault::Exhausted`] when the stream identity space is
    /// exhausted.
    pub fn issue_stream(&self) -> Result<StreamId, IdentityFault> {
        Ok(StreamId {
            engine: self.engine,
            ordinal: next_ordinal(&self.next_stream)?,
        })
    }
}

impl Default for IdentityIssuer {
    fn default() -> Self {
        Self::new()
    }
}

fn next_ordinal(counter: &AtomicU64) -> Result<NonZeroU64, IdentityFault> {
    let previous = counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        })
        .map_err(|_| IdentityFault::Exhausted)?;
    // `checked_add` succeeded, so `previous < u64::MAX` and the successor is a
    // valid non-zero ordinal.
    NonZeroU64::new(previous.wrapping_add(1)).ok_or(IdentityFault::Exhausted)
}

/// Assigns epochs and sequences for one stream's published frames.
///
/// The rules that make a frame identity trustworthy live here once, rather than
/// in every capture adapter: sequences start at zero, advance exactly once per
/// published frame, reset when an epoch begins, and never wrap.
#[derive(Debug, Clone)]
pub struct StreamCursor {
    stream: StreamId,
    epoch: StreamEpoch,
    /// The next sequence to hand out, or `None` once the epoch is exhausted.
    next_sequence: Option<u64>,
}

impl StreamCursor {
    /// Starts a cursor at epoch zero, before any frame has been published.
    #[must_use]
    pub const fn new(stream: StreamId) -> Self {
        Self {
            stream,
            epoch: StreamEpoch::FIRST,
            next_sequence: Some(0),
        }
    }

    /// Returns the stream this cursor publishes for.
    #[must_use]
    pub const fn stream(&self) -> StreamId {
        self.stream
    }

    /// Returns the current epoch.
    #[must_use]
    pub const fn epoch(&self) -> StreamEpoch {
        self.epoch
    }

    /// Stamps the next published frame with `geometry`.
    ///
    /// # Errors
    ///
    /// Returns [`IdentityFault::SequenceExhausted`] when the next sequence
    /// cannot be assigned without reusing a value. A stream in that state must
    /// terminate rather than publish an aliased frame identity.
    pub fn publish(&mut self, geometry: GeometryRevision) -> Result<FrameStamp, IdentityFault> {
        let sequence = self.next_sequence.ok_or(IdentityFault::SequenceExhausted)?;
        self.next_sequence = sequence.checked_add(1);
        Ok(FrameStamp::new(
            self.stream,
            self.epoch,
            FrameSequence(sequence),
            geometry,
        ))
    }

    /// Advances past `count` unpublished observations.
    ///
    /// Native adapters use this to make bounded producer drops observable as a
    /// gap in the next published sequence without inventing frame stamps or
    /// iterating once per drop.
    ///
    /// # Errors
    ///
    /// Returns [`IdentityFault::SequenceExhausted`] when the skipped range would
    /// exhaust or wrap the current epoch.
    pub fn skip(&mut self, count: u64) -> Result<(), IdentityFault> {
        if count == 0 {
            return Ok(());
        }
        let sequence = self.next_sequence.ok_or(IdentityFault::SequenceExhausted)?;
        self.next_sequence = Some(
            sequence
                .checked_add(count)
                .ok_or(IdentityFault::SequenceExhausted)?,
        );
        Ok(())
    }

    /// Begins a later epoch after a discontinuity, resetting the sequence.
    ///
    /// # Errors
    ///
    /// Returns [`IdentityFault::EpochExhausted`] when the epoch counter cannot
    /// advance without reusing a value.
    pub fn begin_epoch(&mut self) -> Result<StreamEpoch, IdentityFault> {
        let epoch = self
            .epoch
            .0
            .checked_add(1)
            .ok_or(IdentityFault::EpochExhausted)?;
        self.epoch = StreamEpoch(epoch);
        self.next_sequence = Some(0);
        Ok(self.epoch)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        FrameOrder, FrameSequence, FrameStamp, GeometryRevision, IdentityFault, IdentityIssuer,
        ProviderId, StreamCursor, StreamEpoch,
    };
    use crate::status::{Error, Status};

    const REPLAY: ProviderId = ProviderId::new("replay");
    const WINDOWS: ProviderId = ProviderId::new("windows");

    #[test]
    fn identical_target_metadata_still_yields_distinct_identities() {
        let issuer = IdentityIssuer::new();

        let first = issuer.issue_target(REPLAY).expect("issued");
        let second = issuer.issue_target(REPLAY).expect("issued");

        assert_ne!(first, second);
        assert_eq!(first.provider(), second.provider());
    }

    #[test]
    fn target_ordinals_are_nonzero_and_engine_local() {
        let first_engine = IdentityIssuer::new();
        let second_engine = IdentityIssuer::new();

        assert_eq!(first_engine.issue_target(REPLAY).expect("issued").get(), 1);
        assert_eq!(first_engine.issue_target(REPLAY).expect("issued").get(), 2);
        assert_eq!(second_engine.issue_target(REPLAY).expect("issued").get(), 1);
    }

    #[test]
    fn a_target_identity_is_rejected_by_another_engine() {
        let issuing = IdentityIssuer::new();
        let other = IdentityIssuer::new();
        let target = issuing.issue_target(REPLAY).expect("issued");

        assert_eq!(target.check_engine(issuing.engine()), Ok(()));
        assert_eq!(
            target.check_engine(other.engine()),
            Err(IdentityFault::ForeignEngine)
        );
    }

    #[test]
    fn a_target_identity_is_rejected_by_another_provider_of_the_same_engine() {
        let issuer = IdentityIssuer::new();
        let replay = issuer.issue_target(REPLAY).expect("issued");

        assert_eq!(replay.check_provider(REPLAY), Ok(()));
        assert_eq!(
            replay.check_provider(WINDOWS),
            Err(IdentityFault::ForeignProvider),
            "the engine check alone does not qualify the ordinal"
        );
        assert_eq!(replay.check_issued_by(issuer.engine(), REPLAY), Ok(()));
        assert_eq!(
            replay.check_issued_by(issuer.engine(), WINDOWS),
            Err(IdentityFault::ForeignProvider)
        );
    }

    #[test]
    fn a_foreign_engine_is_reported_before_the_provider() {
        let issuing = IdentityIssuer::new();
        let other = IdentityIssuer::new();
        let target = issuing.issue_target(REPLAY).expect("issued");

        assert_eq!(
            target.check_issued_by(other.engine(), WINDOWS),
            Err(IdentityFault::ForeignEngine),
            "a provider name from another engine says nothing"
        );
    }

    #[test]
    fn a_stream_identity_is_rejected_by_another_engine() {
        let issuing = IdentityIssuer::new();
        let other = IdentityIssuer::new();
        let stream = issuing.issue_stream().expect("issued");

        assert_eq!(stream.check_engine(issuing.engine()), Ok(()));
        assert_eq!(
            stream.check_engine(other.engine()),
            Err(IdentityFault::ForeignEngine)
        );
    }

    #[test]
    fn stream_ordinals_are_nonzero_and_engine_local() {
        let first_engine = IdentityIssuer::new();
        let second_engine = IdentityIssuer::new();

        assert_eq!(first_engine.issue_stream().expect("issued").get(), 1);
        assert_eq!(first_engine.issue_stream().expect("issued").get(), 2);
        assert_eq!(second_engine.issue_stream().expect("issued").get(), 1);
    }

    #[test]
    fn two_engines_never_share_an_identity() {
        let first = IdentityIssuer::new();
        let second = IdentityIssuer::new();

        assert_ne!(first.engine(), second.engine());
        assert_ne!(
            first.issue_stream().expect("issued"),
            second.issue_stream().expect("issued")
        );
    }

    #[test]
    fn providers_qualify_a_target_identity() {
        let issuer = IdentityIssuer::new();

        let replay = issuer.issue_target(REPLAY).expect("issued");
        let windows = issuer.issue_target(WINDOWS).expect("issued");

        assert_ne!(replay, windows);
        assert_eq!(replay.provider().name(), "replay");
        assert_eq!(windows.provider().name(), "windows");
    }

    #[test]
    fn the_first_published_frame_is_epoch_zero_sequence_zero() {
        let issuer = IdentityIssuer::new();
        let mut cursor = StreamCursor::new(issuer.issue_stream().expect("issued"));

        let stamp = cursor.publish(GeometryRevision::FIRST).expect("published");

        assert_eq!(stamp.epoch(), StreamEpoch::FIRST);
        assert_eq!(stamp.sequence(), FrameSequence::FIRST);
    }

    #[test]
    fn a_repeated_frame_still_advances_the_sequence() {
        let issuer = IdentityIssuer::new();
        let mut cursor = StreamCursor::new(issuer.issue_stream().expect("issued"));

        let first = cursor.publish(GeometryRevision::FIRST).expect("published");
        let second = cursor.publish(GeometryRevision::FIRST).expect("published");

        assert_ne!(first, second);
        assert_eq!(second.sequence().value(), first.sequence().value() + 1);
        assert_eq!(first.order(&second), Ok(FrameOrder::Before));
    }

    #[test]
    fn skipped_observations_become_a_sequence_gap_without_iteration() {
        let issuer = IdentityIssuer::new();
        let mut cursor = StreamCursor::new(issuer.issue_stream().expect("issued"));

        let first = cursor.publish(GeometryRevision::FIRST).expect("published");
        cursor.skip(3).expect("three bounded drops");
        let after_gap = cursor.publish(GeometryRevision::FIRST).expect("published");

        assert_eq!(first.sequence().value(), 0);
        assert_eq!(after_gap.sequence().value(), 4);
    }

    #[test]
    fn a_discontinuity_begins_a_later_epoch_at_sequence_zero() {
        let issuer = IdentityIssuer::new();
        let mut cursor = StreamCursor::new(issuer.issue_stream().expect("issued"));
        cursor.publish(GeometryRevision::FIRST).expect("published");
        cursor.publish(GeometryRevision::FIRST).expect("published");

        let epoch = cursor.begin_epoch().expect("advanced");
        let stamp = cursor.publish(GeometryRevision::FIRST).expect("published");

        assert_eq!(epoch.value(), 1);
        assert_eq!(stamp.epoch(), epoch);
        assert_eq!(stamp.sequence(), FrameSequence::FIRST);
    }

    #[test]
    fn a_later_epoch_orders_after_an_earlier_one_despite_a_lower_sequence() {
        let issuer = IdentityIssuer::new();
        let mut cursor = StreamCursor::new(issuer.issue_stream().expect("issued"));
        cursor.publish(GeometryRevision::FIRST).expect("published");
        let late_in_first_epoch = cursor.publish(GeometryRevision::FIRST).expect("published");
        cursor.begin_epoch().expect("advanced");
        let first_in_second_epoch = cursor.publish(GeometryRevision::FIRST).expect("published");

        assert!(first_in_second_epoch.sequence() < late_in_first_epoch.sequence());
        assert_eq!(
            late_in_first_epoch.order(&first_in_second_epoch),
            Ok(FrameOrder::Before)
        );
    }

    #[test]
    fn geometry_revision_does_not_affect_ordering() {
        let issuer = IdentityIssuer::new();
        let stream = issuer.issue_stream().expect("issued");
        let early_geometry = FrameStamp::new(
            stream,
            StreamEpoch::FIRST,
            FrameSequence::FIRST,
            GeometryRevision::FIRST,
        );
        let later_geometry = FrameStamp::new(
            stream,
            StreamEpoch::FIRST,
            FrameSequence::FIRST,
            GeometryRevision::FIRST.next().expect("representable"),
        );

        assert_eq!(early_geometry.order(&later_geometry), Ok(FrameOrder::Same));
        assert_ne!(
            early_geometry, later_geometry,
            "the stamps still differ; only the ordering key ignores geometry"
        );
    }

    #[test]
    fn frames_from_different_streams_are_not_orderable() {
        let issuer = IdentityIssuer::new();
        let mut first = StreamCursor::new(issuer.issue_stream().expect("issued"));
        let mut second = StreamCursor::new(issuer.issue_stream().expect("issued"));
        let from_first = first.publish(GeometryRevision::FIRST).expect("published");
        second.publish(GeometryRevision::FIRST).expect("published");
        let from_second = second.publish(GeometryRevision::FIRST).expect("published");

        assert!(!from_first.is_same_stream(&from_second));
        assert_eq!(
            from_first.order(&from_second),
            Err(IdentityFault::StreamMismatch)
        );
    }

    #[test]
    fn an_exhausted_sequence_terminates_the_stream_instead_of_wrapping() {
        let issuer = IdentityIssuer::new();
        let mut cursor = StreamCursor::new(issuer.issue_stream().expect("issued"));
        // Drive the cursor to the last representable sequence without publishing
        // `u64::MAX` frames.
        cursor.next_sequence = Some(u64::MAX);

        let last = cursor.publish(GeometryRevision::FIRST).expect("published");

        assert_eq!(last.sequence().value(), u64::MAX);
        assert_eq!(
            cursor.publish(GeometryRevision::FIRST),
            Err(IdentityFault::SequenceExhausted),
            "the frame that would alias is refused, not renumbered"
        );
    }

    #[test]
    fn an_exhausted_epoch_counter_is_reported() {
        let issuer = IdentityIssuer::new();
        let mut cursor = StreamCursor::new(issuer.issue_stream().expect("issued"));
        cursor.epoch = StreamEpoch(u64::MAX);

        assert_eq!(cursor.begin_epoch(), Err(IdentityFault::EpochExhausted));
    }

    #[test]
    fn beginning_an_epoch_recovers_an_exhausted_sequence() {
        let issuer = IdentityIssuer::new();
        let mut cursor = StreamCursor::new(issuer.issue_stream().expect("issued"));
        cursor.next_sequence = None;

        cursor.begin_epoch().expect("advanced");

        assert_eq!(
            cursor
                .publish(GeometryRevision::FIRST)
                .expect("published")
                .sequence(),
            FrameSequence::FIRST
        );
    }

    #[test]
    fn faults_map_to_public_statuses() {
        assert_eq!(
            IdentityFault::ForeignEngine.status(),
            Status::InvalidArgument
        );
        assert_eq!(
            IdentityFault::ForeignProvider.status(),
            Status::InvalidArgument
        );
        assert_eq!(
            IdentityFault::StreamMismatch.status(),
            Status::InvalidArgument
        );
        assert_eq!(IdentityFault::Exhausted.status(), Status::LimitExceeded);
        assert_eq!(
            IdentityFault::SequenceExhausted.status(),
            Status::LimitExceeded
        );
        assert_eq!(
            IdentityFault::EpochExhausted.status(),
            Status::LimitExceeded
        );

        let error: Error = IdentityFault::ForeignEngine.into();
        assert_eq!(error.status(), Status::InvalidArgument);
        assert!(!error.detail().is_empty());
    }

    #[test]
    fn geometry_revisions_advance_without_wrapping() {
        assert_eq!(GeometryRevision::FIRST.value(), 0);
        assert_eq!(
            GeometryRevision::FIRST
                .next()
                .expect("representable")
                .value(),
            1
        );
        assert_eq!(GeometryRevision(u64::MAX).next(), None);
    }

    #[test]
    fn display_forms_stay_diagnostic_only() {
        let issuer = IdentityIssuer::new();
        let mut cursor = StreamCursor::new(issuer.issue_stream().expect("issued"));
        let stamp = cursor.publish(GeometryRevision::FIRST).expect("published");
        let text = stamp.to_string();

        assert!(text.contains("stream#"), "{text}");
        assert!(text.contains("@0.0~0"), "{text}");
    }
}
