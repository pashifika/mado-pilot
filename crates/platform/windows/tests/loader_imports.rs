#![cfg(windows)]
//! Verifies that version-sensitive Windows APIs remain runtime-resolved.

use std::{fs, sync::Arc};

use mado_pilot_capture::CaptureProvider;
use mado_pilot_core::{IdentityIssuer, OperationContext};
use mado_pilot_platform_windows::WindowsCaptureProvider;

#[test]
fn optional_windows_exports_are_absent_from_the_pe_import_table() {
    // Keep the production availability and discovery path reachable from this
    // executable. Otherwise the linker may omit the Adapter and make an empty
    // import table look like evidence for lazy loading.
    let provider = WindowsCaptureProvider::new(Arc::new(IdentityIssuer::new()));
    let _availability = provider.discover(&OperationContext::new());

    let executable = std::env::current_exe().expect("current test executable");
    let bytes = fs::read(executable).expect("read test executable");
    let imports = pe_imports(&bytes);

    for forbidden in [
        "CoIncrementMTAUsage",
        "CreateDirect3D11DeviceFromDXGIDevice",
        "GetDpiForMonitor",
        "GetDpiForWindow",
        "GetScaleFactorForMonitor",
        "LogicalToPhysicalPointForPerMonitorDPI",
        "RoGetActivationFactory",
        "RoInitialize",
        "RoUninitialize",
    ] {
        assert!(
            !imports.iter().any(|import| import == forbidden),
            "{forbidden} must be resolved after runtime availability checks"
        );
    }
}

fn pe_imports(bytes: &[u8]) -> Vec<String> {
    let pe = usize::try_from(read_u32(bytes, 0x3c)).expect("PE offset fits");
    assert_eq!(bytes.get(pe..pe + 4), Some(&b"PE\0\0"[..]), "PE signature");
    let coff = pe + 4;
    let section_count = usize::from(read_u16(bytes, coff + 2));
    let optional_size = usize::from(read_u16(bytes, coff + 16));
    let optional = coff + 20;
    let pe32_plus = match read_u16(bytes, optional) {
        0x20b => true,
        0x10b => false,
        magic => panic!("unsupported PE optional-header magic {magic:#x}"),
    };
    let directory = optional + if pe32_plus { 112 } else { 96 };
    let import_rva = read_u32(bytes, directory + 8);
    let sections = optional + optional_size;
    let rva_to_offset = |rva| {
        rva_offset(bytes, sections, section_count, rva)
            .unwrap_or_else(|| panic!("RVA {rva:#x} is not mapped by a PE section"))
    };

    let mut imports = Vec::new();
    let mut descriptor = rva_to_offset(import_rva);
    loop {
        let original_thunk = read_u32(bytes, descriptor);
        let name = read_u32(bytes, descriptor + 12);
        let first_thunk = read_u32(bytes, descriptor + 16);
        if original_thunk == 0 && name == 0 && first_thunk == 0 {
            break;
        }
        let mut thunk = rva_to_offset(if original_thunk == 0 {
            first_thunk
        } else {
            original_thunk
        });
        loop {
            let value = if pe32_plus {
                read_u64(bytes, thunk)
            } else {
                u64::from(read_u32(bytes, thunk))
            };
            if value == 0 {
                break;
            }
            let ordinal_mask = if pe32_plus { 1u64 << 63 } else { 1u64 << 31 };
            if value & ordinal_mask == 0 {
                let name_rva = u32::try_from(value).expect("import name RVA fits");
                imports.push(read_c_string(bytes, rva_to_offset(name_rva) + 2));
            }
            thunk += if pe32_plus { 8 } else { 4 };
        }
        descriptor += 20;
    }
    imports
}

fn rva_offset(bytes: &[u8], sections: usize, section_count: usize, rva: u32) -> Option<usize> {
    for index in 0..section_count {
        let section = sections + index * 40;
        let virtual_size = read_u32(bytes, section + 8);
        let virtual_address = read_u32(bytes, section + 12);
        let raw_size = read_u32(bytes, section + 16);
        let raw_offset = read_u32(bytes, section + 20);
        let span = virtual_size.max(raw_size);
        if rva >= virtual_address && rva < virtual_address.saturating_add(span) {
            return usize::try_from(raw_offset.saturating_add(rva.saturating_sub(virtual_address)))
                .ok();
        }
    }
    None
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(
        bytes
            .get(offset..offset + 2)
            .expect("u16 lies within PE")
            .try_into()
            .expect("two bytes"),
    )
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes
            .get(offset..offset + 4)
            .expect("u32 lies within PE")
            .try_into()
            .expect("four bytes"),
    )
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        bytes
            .get(offset..offset + 8)
            .expect("u64 lies within PE")
            .try_into()
            .expect("eight bytes"),
    )
}

fn read_c_string(bytes: &[u8], offset: usize) -> String {
    let tail = bytes.get(offset..).expect("string begins within PE");
    let length = tail
        .iter()
        .position(|byte| *byte == 0)
        .expect("import string is NUL-terminated");
    String::from_utf8(tail[..length].to_vec()).expect("import symbol is ASCII")
}
