//! Catalog-declared resource loading under one explicit model root.
//!
//! Each declared resource is read once into its final allocation, then
//! length/digest-validated before any session or vocabulary parser sees it.

use std::fs::File;
use std::io::Read;
use std::mem::MaybeUninit;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mado_pilot_core::OperationContext;
use mado_pilot_ocr::{
    ACCEPTED_G004_PROFILE_ID, OcrModelComponent, OcrModelIdentity, OcrModelSource,
    OcrModelSourceRequest, ResourceDescriptor, ocr_profile,
};

use crate::profile::ExecutionProfile;
use crate::{OnnxBackendFault, checkpoint};

/// Reads the selected profile's exact resources and commits one immutable source.
pub(crate) fn profile_source(
    model_root: &Path,
    profile: ExecutionProfile,
    identity: OcrModelIdentity,
    operation: &OperationContext,
) -> Result<OcrModelSource, OnnxBackendFault> {
    checkpoint(operation)?;
    let descriptor = profile.descriptor();
    let root = canonical_root(model_root)?;
    let released = ocr_profile(ACCEPTED_G004_PROFILE_ID);
    // Released G-004 faults name that exact resource; other resources get
    // profile-relative faults whose path/digest the caller reads from the catalog.
    let detector_faults = if released.is_some_and(|g004| *g004.detector == *descriptor.detector) {
        (
            OnnxBackendFault::DetectorUnavailable,
            OnnxBackendFault::DetectorMismatch,
        )
    } else {
        (
            OnnxBackendFault::ProfileDetectorUnavailable,
            OnnxBackendFault::ProfileDetectorMismatch,
        )
    };
    let recognizer_faults =
        if released.is_some_and(|g004| *g004.recognizer == *descriptor.recognizer) {
            (
                OnnxBackendFault::RecognizerUnavailable,
                OnnxBackendFault::RecognizerMismatch,
            )
        } else {
            (
                OnnxBackendFault::ProfileRecognizerUnavailable,
                OnnxBackendFault::ProfileRecognizerMismatch,
            )
        };
    let detector = read_component(&root, descriptor.detector, detector_faults, operation)?;
    let recognizer = read_component(&root, descriptor.recognizer, recognizer_faults, operation)?;
    let auxiliary = descriptor
        .auxiliary
        .map(|resource| {
            read_component(
                &root,
                resource,
                (
                    OnnxBackendFault::AuxiliaryUnavailable,
                    OnnxBackendFault::AuxiliaryMismatch,
                ),
                operation,
            )
        })
        .transpose()?;
    checkpoint(operation)?;

    let request = OcrModelSourceRequest {
        identity,
        detector,
        recognizer,
    };
    match auxiliary {
        None => OcrModelSource::new(request),
        Some(auxiliary) => OcrModelSource::new_with_auxiliary(request, auxiliary),
    }
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
    resource: &ResourceDescriptor,
    (unavailable, mismatch): (OnnxBackendFault, OnnxBackendFault),
    operation: &OperationContext,
) -> Result<OcrModelComponent, OnnxBackendFault> {
    checkpoint(operation)?;
    let identity = resource.identity();
    let selected = root
        .join(resource.relative_path)
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
    use std::fs;
    use std::path::{Path, PathBuf};

    use mado_pilot_core::{CancellationToken, OperationContext};
    use mado_pilot_ocr::{ACCEPTED_G004_PROFILE_ID, ocr_profile};

    use super::profile_source;
    use crate::OnnxBackendFault;
    use crate::profile::ExecutionProfile;

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new(name: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("mado-pilot-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("create temporary model root");
            Self(path.canonicalize().expect("canonical temporary model root"))
        }

        fn write(&self, relative: &str, bytes: &[u8]) -> PathBuf {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().expect("resource parent")).expect("resource parent");
            fs::write(&path, bytes).expect("write resource");
            path
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn g004_source(root: &Path, operation: &OperationContext) -> Result<(), OnnxBackendFault> {
        let (profile, identity) =
            ExecutionProfile::from_id(ACCEPTED_G004_PROFILE_ID).expect("accepted G-004");
        profile_source(root, profile, identity, operation).map(|_| ())
    }

    fn hex(digest: [u8; 32]) -> String {
        digest.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn a_relative_model_root_is_never_resolved_against_process_state() {
        assert_eq!(
            g004_source(Path::new("rapidocr-models"), &OperationContext::new()),
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
            g004_source(&missing, &OperationContext::new()),
            Err(OnnxBackendFault::ModelRootUnavailable)
        );
    }

    #[test]
    fn cancellation_precedes_any_model_root_access() {
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let operation = OperationContext::new().with_cancellation(cancellation);
        assert_eq!(
            g004_source(Path::new("relative"), &operation),
            Err(OnnxBackendFault::Cancelled)
        );
    }

    #[test]
    fn released_component_faults_keep_their_documented_resource() {
        let g004 = ocr_profile(ACCEPTED_G004_PROFILE_ID).expect("accepted G-004");
        for (resource, unavailable, mismatch) in [
            (
                g004.detector,
                OnnxBackendFault::DetectorUnavailable,
                OnnxBackendFault::DetectorMismatch,
            ),
            (
                g004.recognizer,
                OnnxBackendFault::RecognizerUnavailable,
                OnnxBackendFault::RecognizerMismatch,
            ),
        ] {
            assert!(unavailable.detail().ends_with(resource.relative_path));
            assert!(
                mismatch
                    .detail()
                    .contains(&format!("{} bytes", resource.byte_len))
            );
            assert!(mismatch.detail().ends_with(&hex(resource.sha256)));
        }

        let root = TempRoot::new("g004-component-faults");
        let operation = OperationContext::new();
        assert_eq!(
            g004_source(&root.0, &operation),
            Err(OnnxBackendFault::DetectorUnavailable)
        );
        root.write(g004.detector.relative_path, b"truncated");
        assert_eq!(
            g004_source(&root.0, &operation),
            Err(OnnxBackendFault::DetectorMismatch)
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_resource_symlink_leaving_the_model_root_is_refused() {
        let g004 = ocr_profile(ACCEPTED_G004_PROFILE_ID).expect("accepted G-004");
        let root = TempRoot::new("g004-symlink-root");
        let outside = TempRoot::new("g004-symlink-outside");
        let target = outside.write("detector.onnx", b"outside");
        let link = root.0.join(g004.detector.relative_path);
        fs::create_dir_all(link.parent().expect("link parent")).expect("link parent");
        std::os::unix::fs::symlink(target, link).expect("create escaping symlink");
        assert_eq!(
            g004_source(&root.0, &OperationContext::new()),
            Err(OnnxBackendFault::DetectorMismatch)
        );
    }

    #[test]
    fn declared_auxiliary_resource_is_captured_once_with_exact_length_and_digest() {
        use super::read_component;

        const CANDIDATE: &str = "ppocrv6-tiny-det-small-rec-max736-v1";
        let candidate = ocr_profile(CANDIDATE).expect("declared profile");
        let auxiliary = candidate.auxiliary.expect("declared vocabulary resource");
        let length = usize::try_from(auxiliary.byte_len).expect("auxiliary length fits");
        let faults = (
            OnnxBackendFault::AuxiliaryUnavailable,
            OnnxBackendFault::AuxiliaryMismatch,
        );
        let operation = OperationContext::new();
        let root = TempRoot::new("candidate-auxiliary");
        let read = || read_component(&root.0, auxiliary, faults, &operation).map(|_| ());

        assert_eq!(read(), Err(OnnxBackendFault::AuxiliaryUnavailable));
        for tampered in [
            vec![b'a'; length],
            vec![b'a'; length - 1],
            vec![b'a'; length + 1],
        ] {
            root.write(auxiliary.relative_path, &tampered);
            assert_eq!(read(), Err(OnnxBackendFault::AuxiliaryMismatch));
        }
        let path = root.0.join(auxiliary.relative_path);
        fs::remove_file(&path).expect("remove tampered resource");
        fs::create_dir_all(&path).expect("directory in place of resource");
        assert_eq!(read(), Err(OnnxBackendFault::AuxiliaryMismatch));

        let (profile, identity) = ExecutionProfile::from_id(CANDIDATE).expect("declared profile");
        assert_eq!(
            profile_source(&root.0, profile, identity, &operation).map(|_| ()),
            Err(OnnxBackendFault::ProfileDetectorUnavailable)
        );
    }
}
