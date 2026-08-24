//! Synchronous native runs with joinable deadline/cancellation termination.

use std::mem::size_of;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use mado_pilot_core::OperationContext;
use ort::session::{RunOptions, Session};
use ort::value::TensorRef;

use crate::decode::{self, DecodedText};
use crate::detect::{self, Detection};
use crate::fault::OnnxBackendFault;
use crate::image::TensorInput;
use crate::vocabulary::Vocabulary;

const MONITOR_INTERVAL: Duration = Duration::from_millis(1);

pub(crate) fn detector(
    session: &mut Session,
    input: &TensorInput,
    source_width: u32,
    source_height: u32,
    max_candidates: usize,
    operation: &OperationContext,
) -> Result<Vec<Detection>, OnnxBackendFault> {
    checkpoint(operation)?;
    let tensor = TensorRef::from_array_view((input.shape, input.data.as_slice()))
        .map_err(|_| OnnxBackendFault::ResourceLimit)?;
    let options = Arc::new(RunOptions::new().map_err(|_| OnnxBackendFault::NativeFailure)?);
    let mut monitor = TerminationMonitor::start(Arc::clone(&options), operation.clone())?;
    #[cfg(test)]
    test_hook::at_run_admission();
    #[cfg(test)]
    test_hook::mark_run_started();
    let run = session.run_with_options(ort::inputs![tensor], options.as_ref());
    if let Some(interruption) = operation.interruption() {
        drop(run);
        monitor.finish()?;
        return Err(interruption.into());
    }
    let outputs = run.map_err(|_| OnnxBackendFault::NativeFailure)?;
    let output = outputs
        .get("sigmoid_0.tmp_0")
        .ok_or(OnnxBackendFault::MalformedOutput)?;
    let (shape, values) = output
        .try_extract_tensor::<f32>()
        .map_err(|_| OnnxBackendFault::MalformedOutput)?;
    let detections =
        detect::postprocess(shape, values, source_width, source_height, max_candidates)?;
    drop(outputs);
    monitor.finish()?;
    checkpoint(operation)?;
    Ok(detections)
}

pub(crate) fn recognizer(
    session: &mut Session,
    vocabulary: &Vocabulary,
    input: &TensorInput,
    max_text_bytes: usize,
    operation: &OperationContext,
) -> Result<Vec<DecodedText>, OnnxBackendFault> {
    checkpoint(operation)?;
    let batch = input.shape[0];
    let width = input.shape[3];
    let maximum_output_elements = batch
        .checked_mul(width.div_ceil(8))
        .and_then(|value| value.checked_mul(Vocabulary::classes()))
        .ok_or(OnnxBackendFault::ResourceLimit)?;
    if maximum_output_elements
        .checked_mul(size_of::<f32>())
        .is_none_or(|bytes| bytes > crate::MAX_OUTPUT_BYTES)
    {
        return Err(OnnxBackendFault::ResourceLimit);
    }

    let tensor = TensorRef::from_array_view((input.shape, input.data.as_slice()))
        .map_err(|_| OnnxBackendFault::ResourceLimit)?;
    let options = Arc::new(RunOptions::new().map_err(|_| OnnxBackendFault::NativeFailure)?);
    let mut monitor = TerminationMonitor::start(Arc::clone(&options), operation.clone())?;
    #[cfg(test)]
    test_hook::at_run_admission();
    #[cfg(test)]
    test_hook::mark_run_started();
    let run = session.run_with_options(ort::inputs![tensor], options.as_ref());
    if let Some(interruption) = operation.interruption() {
        drop(run);
        monitor.finish()?;
        return Err(interruption.into());
    }
    let outputs = run.map_err(|_| OnnxBackendFault::NativeFailure)?;
    let output = outputs
        .get("fetch_name_0")
        .ok_or(OnnxBackendFault::MalformedOutput)?;
    let (shape, values) = output
        .try_extract_tensor::<f32>()
        .map_err(|_| OnnxBackendFault::MalformedOutput)?;
    let decoded = decode::decode(shape, values, vocabulary, batch, max_text_bytes)?;
    drop(outputs);
    monitor.finish()?;
    checkpoint(operation)?;
    Ok(decoded)
}

#[derive(Debug)]
struct TerminationMonitor {
    done: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl TerminationMonitor {
    fn start(
        options: Arc<RunOptions>,
        operation: OperationContext,
    ) -> Result<Self, OnnxBackendFault> {
        let done = Arc::new(AtomicBool::new(false));
        if operation.deadline().is_none() && operation.cancellation().is_none() {
            return Ok(Self { done, thread: None });
        }
        let monitor_done = Arc::clone(&done);
        let thread = thread::Builder::new()
            .name("mado-onnx-terminate".to_owned())
            .spawn(move || {
                while !monitor_done.load(Ordering::Acquire) {
                    if operation.interruption().is_some() {
                        let _ = options.terminate();
                        #[cfg(test)]
                        test_hook::mark_termination_issued();
                        break;
                    }
                    let sleep = operation.remaining().map_or(MONITOR_INTERVAL, |remaining| {
                        remaining.min(MONITOR_INTERVAL)
                    });
                    if sleep.is_zero() {
                        thread::yield_now();
                    } else {
                        thread::sleep(sleep);
                    }
                }
            })
            .map_err(|_| OnnxBackendFault::ResourceLimit)?;
        Ok(Self {
            done,
            thread: Some(thread),
        })
    }

    fn finish(&mut self) -> Result<(), OnnxBackendFault> {
        self.done.store(true, Ordering::Release);
        if self
            .thread
            .take()
            .is_some_and(|monitor| monitor.join().is_err())
        {
            return Err(OnnxBackendFault::NativeFailure);
        }
        Ok(())
    }
}

impl Drop for TerminationMonitor {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Release);
        if let Some(monitor) = self.thread.take() {
            let _ = monitor.join();
        }
    }
}

fn checkpoint(operation: &OperationContext) -> Result<(), OnnxBackendFault> {
    operation
        .interruption()
        .map_or(Ok(()), |interruption| Err(interruption.into()))
}

#[cfg(test)]
pub(crate) mod test_hook {
    use std::ops::Deref;
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::Duration;

    #[derive(Default)]
    struct GateState {
        admitted: bool,
        released: bool,
        run_started: bool,
        termination_issued: bool,
    }

    pub(crate) struct RunGate {
        state: Mutex<GateState>,
        changed: Condvar,
    }

    pub(crate) struct RunGateGuard {
        gate: Arc<RunGate>,
    }

    static ACTIVE: Mutex<Option<Arc<RunGate>>> = Mutex::new(None);

    pub(crate) fn install() -> RunGateGuard {
        let gate = Arc::new(RunGate {
            state: Mutex::new(GateState::default()),
            changed: Condvar::new(),
        });
        *ACTIVE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Arc::clone(&gate));
        RunGateGuard { gate }
    }

    pub(crate) fn at_run_admission() {
        let gate = ACTIVE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if let Some(gate) = gate {
            let mut state = gate
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.admitted = true;
            gate.changed.notify_all();
            while !state.released {
                state = gate
                    .changed
                    .wait(state)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
        }
    }

    pub(crate) fn mark_run_started() {
        update_active(|state| state.run_started = true);
    }

    pub(crate) fn mark_termination_issued() {
        update_active(|state| state.termination_issued = true);
    }

    fn update_active(update: impl FnOnce(&mut GateState)) {
        let gate = ACTIVE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if let Some(gate) = gate {
            let mut state = gate
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            update(&mut state);
            gate.changed.notify_all();
        }
    }

    impl RunGate {
        pub(crate) fn wait_until_admitted(&self, timeout: Duration) -> bool {
            self.wait_until(timeout, |state| state.admitted)
        }

        pub(crate) fn wait_until_run_started(&self, timeout: Duration) -> bool {
            self.wait_until(timeout, |state| state.run_started)
        }

        pub(crate) fn wait_until_termination_issued(&self, timeout: Duration) -> bool {
            self.wait_until(timeout, |state| state.termination_issued)
        }

        fn wait_until(&self, timeout: Duration, observed: fn(&GateState) -> bool) -> bool {
            let state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let (state, _) = self
                .changed
                .wait_timeout_while(state, timeout, |state| !observed(state))
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            observed(&state)
        }

        pub(crate) fn release(&self) {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.released = true;
            self.changed.notify_all();
        }
    }

    impl Deref for RunGateGuard {
        type Target = RunGate;

        fn deref(&self) -> &Self::Target {
            &self.gate
        }
    }

    impl Drop for RunGateGuard {
        fn drop(&mut self) {
            self.gate.release();
            let mut active = ACTIVE
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if active
                .as_ref()
                .is_some_and(|gate| Arc::ptr_eq(gate, &self.gate))
            {
                *active = None;
            }
        }
    }
}
