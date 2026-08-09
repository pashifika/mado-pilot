//! C ABI ownership and observation contracts for the bounded diagnostic stream.

mod support;

use std::mem::size_of;
use std::ptr;
use std::thread;

use madopilot::*;
use support::{Scene, operation};

const ACTIVITY_TAG: u64 = 0x5a17;

fn table() -> &'static madopilot_api_t {
    support::negotiate(
        MADOPILOT_ABI_MAJOR,
        MADOPILOT_ABI_MINOR,
        size_of::<madopilot_api_t>(),
    )
    .expect("the current ABI negotiates")
}

fn diagnostic_record() -> madopilot_diagnostic_record_t {
    // SAFETY: the C record and every embedded record contain only integer
    // scalars, so the all-zero bit pattern is valid.
    let mut record = unsafe { std::mem::zeroed::<madopilot_diagnostic_record_t>() };
    record.struct_size = u32::try_from(size_of::<madopilot_diagnostic_record_t>())
        .expect("the record size fits u32");
    record
}

#[test]
fn diagnostics_off_allocates_no_reader() {
    let api = table();
    let flow = support::Flow::open();
    let mut reader = ptr::null_mut();

    assert_eq!(
        // SAFETY: the engine is live and `reader` is a writable output.
        unsafe { (api.engine_take_diagnostic_reader)(flow.engine, &raw mut reader) },
        MADOPILOT_STATUS_OK
    );
    assert!(reader.is_null());
}

#[test]
fn a_bounded_reader_and_immutable_batch_outlive_the_engine() {
    let api = table();
    let scene = Scene::new();
    let mut operation = operation();
    operation.flags |= MADOPILOT_OPERATION_HAS_ACTIVITY_TAG;
    operation.activity_tag = ACTIVITY_TAG;
    let options = madopilot_engine_options_t {
        struct_size: u32::try_from(size_of::<madopilot_engine_options_t>())
            .expect("the options size fits u32"),
        flags: 0,
        diagnostic_level: MADOPILOT_DIAGNOSTIC_LEVEL_DEBUG,
        diagnostic_capacity: 2,
    };
    let mut engine = ptr::null_mut();
    let mut error = ptr::null_mut();

    assert_eq!(
        // SAFETY: every input and output remains live for the call.
        unsafe {
            (api.engine_create_with_options)(
                scene.source(),
                &raw const options,
                &raw const operation,
                &raw mut engine,
                &raw mut error,
            )
        },
        MADOPILOT_STATUS_OK
    );
    assert!(!engine.is_null());
    assert!(error.is_null());

    let mut reader = ptr::null_mut();
    assert_eq!(
        // SAFETY: the engine is live and the output is writable.
        unsafe { (api.engine_take_diagnostic_reader)(engine, &raw mut reader) },
        MADOPILOT_STATUS_OK
    );
    assert!(!reader.is_null());

    let mut second = ptr::null_mut();
    assert_eq!(
        // SAFETY: as above.
        unsafe { (api.engine_take_diagnostic_reader)(engine, &raw mut second) },
        MADOPILOT_STATUS_OK
    );
    assert!(second.is_null(), "an engine exposes exactly one reader");

    for _ in 0..4 {
        let mut targets = ptr::null_mut();
        assert_eq!(
            // SAFETY: all arguments remain valid for the call.
            unsafe {
                (api.engine_discover)(
                    engine,
                    &raw const operation,
                    &raw mut targets,
                    &raw mut error,
                )
            },
            MADOPILOT_STATUS_OK
        );
        assert!(!targets.is_null());
        assert!(error.is_null());
        assert_eq!(
            // SAFETY: discovery returned this owned handle.
            unsafe { (api.target_list_release)(targets) },
            MADOPILOT_STATUS_OK
        );
    }

    // SAFETY: this drops the engine's final reference. The independent reader
    // retains its queue and boundary-identity state.
    assert_eq!(unsafe { (api.engine_release)(engine) }, MADOPILOT_STATUS_OK);

    let mut state = MADOPILOT_DIAGNOSTIC_DRAIN_OPEN_EMPTY;
    let mut batch = ptr::null_mut();
    assert_eq!(
        // SAFETY: the reader is live and both outputs are writable.
        unsafe { (api.diagnostic_reader_drain)(reader, &raw mut state, &raw mut batch) },
        MADOPILOT_STATUS_OK
    );
    assert_eq!(state, MADOPILOT_DIAGNOSTIC_DRAIN_BATCH);
    assert!(!batch.is_null());

    let mut info = madopilot_diagnostic_batch_info_t {
        struct_size: u32::try_from(size_of::<madopilot_diagnostic_batch_info_t>())
            .expect("the batch info size fits u32"),
        flags: u32::MAX,
        record_count: 0,
        discarded_normal: 0,
        discarded_debug: 0,
    };
    assert_eq!(
        // SAFETY: the batch is live and `info` is writable.
        unsafe { (api.diagnostic_batch_info)(batch, &raw mut info) },
        MADOPILOT_STATUS_OK
    );
    assert_eq!(info.flags, 0);
    assert_eq!(info.record_count, 2);
    assert_eq!(info.discarded_normal, 0);
    assert_eq!(info.discarded_debug, 2);

    let mut previous_sequence = 0;
    for index in 0..info.record_count {
        let mut record = diagnostic_record();
        assert_eq!(
            // SAFETY: the batch is live, the checked count bounds `index`, and
            // the output is writable.
            unsafe {
                (api.diagnostic_batch_record_at)(
                    batch,
                    usize::try_from(index).expect("the bounded index fits usize"),
                    &raw mut record,
                )
            },
            MADOPILOT_STATUS_OK
        );
        assert!(record.sequence > previous_sequence);
        previous_sequence = record.sequence;
        assert_eq!(record.level, MADOPILOT_DIAGNOSTIC_LEVEL_DEBUG);
        assert_eq!(record.kind, MADOPILOT_DIAGNOSTIC_KIND_OPERATION_STARTED);
        assert_eq!(record.operation, MADOPILOT_DIAGNOSTIC_OPERATION_DISCOVERY);
        assert_ne!(record.operation_id, 0);
        assert_ne!(record.flags & MADOPILOT_DIAGNOSTIC_RECORD_HAS_ACTIVITY, 0);
        assert_eq!(record.activity_tag, ACTIVITY_TAG);
    }

    let mut workers = Vec::new();
    for _ in 0..4 {
        assert_eq!(
            // SAFETY: each worker receives one retained reference.
            unsafe { (api.diagnostic_batch_retain)(batch) },
            MADOPILOT_STATUS_OK
        );
        let batch_address = batch.addr();
        workers.push(thread::spawn(move || {
            let batch =
                ptr::with_exposed_provenance_mut::<madopilot_diagnostic_batch_t>(batch_address);
            for _ in 0..64 {
                let mut record = diagnostic_record();
                assert_eq!(
                    // SAFETY: this worker owns a retained batch reference and
                    // index zero exists in the immutable two-record batch.
                    unsafe { (api.diagnostic_batch_record_at)(batch, 0, &raw mut record) },
                    MADOPILOT_STATUS_OK
                );
                assert_eq!(record.activity_tag, ACTIVITY_TAG);
            }
            assert_eq!(
                // SAFETY: balances the retain before this worker was spawned.
                unsafe { (api.diagnostic_batch_release)(batch) },
                MADOPILOT_STATUS_OK
            );
        }));
    }
    for worker in workers {
        worker.join().expect("the immutable batch reader completed");
    }

    assert_eq!(
        // SAFETY: releases the original batch reference.
        unsafe { (api.diagnostic_batch_release)(batch) },
        MADOPILOT_STATUS_OK
    );

    state = MADOPILOT_DIAGNOSTIC_DRAIN_BATCH;
    batch = ptr::without_provenance_mut(usize::MAX);
    assert_eq!(
        // SAFETY: the reader remains live and outputs are writable.
        unsafe { (api.diagnostic_reader_drain)(reader, &raw mut state, &raw mut batch) },
        MADOPILOT_STATUS_OK
    );
    assert_eq!(state, MADOPILOT_DIAGNOSTIC_DRAIN_END_OF_STREAM);
    assert!(batch.is_null());

    assert_eq!(
        // SAFETY: drops the reader's final reference after the sealed stream ended.
        unsafe { (api.diagnostic_reader_release)(reader) },
        MADOPILOT_STATUS_OK
    );
}
