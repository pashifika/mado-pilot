//! The primitives one input sequence is made of, and the bounds they obey.
//!
//! Every event here is one irreversible act, which is why the sequence that holds
//! them is bounded and why a receipt counts them. An operating system cannot take
//! a delivered keystroke back, so the contract does not pretend a sequence is
//! atomic; it makes exactly how far one got observable instead.

use std::fmt;
use std::time::Duration;

use mado_pilot_core::{InputOperationKind, Point};

use crate::fault::InputFault;

/// A pointer button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum PointerButton {
    /// The primary button, wherever the user's settings put it.
    Primary,
    /// The secondary button, which ordinarily opens a context menu.
    Secondary,
    /// The middle button or wheel click.
    Middle,
}

impl PointerButton {
    /// Every button version one delivers.
    pub const ALL: [Self; 3] = [
        PointerButton::Primary,
        PointerButton::Secondary,
        PointerButton::Middle,
    ];

    /// Returns a stable lowercase slug, for logs and the C ABI mapping.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            PointerButton::Primary => "primary",
            PointerButton::Secondary => "secondary",
            PointerButton::Middle => "middle",
        }
    }
}

impl fmt::Display for PointerButton {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A keyboard modifier, as a key rather than as a state.
///
/// A modifier is pressed and released like any other key, because a sequence that
/// declared modifier *state* alongside each keystroke could not express holding a
/// modifier across several of them, and could not report which half of a press and
/// release had been delivered when a sequence stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Modifier {
    /// Shift.
    Shift,
    /// Control.
    Control,
    /// Alt, or Option.
    Alt,
    /// The platform's command or Windows key.
    Meta,
}

impl Modifier {
    /// Every modifier version one delivers.
    pub const ALL: [Self; 4] = [
        Modifier::Shift,
        Modifier::Control,
        Modifier::Alt,
        Modifier::Meta,
    ];

    /// Returns a stable lowercase slug, for logs and the C ABI mapping.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Modifier::Shift => "shift",
            Modifier::Control => "control",
            Modifier::Alt => "alt",
            Modifier::Meta => "meta",
        }
    }
}

impl fmt::Display for Modifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One key a sequence can press or release.
///
/// The set is deliberately small and logical rather than a scancode table. A
/// caller says what it means; the Adapter resolves that through the target's
/// active layout and reports [`InputFault::UnsupportedCombination`] when it
/// cannot. A scancode contract would have made the caller responsible for a
/// keyboard layout it cannot see.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Key {
    /// A printable character, resolved through the target's active layout.
    Character(char),
    /// A function key, numbered from one.
    Function(u8),
    /// A modifier, pressed and released like any other key.
    Modifier(Modifier),
    /// Return or Enter.
    Enter,
    /// Tab.
    Tab,
    /// Backspace.
    Backspace,
    /// Forward delete.
    Delete,
    /// Escape.
    Escape,
    /// The space bar.
    Space,
    /// Arrow up.
    ArrowUp,
    /// Arrow down.
    ArrowDown,
    /// Arrow left.
    ArrowLeft,
    /// Arrow right.
    ArrowRight,
    /// Home.
    Home,
    /// End.
    End,
    /// Page up.
    PageUp,
    /// Page down.
    PageDown,
}

impl Key {
    /// The highest function-key number version one accepts.
    ///
    /// Twenty-four is the extent of the function-key range both release targets
    /// define. A number above it names no key, so it is refused rather than
    /// delivered as whatever the platform does with an out-of-range code.
    pub const MAX_FUNCTION: u8 = 24;

    /// Checks that the key names something deliverable.
    ///
    /// # Errors
    ///
    /// Returns [`InputFault::SequenceOutOfBounds`] for a function-key number
    /// outside `1..=24`, and for a character that is a control code: a control
    /// character is the encoding of a key combination rather than a key, and
    /// delivering one as a character would produce whatever the target made of the
    /// byte.
    pub fn check(self) -> Result<(), InputFault> {
        match self {
            Key::Function(number) if number == 0 || number > Self::MAX_FUNCTION => {
                Err(InputFault::SequenceOutOfBounds)
            }
            Key::Character(character) if character.is_control() => {
                Err(InputFault::SequenceOutOfBounds)
            }
            _ => Ok(()),
        }
    }
}

impl fmt::Display for Key {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Key::Character(character) => write!(formatter, "char({character})"),
            Key::Function(number) => write!(formatter, "f{number}"),
            Key::Modifier(modifier) => write!(formatter, "{modifier}"),
            Key::Enter => formatter.write_str("enter"),
            Key::Tab => formatter.write_str("tab"),
            Key::Backspace => formatter.write_str("backspace"),
            Key::Delete => formatter.write_str("delete"),
            Key::Escape => formatter.write_str("escape"),
            Key::Space => formatter.write_str("space"),
            Key::ArrowUp => formatter.write_str("arrow_up"),
            Key::ArrowDown => formatter.write_str("arrow_down"),
            Key::ArrowLeft => formatter.write_str("arrow_left"),
            Key::ArrowRight => formatter.write_str("arrow_right"),
            Key::Home => formatter.write_str("home"),
            Key::End => formatter.write_str("end"),
            Key::PageUp => formatter.write_str("page_up"),
            Key::PageDown => formatter.write_str("page_down"),
        }
    }
}

/// One event a controller delivers, or one wait between two of them.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum InputEvent {
    /// Move the pointer to `position`.
    PointerMove(Point),
    /// Press a pointer button where the pointer is.
    PointerPress(PointerButton),
    /// Release a pointer button where the pointer is.
    PointerRelease(PointerButton),
    /// Scroll by a bounded number of wheel notches on either axis.
    PointerScroll {
        /// Notches to the right, negative for left.
        horizontal: i16,
        /// Notches down, negative for up.
        vertical: i16,
    },
    /// Press a key.
    KeyPress(Key),
    /// Release a key.
    KeyRelease(Key),
    /// Enter `text` as text rather than as key codes.
    Text(String),
    /// Wait before the next event.
    ///
    /// A delay is an event because it is a point where the operation's deadline
    /// and cancellation are checked, and because a sequence that stopped during
    /// one has delivered a countable number of events.
    Delay(Duration),
}

impl InputEvent {
    /// The longest single delay version one accepts.
    ///
    /// A caller that wants to wait longer waits outside the sequence, where its
    /// own deadline governs. A bound here keeps one admitted sequence from holding
    /// a controller for an unbounded time even when the caller set no deadline.
    pub const MAX_DELAY: Duration = Duration::from_secs(5);

    /// The most characters one text event may carry.
    pub const MAX_TEXT_CHARS: usize = 4096;

    /// The largest scroll magnitude one event may carry.
    pub const MAX_SCROLL_NOTCHES: i16 = 120;

    /// Returns which input operation this event performs.
    ///
    /// A delay performs none: it is part of a sequence's shape rather than
    /// something a target has to support.
    #[must_use]
    pub const fn operation_kind(&self) -> Option<InputOperationKind> {
        match self {
            InputEvent::PointerMove(_)
            | InputEvent::PointerPress(_)
            | InputEvent::PointerRelease(_)
            | InputEvent::PointerScroll { .. } => Some(InputOperationKind::Pointer),
            InputEvent::KeyPress(_) | InputEvent::KeyRelease(_) => {
                Some(InputOperationKind::Keyboard)
            }
            InputEvent::Text(_) => Some(InputOperationKind::Text),
            InputEvent::Delay(_) => None,
        }
    }

    /// Reports whether delivering this event changes observable state.
    ///
    /// The deadline is checked before every irreversible event. A delay is the one
    /// that is not: it can be cut short and leave nothing behind.
    #[must_use]
    pub const fn is_irreversible(&self) -> bool {
        !matches!(self, InputEvent::Delay(_))
    }

    /// Returns the state this event leaves pressed, for cleanup to release.
    ///
    /// Cleanup releases only what the sequence itself pressed, so it needs to know
    /// which events press something and which do not. A move, a scroll, a text
    /// entry, and a delay leave nothing held.
    #[must_use]
    pub const fn presses(&self) -> Option<PressedState> {
        match self {
            InputEvent::PointerPress(button) => Some(PressedState::Button(*button)),
            InputEvent::KeyPress(key) => Some(PressedState::Key(*key)),
            _ => None,
        }
    }

    /// Returns the state this event releases.
    #[must_use]
    pub const fn releases(&self) -> Option<PressedState> {
        match self {
            InputEvent::PointerRelease(button) => Some(PressedState::Button(*button)),
            InputEvent::KeyRelease(key) => Some(PressedState::Key(*key)),
            _ => None,
        }
    }

    /// Checks that the event is within its own bounds.
    ///
    /// # Errors
    ///
    /// Returns [`InputFault::SequenceOutOfBounds`] for a delay longer than
    /// [`InputEvent::MAX_DELAY`], text longer than
    /// [`InputEvent::MAX_TEXT_CHARS`], empty text, a scroll beyond
    /// [`InputEvent::MAX_SCROLL_NOTCHES`], a scroll of nothing, and an
    /// undeliverable key.
    pub fn check(&self) -> Result<(), InputFault> {
        match self {
            InputEvent::Delay(delay) if *delay > Self::MAX_DELAY => {
                Err(InputFault::SequenceOutOfBounds)
            }
            InputEvent::Text(text)
                if text.is_empty() || text.chars().count() > Self::MAX_TEXT_CHARS =>
            {
                Err(InputFault::SequenceOutOfBounds)
            }
            InputEvent::PointerScroll {
                horizontal,
                vertical,
            } if (*horizontal == 0 && *vertical == 0)
                || horizontal.unsigned_abs() > Self::MAX_SCROLL_NOTCHES.unsigned_abs()
                || vertical.unsigned_abs() > Self::MAX_SCROLL_NOTCHES.unsigned_abs() =>
            {
                Err(InputFault::SequenceOutOfBounds)
            }
            InputEvent::KeyPress(key) | InputEvent::KeyRelease(key) => key.check(),
            _ => Ok(()),
        }
    }
}

/// One thing a sequence pressed and has not released.
///
/// Cleanup after a partial failure releases exactly these, which is why they are a
/// type rather than a pair of counters: releasing "two buttons" cannot be done
/// without knowing which two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PressedState {
    /// A pointer button.
    Button(PointerButton),
    /// A key, including a modifier.
    Key(Key),
}

impl fmt::Display for PressedState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PressedState::Button(button) => write!(formatter, "button({button})"),
            PressedState::Key(key) => write!(formatter, "key({key})"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{InputEvent, Key, Modifier, PointerButton, PressedState};
    use crate::fault::InputFault;
    use mado_pilot_core::{CoordinateSpace, InputOperationKind, Point};

    fn point() -> Point {
        Point::new(CoordinateSpace::CapturePixels, 4.0, 8.0).expect("valid")
    }

    #[test]
    fn every_event_reports_the_operation_it_performs() {
        assert_eq!(
            InputEvent::PointerMove(point()).operation_kind(),
            Some(InputOperationKind::Pointer)
        );
        assert_eq!(
            InputEvent::PointerScroll {
                horizontal: 0,
                vertical: -3,
            }
            .operation_kind(),
            Some(InputOperationKind::Pointer)
        );
        assert_eq!(
            InputEvent::KeyPress(Key::Enter).operation_kind(),
            Some(InputOperationKind::Keyboard)
        );
        assert_eq!(
            InputEvent::Text("hello".to_owned()).operation_kind(),
            Some(InputOperationKind::Text)
        );
        assert_eq!(
            InputEvent::Delay(Duration::from_millis(1)).operation_kind(),
            None,
            "a delay is not something a target has to support"
        );
    }

    #[test]
    fn only_a_delay_is_reversible() {
        assert!(!InputEvent::Delay(Duration::ZERO).is_irreversible());
        for event in [
            InputEvent::PointerMove(point()),
            InputEvent::PointerPress(PointerButton::Primary),
            InputEvent::KeyRelease(Key::Tab),
            InputEvent::Text("x".to_owned()),
        ] {
            assert!(event.is_irreversible(), "{event:?}");
        }
    }

    #[test]
    fn presses_and_releases_are_reported_for_cleanup() {
        assert_eq!(
            InputEvent::PointerPress(PointerButton::Middle).presses(),
            Some(PressedState::Button(PointerButton::Middle))
        );
        assert_eq!(
            InputEvent::KeyPress(Key::Modifier(Modifier::Control)).presses(),
            Some(PressedState::Key(Key::Modifier(Modifier::Control)))
        );
        assert_eq!(
            InputEvent::KeyRelease(Key::Modifier(Modifier::Control)).releases(),
            Some(PressedState::Key(Key::Modifier(Modifier::Control)))
        );
        assert_eq!(
            InputEvent::PointerMove(point()).presses(),
            None,
            "a move leaves nothing held"
        );
        assert_eq!(InputEvent::Text("x".to_owned()).presses(), None);
    }

    #[test]
    fn an_out_of_range_function_key_names_no_key() {
        assert_eq!(Key::Function(1).check(), Ok(()));
        assert_eq!(Key::Function(24).check(), Ok(()));
        assert_eq!(
            Key::Function(0).check(),
            Err(InputFault::SequenceOutOfBounds)
        );
        assert_eq!(
            Key::Function(25).check(),
            Err(InputFault::SequenceOutOfBounds)
        );
    }

    #[test]
    fn a_control_character_is_not_a_key() {
        assert_eq!(Key::Character('a').check(), Ok(()));
        assert_eq!(Key::Character('あ').check(), Ok(()));
        assert_eq!(
            Key::Character('\n').check(),
            Err(InputFault::SequenceOutOfBounds),
            "the key that produces a newline is Enter, not the byte"
        );
        assert_eq!(
            Key::Character('\u{7}').check(),
            Err(InputFault::SequenceOutOfBounds)
        );
    }

    #[test]
    fn event_bounds_are_checked_before_delivery() {
        assert_eq!(InputEvent::Delay(InputEvent::MAX_DELAY).check(), Ok(()));
        assert_eq!(
            InputEvent::Delay(InputEvent::MAX_DELAY + Duration::from_millis(1)).check(),
            Err(InputFault::SequenceOutOfBounds)
        );
        assert_eq!(
            InputEvent::Text(String::new()).check(),
            Err(InputFault::SequenceOutOfBounds),
            "entering nothing is a mistake rather than a no-op"
        );
        assert_eq!(
            InputEvent::Text("x".repeat(InputEvent::MAX_TEXT_CHARS)).check(),
            Ok(())
        );
        assert_eq!(
            InputEvent::Text("x".repeat(InputEvent::MAX_TEXT_CHARS + 1)).check(),
            Err(InputFault::SequenceOutOfBounds)
        );
        assert_eq!(
            InputEvent::PointerScroll {
                horizontal: 0,
                vertical: 0,
            }
            .check(),
            Err(InputFault::SequenceOutOfBounds)
        );
        assert_eq!(
            InputEvent::PointerScroll {
                horizontal: InputEvent::MAX_SCROLL_NOTCHES,
                vertical: -InputEvent::MAX_SCROLL_NOTCHES,
            }
            .check(),
            Ok(())
        );
        assert_eq!(
            InputEvent::PointerScroll {
                horizontal: InputEvent::MAX_SCROLL_NOTCHES + 1,
                vertical: 0,
            }
            .check(),
            Err(InputFault::SequenceOutOfBounds)
        );
    }

    #[test]
    fn text_length_counts_characters_rather_than_bytes() {
        let multibyte = "あ".repeat(InputEvent::MAX_TEXT_CHARS);

        assert!(multibyte.len() > InputEvent::MAX_TEXT_CHARS);
        assert_eq!(
            InputEvent::Text(multibyte).check(),
            Ok(()),
            "a bound in bytes would refuse a Japanese string a Latin one fits"
        );
    }
}
