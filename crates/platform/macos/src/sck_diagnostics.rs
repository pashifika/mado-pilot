pub fn set_tier(tier: u32) -> Result<(), u32> {
    // SAFETY: `tier` is a value argument; the native boundary validates its range.
    let status = unsafe { mp_shim_sck_diagnostics_set_tier(tier) };
    if status == 0 { Ok(()) } else { Err(status) }
}

pub fn dump() -> Result<(), u32> {
    // SAFETY: the native function takes no pointers and writes only to stderr.
    let status = unsafe { mp_shim_sck_diagnostics_dump() };
    if status == 0 { Ok(()) } else { Err(status) }
}

unsafe extern "C" {
    fn mp_shim_sck_diagnostics_set_tier(tier: u32) -> u32;
    fn mp_shim_sck_diagnostics_dump() -> u32;
}
