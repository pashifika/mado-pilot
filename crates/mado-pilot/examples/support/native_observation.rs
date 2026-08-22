//! Shared visual-oracle and diagnostic-drain steps for the native Rust examples.

use std::time::Duration;

use mado_pilot::{
    ActivityTag, CpuMapping, DiagnosticDrain, DiagnosticLevel, DiagnosticReader, Error, FrameOrder,
    FrameRequest, FrameStamp, OperationContext, PixelFormat, Session,
};

const ACTIVITY_TAG_VALUE: u64 = 0x5a17;
const EXPECTED_FILL_BGRA: [u8; 3] = [0x2e, 0x5b, 0xc4];
#[cfg(target_os = "macos")]
const FILL_TOLERANCE: u8 = 24;
#[cfg(windows)]
const FILL_TOLERANCE: u8 = 8;
const FRAME_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Returns the nonzero tag shared by every operation in one example run.
pub(crate) fn activity_tag() -> ActivityTag {
    ActivityTag::new(ACTIVITY_TAG_VALUE).expect("the example activity tag is nonzero")
}

/// Returns one tagged operation bounded by `budget`.
pub(crate) fn bounded(budget: Duration) -> Result<OperationContext, Error> {
    OperationContext::new()
        .with_activity_tag(activity_tag())
        .with_timeout(budget)
}

/// Reports whether the central half of a canonical BGRA8 mapping has the
/// fixture's deterministic post-input fill.
pub(crate) fn expected_condition_matches(mapping: &CpuMapping) -> bool {
    let descriptor = mapping.descriptor();
    if descriptor.format() != PixelFormat::Bgra8 {
        return false;
    }
    let extent = descriptor.extent();
    let Ok(width) = usize::try_from(extent.width()) else {
        return false;
    };
    let Ok(height) = usize::try_from(extent.height()) else {
        return false;
    };
    if width < 8 || height < 8 {
        return false;
    }
    let stride = descriptor.stride();
    if stride < width.saturating_mul(4)
        || stride
            .checked_mul(height)
            .is_none_or(|required| mapping.bytes().len() < required)
    {
        return false;
    }

    for row in height / 4..(height * 3) / 4 {
        for column in width / 4..(width * 3) / 4 {
            let at = row * stride + column * 4;
            for (channel, expected) in EXPECTED_FILL_BGRA.iter().copied().enumerate() {
                if mapping.bytes()[at + channel].abs_diff(expected) > FILL_TOLERANCE {
                    return false;
                }
            }
        }
    }
    true
}

/// Waits for a frame strictly newer than `before` whose central pixels satisfy
/// the fixture's expected post-input condition.
pub(crate) fn observe_expected_condition(
    session: &Session,
    before: FrameStamp,
    budget: Duration,
) -> Result<(), Box<dyn std::error::Error>> {
    let operation = bounded(budget)?;
    loop {
        let frame = session.acquire_frame(&FrameRequest::newer_than(before), &operation)?;
        if before.order(&frame.stamp()) != Ok(FrameOrder::Before) {
            return Err(
                "a newer-frame request returned a frame that was not strictly newer".into(),
            );
        }
        let mapping = session.map_frame(&frame, PixelFormat::Bgra8, &operation)?;
        if expected_condition_matches(&mapping) {
            println!(
                "expected condition: frame sequence {} is strictly newer than {}",
                frame.stamp().sequence().value(),
                before.sequence().value()
            );
            return Ok(());
        }
        std::thread::sleep(FRAME_POLL_INTERVAL);
    }
}

/// Drains the sealed engine stream, verifies one activity correlation, and
/// reports only privacy-reviewed identity, level, kind, count, and loss fields.
pub(crate) fn drain_diagnostics(
    reader: &DiagnosticReader,
) -> Result<(), Box<dyn std::error::Error>> {
    let expected_activity = activity_tag();
    let mut normal = 0_u64;
    let mut debug = 0_u64;
    let mut discarded_normal = 0_u64;
    let mut discarded_debug = 0_u64;

    loop {
        match reader.drain() {
            DiagnosticDrain::Batch(batch) => {
                discarded_normal = discarded_normal.saturating_add(batch.losses().normal());
                discarded_debug = discarded_debug.saturating_add(batch.losses().debug());
                for record in batch.records() {
                    if record.activity() != Some(expected_activity) {
                        return Err("a diagnostic record lost the example activity tag".into());
                    }
                    match record.level() {
                        DiagnosticLevel::Normal => normal = normal.saturating_add(1),
                        DiagnosticLevel::Debug => debug = debug.saturating_add(1),
                        DiagnosticLevel::Off => {
                            return Err(
                                "an enabled diagnostic stream retained an Off record".into()
                            );
                        }
                        _ => return Err("the diagnostic stream returned an unknown level".into()),
                    }
                    println!(
                        "diagnostic: sequence {} operation {} activity {} level {:?} kind {:?}",
                        record.sequence().get(),
                        record.operation().get(),
                        expected_activity.get(),
                        record.level(),
                        record.kind()
                    );
                }
            }
            DiagnosticDrain::EndOfStream => break,
            DiagnosticDrain::OpenEmpty => {
                return Err("the diagnostic stream remained open after engine release".into());
            }
            _ => return Err("the diagnostic reader returned an unknown drain state".into()),
        }
    }

    if normal == 0 || debug == 0 {
        return Err("the debug stream did not retain both normal and debug records".into());
    }
    println!(
        "diagnostics: normal {normal} debug {debug} discarded-normal {discarded_normal} \
         discarded-debug {discarded_debug}"
    );
    Ok(())
}
