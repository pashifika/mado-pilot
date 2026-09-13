//! Shared counted UTF-16 path validation without live image observation.

use std::path::Path;

/// Converts UTF-16 content (without a terminator) to an absolute, line-safe path.
pub(crate) fn validated_path(units: &[u16]) -> Result<String, &'static str> {
    validate_line_protocol(units)?;
    let path = String::from_utf16(units).map_err(|_| "module-path-encoding")?;
    if !Path::new(&path).is_absolute() {
        return Err("module-path-absolute");
    }
    Ok(path)
}

pub(crate) fn validate_line_protocol(units: &[u16]) -> Result<(), &'static str> {
    if units.iter().any(|unit| matches!(*unit, 0 | 10 | 13)) {
        return Err("module-path-line-protocol");
    }
    Ok(())
}
