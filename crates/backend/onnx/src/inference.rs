//! Synchronous native runs with joinable deadline/cancellation termination.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use mado_pilot_core::OperationContext;
use mado_pilot_ocr::{GraphDescriptor, TensorDescriptor};
use ort::session::{RunOptions, Session};
use ort::value::TensorRef;

use crate::decode::{self, DecodedText};
use crate::detect::{self, DbParameters, Detection};
use crate::fault::OnnxBackendFault;
use crate::image::{DetectorInput, TensorInput};
use crate::stage::{RunNanos, StageTimer};
use crate::vocabulary::Vocabulary;

const MONITOR_INTERVAL: Duration = Duration::from_millis(1);

pub(crate) fn detector(
    session: &mut Session,
    input: &DetectorInput,
    graph: GraphDescriptor,
    db: DbParameters,
    max_candidates: usize,
    operation: &OperationContext,
) -> Result<(Vec<Detection>, RunNanos), OnnxBackendFault> {
    checkpoint(operation)?;
    if !input_within_limits(&input.tensor.shape, graph.input) {
        return Err(OnnxBackendFault::ResourceLimit);
    }
    let tensor = TensorRef::from_array_view((input.tensor.shape, input.tensor.data.as_slice()))
        .map_err(|_| OnnxBackendFault::ResourceLimit)?;
    let options = Arc::new(RunOptions::new().map_err(|_| OnnxBackendFault::NativeFailure)?);
    let mut monitor = TerminationMonitor::start(Arc::clone(&options), operation.clone())?;
    #[cfg(any(test, feature = "benchmark-instrumentation"))]
    test_hook::at_run_admission();
    #[cfg(any(test, feature = "benchmark-instrumentation"))]
    test_hook::mark_run_started();
    let run_timer = StageTimer::start();
    let run = session.run_with_options(ort::inputs![tensor], options.as_ref());
    let inference = run_timer.elapsed_nanos();
    if let Some(interruption) = operation.interruption() {
        drop(run);
        monitor.finish()?;
        return Err(interruption.into());
    }
    let outputs = run.map_err(|_| OnnxBackendFault::NativeFailure)?;
    let postprocess_timer = StageTimer::start();
    let output = outputs
        .get(graph.output.name)
        .ok_or(OnnxBackendFault::MalformedOutput)?;
    let (shape, values) = output
        .try_extract_tensor::<f32>()
        .map_err(|_| OnnxBackendFault::MalformedOutput)?;
    if !output_within_limits(shape, graph.output) {
        return Err(OnnxBackendFault::MalformedOutput);
    }
    let detections = detect::postprocess(shape, values, input.plan, max_candidates, db)?;
    drop(outputs);
    let postprocess = postprocess_timer.elapsed_nanos();
    monitor.finish()?;
    checkpoint(operation)?;
    Ok((
        detections,
        RunNanos {
            inference,
            postprocess,
        },
    ))
}

pub(crate) fn recognizer(
    session: &mut Session,
    vocabulary: &Vocabulary,
    input: &TensorInput,
    graph: GraphDescriptor,
    max_text_bytes: usize,
    max_output_bytes: usize,
    operation: &OperationContext,
) -> Result<(Vec<DecodedText>, RunNanos), OnnxBackendFault> {
    checkpoint(operation)?;
    if !input_within_limits(&input.shape, graph.input) {
        return Err(OnnxBackendFault::ResourceLimit);
    }
    preflight_recognizer_output(input, max_output_bytes)?;
    let batch = input.shape[0];

    let tensor = TensorRef::from_array_view((input.shape, input.data.as_slice()))
        .map_err(|_| OnnxBackendFault::ResourceLimit)?;
    let options = Arc::new(RunOptions::new().map_err(|_| OnnxBackendFault::NativeFailure)?);
    let mut monitor = TerminationMonitor::start(Arc::clone(&options), operation.clone())?;
    #[cfg(any(test, feature = "benchmark-instrumentation"))]
    test_hook::at_run_admission();
    #[cfg(any(test, feature = "benchmark-instrumentation"))]
    test_hook::mark_run_started();
    let run_timer = StageTimer::start();
    let run = session.run_with_options(ort::inputs![tensor], options.as_ref());
    let inference = run_timer.elapsed_nanos();
    if let Some(interruption) = operation.interruption() {
        drop(run);
        monitor.finish()?;
        return Err(interruption.into());
    }
    let outputs = run.map_err(|_| OnnxBackendFault::NativeFailure)?;
    let decode_timer = StageTimer::start();
    let output = outputs
        .get(graph.output.name)
        .ok_or(OnnxBackendFault::MalformedOutput)?;
    let (shape, values) = output
        .try_extract_tensor::<f32>()
        .map_err(|_| OnnxBackendFault::MalformedOutput)?;
    if !output_within_limits(shape, graph.output) {
        return Err(OnnxBackendFault::MalformedOutput);
    }
    let decoded = decode::decode(
        shape,
        values,
        vocabulary,
        batch,
        max_text_bytes,
        max_output_bytes,
    )?;
    drop(outputs);
    let postprocess = decode_timer.elapsed_nanos();
    monitor.finish()?;
    checkpoint(operation)?;
    Ok((
        decoded,
        RunNanos {
            inference,
            postprocess,
        },
    ))
}

/// Checks an actual input tensor against the profile's finite per-axis ceilings.
fn input_within_limits(shape: &[usize; 4], tensor: TensorDescriptor) -> bool {
    shape.len() == tensor.axis_limits.len()
        && shape
            .iter()
            .zip(tensor.axis_limits)
            .all(|(dimension, limit)| u32::try_from(*dimension).is_ok_and(|value| value <= *limit))
}

/// Checks an extracted output shape against the profile's finite per-axis ceilings.
fn output_within_limits(shape: &[i64], tensor: TensorDescriptor) -> bool {
    shape.len() == tensor.axis_limits.len()
        && shape
            .iter()
            .zip(tensor.axis_limits)
            .all(|(dimension, limit)| u32::try_from(*dimension).is_ok_and(|value| value <= *limit))
}

fn preflight_recognizer_output(
    input: &TensorInput,
    max_output_bytes: usize,
) -> Result<(), OnnxBackendFault> {
    let output_bytes = decode::estimated_recognizer_output_bytes(input.shape[0], input.shape[3])?;
    if output_bytes > max_output_bytes {
        return Err(OnnxBackendFault::ResourceLimit);
    }
    Ok(())
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
                        #[cfg(any(test, feature = "benchmark-instrumentation"))]
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

#[cfg(any(test, feature = "benchmark-instrumentation"))]
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
        let mut active = ACTIVE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(
            active.is_none(),
            "only one native-run qualification gate may be active"
        );
        *active = Some(Arc::clone(&gate));
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

#[cfg(test)]
mod tests {
    use mado_pilot_ocr::{ACCEPTED_BOUNDED_PROFILE_ID, ACCEPTED_G004_PROFILE_ID, ocr_profile};

    use super::{input_within_limits, output_within_limits, preflight_recognizer_output};
    use crate::fault::OnnxBackendFault;
    use crate::image::TensorInput;
    use crate::provider::CUDA_RECOGNIZER_OUTPUT_BYTES;

    #[test]
    fn declared_axis_limits_bound_inputs_and_outputs_without_narrowing_released_shapes() {
        let bounded = ocr_profile(ACCEPTED_BOUNDED_PROFILE_ID).unwrap();
        let detector = bounded.detector_graph;
        assert!(input_within_limits(&[1, 3, 736, 1_312], detector.input));
        assert!(!input_within_limits(&[1, 3, 768, 1_312], detector.input));
        assert!(!input_within_limits(&[2, 3, 736, 736], detector.input));
        assert!(output_within_limits(&[1, 1, 736, 1_312], detector.output));
        assert!(!output_within_limits(&[1, 1, 736, 1_344], detector.output));
        assert!(!output_within_limits(&[1, 1, -1, 32], detector.output));
        assert!(!output_within_limits(&[1, 736, 1_312], detector.output));

        let native = ocr_profile(ACCEPTED_G004_PROFILE_ID).unwrap();
        assert!(input_within_limits(
            &[1, 3, 2_176, 3_840],
            native.detector_graph.input
        ));
        let recognizer = native.recognizer_graph;
        assert!(input_within_limits(&[6, 3, 48, 4_096], recognizer.input));
        assert!(!input_within_limits(&[7, 3, 48, 320], recognizer.input));
        assert!(!input_within_limits(&[1, 3, 48, 4_097], recognizer.input));
        assert!(output_within_limits(&[6, 512, 18_710], recognizer.output));
        assert!(!output_within_limits(&[6, 512, 18_711], recognizer.output));
    }

    fn input(batch: usize, width: usize) -> TensorInput {
        TensorInput {
            data: Vec::new(),
            shape: [batch, 3, 48, width],
        }
    }

    #[test]
    fn cuda_preflight_rejects_an_oversized_run_before_tensor_construction() {
        assert_eq!(
            preflight_recognizer_output(&input(4, 4_096), CUDA_RECOGNIZER_OUTPUT_BYTES),
            Err(OnnxBackendFault::ResourceLimit)
        );
        assert_eq!(
            preflight_recognizer_output(&input(3, 4_096), CUDA_RECOGNIZER_OUTPUT_BYTES),
            Ok(())
        );
    }

    #[test]
    fn cpu_preflight_retains_the_existing_256_mib_boundary() {
        assert_eq!(
            preflight_recognizer_output(&input(6, 4_096), crate::MAX_OUTPUT_BYTES),
            Ok(())
        );
    }
}
