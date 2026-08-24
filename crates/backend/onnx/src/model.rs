//! Fixed-path loading for the accepted G-004 model pair.

use std::fs::File;
use std::io::Read;
use std::mem::MaybeUninit;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mado_pilot_core::OperationContext;
use mado_pilot_ocr::{
    ModelComponentIdentity, OcrModelComponent, OcrModelIdentity, OcrModelSource,
    OcrModelSourceRequest,
};

use crate::{OnnxBackendFault, checkpoint};

pub(crate) const DETECTOR_RELATIVE_PATH: &str = "rapidocr-v3.9.2/ch_PP-OCRv4_det_mobile.onnx";
pub(crate) const RECOGNIZER_RELATIVE_PATH: &str = "rapidocr-v3.9.2/PP-OCRv6_rec_small.onnx";

pub(crate) fn accepted_source(
    model_root: &Path,
    operation: &OperationContext,
) -> Result<OcrModelSource, OnnxBackendFault> {
    checkpoint(operation)?;
    let root = canonical_root(model_root)?;
    let identity = OcrModelIdentity::accepted_g004();
    let detector = read_component(
        &root,
        DETECTOR_RELATIVE_PATH,
        identity.detector(),
        OnnxBackendFault::DetectorUnavailable,
        OnnxBackendFault::DetectorMismatch,
        operation,
    )?;
    let recognizer = read_component(
        &root,
        RECOGNIZER_RELATIVE_PATH,
        identity.recognizer(),
        OnnxBackendFault::RecognizerUnavailable,
        OnnxBackendFault::RecognizerMismatch,
        operation,
    )?;
    checkpoint(operation)?;

    OcrModelSource::new(OcrModelSourceRequest {
        identity,
        detector,
        recognizer,
    })
    .map_err(|_| OnnxBackendFault::ProfileMismatch)
}

fn canonical_root(model_root: &Path) -> Result<PathBuf, OnnxBackendFault> {
    if !model_root.is_absolute() {
        return Err(OnnxBackendFault::InvalidModelRoot);
    }
    let root = model_root
        .canonicalize()
        .map_err(|_| OnnxBackendFault::ModelRootUnavailable)?;
    if !root
        .metadata()
        .map_err(|_| OnnxBackendFault::ModelRootUnavailable)?
        .is_dir()
    {
        return Err(OnnxBackendFault::InvalidModelRoot);
    }
    Ok(root)
}

fn read_component(
    root: &Path,
    relative: &str,
    identity: ModelComponentIdentity,
    unavailable: OnnxBackendFault,
    mismatch: OnnxBackendFault,
    operation: &OperationContext,
) -> Result<OcrModelComponent, OnnxBackendFault> {
    checkpoint(operation)?;
    let selected = root
        .join(relative)
        .canonicalize()
        .map_err(|_| unavailable)?;
    if !selected.starts_with(root) {
        return Err(mismatch);
    }

    let metadata = selected.metadata().map_err(|_| unavailable)?;
    if !metadata.is_file() || metadata.len() != identity.byte_len() {
        return Err(mismatch);
    }
    let length =
        usize::try_from(identity.byte_len()).map_err(|_| OnnxBackendFault::ResourceLimit)?;
    let mut file = File::open(selected).map_err(|_| unavailable)?;

    // Allocate the final shared storage directly. Converting a 25.9 MiB model
    // pair from Vec to Arc would retain a second full allocation during the
    // conversion, even though the exact accepted lengths are known here.
    let mut bytes = Arc::<[u8]>::new_uninit_slice(length);
    let Some(uninitialized) = Arc::get_mut(&mut bytes) else {
        return Err(OnnxBackendFault::ResourceLimit);
    };
    uninitialized.fill(MaybeUninit::new(0));
    // SAFETY: every element in the uniquely owned allocation was initialized to
    // a valid u8 immediately above. No reference to the uninitialized form
    // survives this conversion.
    let mut bytes = unsafe { bytes.assume_init() };
    let Some(buffer) = Arc::get_mut(&mut bytes) else {
        return Err(OnnxBackendFault::ResourceLimit);
    };
    file.read_exact(buffer).map_err(|_| unavailable)?;
    let mut trailing = [0_u8; 1];
    if file.read(&mut trailing).map_err(|_| unavailable)? != 0 {
        return Err(mismatch);
    }
    checkpoint(operation)?;

    OcrModelComponent::new(bytes, identity).map_err(|_| mismatch)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use mado_pilot_core::OperationContext;

    use super::accepted_source;
    use crate::OnnxBackendFault;

    #[test]
    fn a_relative_model_root_is_never_resolved_against_process_state() {
        assert_eq!(
            accepted_source(Path::new("rapidocr-models"), &OperationContext::new()),
            Err(OnnxBackendFault::InvalidModelRoot)
        );
    }

    #[test]
    fn a_missing_absolute_model_root_is_actionably_unavailable() {
        let missing = std::env::temp_dir().join(format!(
            "mado-pilot-g004-definitely-missing-{}",
            std::process::id()
        ));
        assert_eq!(
            accepted_source(&missing, &OperationContext::new()),
            Err(OnnxBackendFault::ModelRootUnavailable)
        );
    }
}
