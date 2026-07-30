//! The shared backend-independent suite, run against OpenCV unchanged.
//!
//! This file is short on purpose. Every check lives in
//! `mado-pilot-testkit`, where the controlled double already runs it, and
//! running the same code here is what makes the vision seam a real contract
//! rather than a description of one implementation. A check that had to be
//! weakened, skipped, or reworded for OpenCV would mean the contract was written
//! around the double.

use std::sync::Arc;

use mado_pilot_backend_opencv::OpenCvBackend;
use mado_pilot_testkit::vision_contract;
use mado_pilot_vision::MatchBackend;

fn backend() -> Arc<dyn MatchBackend> {
    Arc::new(OpenCvBackend::new().expect("the development OpenCV installation is usable"))
}

#[test]
fn the_opencv_backend_satisfies_the_vision_contract() {
    vision_contract::run(&backend());
}
