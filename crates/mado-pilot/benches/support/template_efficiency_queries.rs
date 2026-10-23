//! Fresh controlled query sources with persistent externally owned preparation.
//!
//! The primary interval includes publication, public wait, scalar projection and
//! producer join. Close, retained-terminal checks and final release are separate.

use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use mado_pilot::{
    BackendDescriptor, ChangeDetectionPolicy, FrameStamp, OperationContext, PixelRect,
    PreparedTemplate, TemplateAnalysisRate, TemplateQuery, TemplateQueryOutcome,
    TemplateQueryProgress, TemplateStability, TemplateTerminalOutcome, TemplateWatchRequest,
    TemplateWorkDisposition,
};
use serde_json::{Value, json};

use super::fixtures::Workload;
use super::session_fixtures::{self, Case};
use super::sessions::{
    FrameFacts, Projection, Runtime, Timing, Unit, next_stamp, selection, stamp_json,
};
use super::{ClockStart, operation};

const GUARD: Duration = Duration::from_secs(5);

#[derive(Clone, Copy)]
struct Receipt {
    stamp: FrameStamp,
    admitted: u64,
    completed: u64,
    confirmed: u32,
    pending: u32,
    in_flight: u32,
    exceptional: u64,
}

impl Receipt {
    fn from_progress(progress: TemplateQueryProgress, stamp: FrameStamp) -> Self {
        use TemplateWorkDisposition as Work;
        let exceptional = [
            Work::Failed,
            Work::Rejected,
            Work::QueueExpired,
            Work::Superseded,
            Work::Coalesced,
            Work::SkippedChange,
            Work::DeferredRate,
        ]
        .into_iter()
        .map(|kind| progress.work().get(kind))
        .sum();
        Self {
            stamp,
            admitted: progress.work().get(Work::Admitted),
            completed: progress.work().get(Work::Completed),
            confirmed: progress.confirmed_observations(),
            pending: progress.pending_count(),
            in_flight: progress.in_flight_count(),
            exceptional,
        }
    }

    fn json(self) -> Value {
        json!({"stamp": stamp_json(self.stamp), "admitted": self.admitted,
            "completed": self.completed, "confirmed": self.confirmed,
            "pending": self.pending, "in_flight": self.in_flight, "exceptional": self.exceptional})
    }
}

pub(super) struct QueryRecord {
    terminal: &'static str,
    completed: [Option<Receipt>; 2],
    terminal_stamp: Option<FrameStamp>,
    confirmed: u32,
    after_close_same: bool,
}

impl QueryRecord {
    pub(super) fn json(&self) -> Value {
        json!({"terminal": self.terminal,
            "completed": self.completed.iter().flatten().copied().map(Receipt::json).collect::<Vec<_>>(),
            "terminal_stamp": self.terminal_stamp.map(stamp_json),
            "confirmed": self.confirmed, "after_close_same": self.after_close_same})
    }
}

struct Producer {
    completed: [Option<Receipt>; 2],
    frames: [Option<FrameFacts>; 3],
    error: Option<String>,
}

impl Producer {
    fn empty() -> Self {
        Self {
            completed: [None; 2],
            frames: [None; 3],
            error: None,
        }
    }
}

fn completed(
    query: &TemplateQuery,
    expected: FrameStamp,
    count: u64,
    confirmed: u32,
) -> Result<Receipt, String> {
    let deadline = Instant::now() + GUARD;
    loop {
        match query.poll() {
            TemplateQueryOutcome::Pending(progress) => {
                if progress.id() != query.id() {
                    return Err("progress belongs to another query".into());
                }
                let receipt = Receipt::from_progress(progress, expected);
                if receipt.exceptional != 0 {
                    return Err("query recorded an exceptional work disposition".into());
                }
                if progress.last_frame() == Some(expected)
                    && receipt.admitted == count
                    && receipt.completed == count
                    && receipt.confirmed == confirmed
                    && receipt.pending == 0
                    && receipt.in_flight == 0
                {
                    return Ok(receipt);
                }
                if Instant::now() >= deadline {
                    return Err("completed-work progress guard expired".into());
                }
                std::thread::yield_now();
            }
            TemplateQueryOutcome::Terminal(outcome) => {
                return Err(format!(
                    "query terminated before its completed-work receipt: {outcome:?}"
                ));
            }
        }
    }
}

fn produce(
    runtime: &Runtime,
    query: &TemplateQuery,
    pixels: &[Vec<u8>; 3],
    deadline: bool,
) -> Producer {
    let mut report = Producer::empty();
    let result = (|| -> Result<(), String> {
        for (index, pixels) in pixels.iter().take(if deadline { 1 } else { 3 }).enumerate() {
            let frame = runtime.publish(pixels)?;
            let facts = FrameFacts::of(&frame);
            drop(frame);
            report.frames[index] = Some(facts);
            if index != 0
                && !next_stamp(
                    report.frames[index - 1]
                        .expect("previous publication")
                        .stamp,
                    facts.stamp,
                )
            {
                return Err(
                    "query publication changed stream, epoch or geometry, or skipped a sequence"
                        .into(),
                );
            }
            if index < 2 {
                let confirmed = u32::try_from(index).expect("two progress barriers");
                report.completed[index] = Some(completed(
                    query,
                    facts.stamp,
                    u64::from(confirmed) + 1,
                    confirmed,
                )?);
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        report.error = Some(error);
        // Wake the actual waiter on producer failure; never substitute cancellation for success.
        drop(query.cancel());
    }
    report
}

pub(super) fn run(
    runtime: Runtime,
    row: Workload,
    template: &PreparedTemplate,
    pixels: &[Vec<u8>; 3],
    region: PixelRect,
    descriptor: &BackendDescriptor,
    prepared: Timing,
) -> Unit {
    let mut unit = Unit::new(prepared);
    unit.query_setup = Some([true, false, false]);
    unit.query_teardown = Some([false; 4]);
    let setup = ClockStart::now();
    let is_deadline = matches!(session_fixtures::case(row), Case::QueryDeadline);
    let lifetime = if is_deadline {
        Duration::from_secs(1)
    } else {
        GUARD
    };
    // Construct query authority once; ready/start setup consumes this same lifetime.
    let authority = OperationContext::new()
        .with_timeout(lifetime)
        .expect("bounded query lifetime");
    let request =
        TemplateWatchRequest::new(template.clone(), session_fixtures::options(), authority)
            .with_region(selection(region))
            .with_change_policy(ChangeDetectionPolicy::AnalysisAlways)
            .with_rate(TemplateAnalysisRate::unrestricted())
            .with_stability(TemplateStability::consecutive(2).expect("two confirmations"));
    let query = match runtime.session().start_template_watch(request) {
        Ok(query) => query,
        Err(error) => {
            unit.setup.add(Timing::finish(setup));
            unit.fail(error.to_string());
            let teardown = ClockStart::now();
            let closed = runtime.close().is_ok();
            let released = runtime.release();
            let clean = released.is_ok();
            if let Err(error) = released {
                unit.fail(error);
            }
            unit.query_teardown = Some([closed, false, true, clean]);
            unit.teardown = Timing::finish(teardown);
            return unit;
        }
    };
    unit.query_setup = Some([true, true, false]);
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let (start_tx, start_rx) = mpsc::sync_channel(1);
    let (outcome, producer) = std::thread::scope(|scope| {
        let producer_runtime = &runtime;
        let producer_query = &query;
        let worker = std::thread::Builder::new()
            .name("session-corpus-producer".into())
            .spawn_scoped(scope, move || {
                if ready_tx.send(()).is_err() {
                    return Producer {
                        error: Some("producer ready receiver closed".into()),
                        ..Producer::empty()
                    };
                }
                if start_rx.recv_timeout(GUARD).is_err() {
                    drop(producer_query.cancel());
                    return Producer {
                        error: Some("producer start guard expired or disconnected".into()),
                        ..Producer::empty()
                    };
                }
                produce(producer_runtime, producer_query, pixels, is_deadline)
            });
        let worker = match worker {
            Ok(worker) => worker,
            Err(error) => {
                unit.setup.add(Timing::finish(setup));
                unit.fail(format!("producer creation failed: {error}"));
                drop(query.cancel());
                return (None, Producer::empty());
            }
        };
        if let Err(error) = ready_rx.recv_timeout(GUARD) {
            drop(start_tx);
            drop(query.cancel());
            let producer = worker.join().unwrap_or_else(|_| Producer {
                error: Some("producer panicked before release".into()),
                ..Producer::empty()
            });
            unit.setup.add(Timing::finish(setup));
            unit.fail(format!("producer ready guard failed: {error}"));
            return (None, producer);
        }
        unit.query_setup = Some([true, true, true]);
        unit.setup.add(Timing::finish(setup));
        let wait = operation();
        let started = ClockStart::now();
        if start_tx.send(()).is_err() {
            unit.fail("producer start receiver closed");
            drop(query.cancel());
        }
        drop(start_tx);
        let outcome = match query.wait(&wait) {
            Ok(outcome) => Some(outcome),
            Err(error) => {
                unit.fail(format!("caller wait interrupted: {error}"));
                drop(query.cancel());
                None
            }
        };
        let producer = worker.join().unwrap_or_else(|_| Producer {
            error: Some("producer panicked after release".into()),
            ..Producer::empty()
        });
        if let (Some(outcome), Some(expected)) = (outcome.as_ref(), producer.frames[2])
            && let TemplateTerminalOutcome::Matched(found) = outcome.as_ref()
        {
            match Projection::capture(
                found.result(),
                found.frame(),
                expected,
                template,
                region,
                descriptor,
                found.target() == runtime.session().target() && found.template() == template.id(),
            ) {
                Ok(projection) => unit.projections[0] = Some(projection),
                Err(error) => unit.fail(error),
            }
        }
        unit.primary = Timing::finish(started);
        (outcome, producer)
    });

    if let Some(error) = producer.error {
        unit.fail(error);
    }
    let mut record = QueryRecord {
        terminal: "wait-failed",
        completed: producer.completed,
        terminal_stamp: None,
        confirmed: 0,
        after_close_same: false,
    };
    if let Some(outcome) = &outcome {
        match outcome.as_ref() {
            TemplateTerminalOutcome::Matched(found) => {
                record.terminal = "matched";
                record.terminal_stamp = Some(found.frame().stamp());
                record.confirmed = found.confirmed_observations();
                if is_deadline
                    || record.confirmed != 2
                    || producer.frames[2].is_none_or(|facts| facts != FrameFacts::of(found.frame()))
                {
                    unit.fail("matched query terminal identity or confirmation count differs");
                }
            }
            TemplateTerminalOutcome::DeadlineExceeded => {
                record.terminal = "deadline-exceeded";
                if !is_deadline {
                    unit.fail("matched query reached its lifetime deadline");
                }
            }
            terminal => {
                record.terminal = "unexpected-terminal";
                unit.fail(format!("unexpected query terminal: {terminal:?}"));
            }
        }
    }
    let expected_receipts = if is_deadline { 1 } else { 2 };
    if record.completed.iter().flatten().count() != expected_receipts {
        unit.fail("required completed-work receipts are missing");
    }

    let teardown = ClockStart::now();
    let closed = match runtime.close() {
        Ok(()) => true,
        Err(error) => {
            unit.fail(error);
            false
        }
    };
    let after_close = query.poll();
    if let (Some(before), TemplateQueryOutcome::Terminal(after)) = (&outcome, &after_close) {
        record.after_close_same = Arc::ptr_eq(before, after);
        if let TemplateTerminalOutcome::Matched(found) = after.as_ref() {
            let retained = producer.frames[2].and_then(|expected| {
                Projection::capture(
                    found.result(),
                    found.frame(),
                    expected,
                    template,
                    region,
                    descriptor,
                    found.target() == runtime.session().target()
                        && found.template() == template.id(),
                )
                .ok()
            });
            record.after_close_same &= retained.is_some()
                && retained == unit.projections[0]
                && found.confirmed_observations() == 2;
        }
    }
    if !record.after_close_same {
        unit.fail("closed Session changed or lost the retained query terminal");
    }
    drop(after_close);
    drop(outcome);
    drop(query);
    let released = match runtime.release() {
        Ok(()) => true,
        Err(error) => {
            unit.fail(error);
            false
        }
    };
    unit.query_teardown = Some([closed, record.after_close_same, true, released]);
    unit.teardown = Timing::finish(teardown);
    unit.query = Some(record);
    unit
}
