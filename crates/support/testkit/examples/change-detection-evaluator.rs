//! Emits the deterministic target-neutral G-005 aggregate report.

use std::io::{self, Write as _};
use std::path::Path;

use mado_pilot_testkit::change_detection::{RecordedSequenceSet, evaluate_g005};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let repository_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let sequences = RecordedSequenceSet::load(&repository_root)?;
    let report = evaluate_g005(&sequences);
    io::stdout().write_all(&report.to_canonical_json()?)?;
    Ok(())
}
