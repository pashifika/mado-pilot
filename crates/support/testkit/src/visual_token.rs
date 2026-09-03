//! Platform-independent logical encoding for native qualification visual tokens.
//!
//! The repository-owned Windows and macOS fixtures render this grid beside the
//! template-search marker. The control acknowledgement identifies the requested
//! token, while decoding the same token from captured pixels proves data-plane
//! progress. This module deliberately owns only logical cells; each platform
//! chooses physical cell size and pixel tolerance for its approved display scale.

use std::error::Error;
use std::fmt;
use std::num::NonZeroU32;

/// Logical width of the visual-token grid, in cells.
pub const VISUAL_TOKEN_GRID_WIDTH: usize = 10;
/// Logical height of the visual-token grid, in cells.
pub const VISUAL_TOKEN_GRID_HEIGHT: usize = 9;
/// Total number of logical cells in one encoded visual token.
pub const VISUAL_TOKEN_CELL_COUNT: usize = VISUAL_TOKEN_GRID_WIDTH * VISUAL_TOKEN_GRID_HEIGHT;

const TOKEN_BITS: usize = 32;
const CHECKSUM_BITS: usize = 5;
const PAYLOAD_START: usize = VISUAL_TOKEN_GRID_WIDTH;
const PAYLOAD_END: usize = VISUAL_TOKEN_CELL_COUNT - VISUAL_TOKEN_GRID_WIDTH;
const INVERSE_START: usize = PAYLOAD_START + TOKEN_BITS;
const MARKER_INDEX: usize = INVERSE_START + TOKEN_BITS;
const CHECKSUM_START: usize = MARKER_INDEX + 1;
const TOP_SENTINEL: [bool; VISUAL_TOKEN_GRID_WIDTH] = [
    true, false, true, true, false, false, true, false, false, true,
];
const BOTTOM_SENTINEL: [bool; VISUAL_TOKEN_GRID_WIDTH] = [
    false, false, true, false, true, true, true, false, true, false,
];

const _: () = assert!(CHECKSUM_START + CHECKSUM_BITS == PAYLOAD_END);

/// Marker state committed in the same render transaction as a visual token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VisualMarkerState {
    /// The template-search marker is absent.
    Absent,
    /// The template-search marker is visible.
    Visible,
}

impl VisualMarkerState {
    /// Returns whether the template-search marker must be visible.
    #[must_use]
    pub const fn is_visible(self) -> bool {
        matches!(self, Self::Visible)
    }
}

/// One nonzero logical token and its correlated marker state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VisualToken {
    value: NonZeroU32,
    marker: VisualMarkerState,
}

impl VisualToken {
    /// Creates a token, returning `None` for the reserved zero value.
    #[must_use]
    pub const fn new(value: u32, marker: VisualMarkerState) -> Option<Self> {
        match NonZeroU32::new(value) {
            Some(value) => Some(Self { value, marker }),
            None => None,
        }
    }

    /// Returns the nonzero token value carried by this state.
    #[must_use]
    pub const fn value(self) -> u32 {
        self.value.get()
    }

    /// Returns the marker state correlated with this token.
    #[must_use]
    pub const fn marker(self) -> VisualMarkerState {
        self.marker
    }

    /// Encodes the token as fixed orientation rows and a validated payload.
    ///
    /// `true` denotes the primary cell colour and `false` the secondary colour.
    #[must_use]
    pub fn encode(self) -> [bool; VISUAL_TOKEN_CELL_COUNT] {
        encode_value(self.value(), self.marker)
    }

    /// Decodes and validates one complete logical cell grid.
    ///
    /// The decoder rejects the reserved value, wrong orientation, a corrupt
    /// token/inverse pair, and a marker or payload that disagrees with the
    /// checksum.
    pub fn decode(cells: &[bool]) -> Result<Self, VisualTokenDecodeError> {
        if cells.len() != VISUAL_TOKEN_CELL_COUNT {
            return Err(VisualTokenDecodeError::CellCount {
                expected: VISUAL_TOKEN_CELL_COUNT,
                actual: cells.len(),
            });
        }
        if cells[..VISUAL_TOKEN_GRID_WIDTH] != TOP_SENTINEL
            || cells[PAYLOAD_END..] != BOTTOM_SENTINEL
        {
            return Err(VisualTokenDecodeError::Sentinel);
        }

        let value = read_bits(cells, PAYLOAD_START, TOKEN_BITS);
        let inverse = read_bits(cells, INVERSE_START, TOKEN_BITS);
        if value == 0 {
            return Err(VisualTokenDecodeError::Reserved);
        }
        if inverse != !value {
            return Err(VisualTokenDecodeError::Inverse);
        }

        let marker = if cells[MARKER_INDEX] {
            VisualMarkerState::Visible
        } else {
            VisualMarkerState::Absent
        };
        let observed_checksum = u8::try_from(read_bits(cells, CHECKSUM_START, CHECKSUM_BITS))
            .expect("five checksum bits fit in u8");
        if observed_checksum != checksum(value, marker) {
            return Err(VisualTokenDecodeError::Checksum);
        }

        Ok(Self {
            value: NonZeroU32::new(value).expect("zero was rejected above"),
            marker,
        })
    }
}

/// Why a logical visual-token grid was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisualTokenDecodeError {
    /// The caller supplied a grid with the wrong number of cells.
    CellCount {
        /// Required number of cells.
        expected: usize,
        /// Observed number of cells.
        actual: usize,
    },
    /// One or both fixed orientation rows were invalid.
    Sentinel,
    /// The decoded token used the reserved zero value.
    Reserved,
    /// The token and its encoded inverse disagreed.
    Inverse,
    /// The token, marker state, and checksum disagreed.
    Checksum,
}

impl fmt::Display for VisualTokenDecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CellCount { expected, actual } => {
                write!(
                    formatter,
                    "expected {expected} visual-token cells, got {actual}"
                )
            }
            Self::Sentinel => formatter.write_str("visual-token orientation sentinel mismatch"),
            Self::Reserved => formatter.write_str("visual-token zero is reserved"),
            Self::Inverse => formatter.write_str("visual-token inverse mismatch"),
            Self::Checksum => formatter.write_str("visual-token checksum mismatch"),
        }
    }
}

impl Error for VisualTokenDecodeError {}

/// Monotonic token issuer that refuses reuse after `u32::MAX`.
#[derive(Debug)]
pub struct VisualTokenSequence {
    next: Option<NonZeroU32>,
}

impl VisualTokenSequence {
    /// Creates a sequence whose first issued token is one.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            next: NonZeroU32::new(1),
        }
    }

    /// Issues one token for `marker`, or reports permanent exhaustion.
    pub fn issue(
        &mut self,
        marker: VisualMarkerState,
    ) -> Result<VisualToken, VisualTokenExhausted> {
        let value = self.next.ok_or(VisualTokenExhausted)?;
        self.next = value.get().checked_add(1).and_then(NonZeroU32::new);
        Ok(VisualToken { value, marker })
    }
}

impl Default for VisualTokenSequence {
    fn default() -> Self {
        Self::new()
    }
}

/// The fixture-lifetime visual-token sequence has no unused value remaining.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisualTokenExhausted;

impl fmt::Display for VisualTokenExhausted {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("visual-token sequence exhausted")
    }
}

impl Error for VisualTokenExhausted {}

fn encode_value(value: u32, marker: VisualMarkerState) -> [bool; VISUAL_TOKEN_CELL_COUNT] {
    let mut cells = [false; VISUAL_TOKEN_CELL_COUNT];
    cells[..VISUAL_TOKEN_GRID_WIDTH].copy_from_slice(&TOP_SENTINEL);
    cells[PAYLOAD_END..].copy_from_slice(&BOTTOM_SENTINEL);
    write_bits(&mut cells, PAYLOAD_START, TOKEN_BITS, u64::from(value));
    write_bits(&mut cells, INVERSE_START, TOKEN_BITS, u64::from(!value));
    cells[MARKER_INDEX] = marker.is_visible();
    write_bits(
        &mut cells,
        CHECKSUM_START,
        CHECKSUM_BITS,
        u64::from(checksum(value, marker)),
    );
    cells
}

fn write_bits(cells: &mut [bool; VISUAL_TOKEN_CELL_COUNT], start: usize, bits: usize, value: u64) {
    for bit in 0..bits {
        cells[start + bit] = value & (1_u64 << bit) != 0;
    }
}

fn read_bits(cells: &[bool], start: usize, bits: usize) -> u32 {
    let mut value = 0_u32;
    for bit in 0..bits {
        value |= u32::from(cells[start + bit]) << bit;
    }
    value
}

fn checksum(value: u32, marker: VisualMarkerState) -> u8 {
    let marker_salt = if marker.is_visible() {
        0x15a5_3c6d
    } else {
        0x0a5a_c396
    };
    let mut mixed = value ^ value.rotate_left(7) ^ value.rotate_right(11) ^ marker_salt;
    mixed ^= mixed >> 16;
    mixed ^= mixed >> 8;
    u8::try_from(mixed & 0x1f).expect("five checksum bits fit in u8")
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;

    use super::{
        PAYLOAD_END, PAYLOAD_START, VISUAL_TOKEN_CELL_COUNT, VISUAL_TOKEN_GRID_HEIGHT,
        VISUAL_TOKEN_GRID_WIDTH, VisualMarkerState, VisualToken, VisualTokenDecodeError,
        VisualTokenExhausted, VisualTokenSequence, encode_value,
    };

    #[test]
    fn every_valid_marker_state_and_boundary_token_round_trips() {
        for value in [1, 2, 0x55aa_33cc, u32::MAX] {
            for marker in [VisualMarkerState::Absent, VisualMarkerState::Visible] {
                let token = VisualToken::new(value, marker).expect("nonzero token");

                assert_eq!(VisualToken::decode(&token.encode()), Ok(token));
            }
        }
    }

    #[test]
    fn zero_is_reserved_for_construction_and_decoding() {
        assert_eq!(VisualToken::new(0, VisualMarkerState::Absent), None);
        assert_eq!(
            VisualToken::decode(&encode_value(0, VisualMarkerState::Visible)),
            Err(VisualTokenDecodeError::Reserved)
        );
    }

    #[test]
    fn every_single_cell_corruption_is_rejected() {
        let token =
            VisualToken::new(0x55aa_33cc, VisualMarkerState::Visible).expect("nonzero token");
        let encoded = token.encode();

        for index in 0..VISUAL_TOKEN_CELL_COUNT {
            let mut corrupted = encoded;
            corrupted[index] = !corrupted[index];
            assert!(
                VisualToken::decode(&corrupted).is_err(),
                "cell {index} corruption survived"
            );
        }
    }

    #[test]
    fn partial_old_to_new_transitions_are_rejected() {
        let old = VisualToken::new(41, VisualMarkerState::Absent)
            .expect("nonzero token")
            .encode();
        let new = VisualToken::new(42, VisualMarkerState::Visible)
            .expect("nonzero token")
            .encode();

        for split in PAYLOAD_START + 1..PAYLOAD_END {
            let mut partial = old;
            partial[..split].copy_from_slice(&new[..split]);
            if partial != old && partial != new {
                assert!(
                    VisualToken::decode(&partial).is_err(),
                    "partial transition at cell {split} survived"
                );
            }
        }
    }

    #[test]
    fn stale_tokens_do_not_equal_the_expected_state() {
        let stale = VisualToken::new(41, VisualMarkerState::Visible).expect("nonzero token");
        let expected = VisualToken::new(42, VisualMarkerState::Visible).expect("nonzero token");

        assert_eq!(VisualToken::decode(&stale.encode()), Ok(stale));
        assert_ne!(stale, expected);
    }

    #[test]
    fn flipped_or_rotated_grids_fail_orientation_validation() {
        let encoded = VisualToken::new(7, VisualMarkerState::Visible)
            .expect("nonzero token")
            .encode();

        for transformed in [
            flip_horizontal(&encoded),
            flip_vertical(&encoded),
            rotate_half_turn(&encoded),
        ] {
            assert_eq!(
                VisualToken::decode(&transformed),
                Err(VisualTokenDecodeError::Sentinel)
            );
        }
    }

    #[test]
    fn a_sequence_issues_nonzero_tokens_and_refuses_exhaustion() {
        let mut sequence = VisualTokenSequence::new();
        assert_eq!(
            sequence.issue(VisualMarkerState::Absent),
            VisualToken::new(1, VisualMarkerState::Absent).ok_or(VisualTokenExhausted)
        );
        assert_eq!(
            sequence.issue(VisualMarkerState::Visible),
            VisualToken::new(2, VisualMarkerState::Visible).ok_or(VisualTokenExhausted)
        );

        let mut final_token = VisualTokenSequence {
            next: NonZeroU32::new(u32::MAX),
        };
        assert_eq!(
            final_token.issue(VisualMarkerState::Visible),
            VisualToken::new(u32::MAX, VisualMarkerState::Visible).ok_or(VisualTokenExhausted)
        );
        assert_eq!(
            final_token.issue(VisualMarkerState::Absent),
            Err(VisualTokenExhausted)
        );
    }

    fn flip_horizontal(cells: &[bool; VISUAL_TOKEN_CELL_COUNT]) -> [bool; VISUAL_TOKEN_CELL_COUNT] {
        transform(cells, |row, column| {
            (row, VISUAL_TOKEN_GRID_WIDTH - 1 - column)
        })
    }

    fn flip_vertical(cells: &[bool; VISUAL_TOKEN_CELL_COUNT]) -> [bool; VISUAL_TOKEN_CELL_COUNT] {
        transform(cells, |row, column| {
            (VISUAL_TOKEN_GRID_HEIGHT - 1 - row, column)
        })
    }

    fn rotate_half_turn(
        cells: &[bool; VISUAL_TOKEN_CELL_COUNT],
    ) -> [bool; VISUAL_TOKEN_CELL_COUNT] {
        transform(cells, |row, column| {
            (
                VISUAL_TOKEN_GRID_HEIGHT - 1 - row,
                VISUAL_TOKEN_GRID_WIDTH - 1 - column,
            )
        })
    }

    fn transform(
        cells: &[bool; VISUAL_TOKEN_CELL_COUNT],
        source: impl Fn(usize, usize) -> (usize, usize),
    ) -> [bool; VISUAL_TOKEN_CELL_COUNT] {
        let mut transformed = [false; VISUAL_TOKEN_CELL_COUNT];
        for row in 0..VISUAL_TOKEN_GRID_HEIGHT {
            for column in 0..VISUAL_TOKEN_GRID_WIDTH {
                let (source_row, source_column) = source(row, column);
                transformed[row * VISUAL_TOKEN_GRID_WIDTH + column] =
                    cells[source_row * VISUAL_TOKEN_GRID_WIDTH + source_column];
            }
        }
        transformed
    }
}
