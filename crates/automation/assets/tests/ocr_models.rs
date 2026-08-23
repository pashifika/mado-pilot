//! OCR model package equivalence, bounds, attack, and immutability evidence.

mod support;

use std::fs;
use std::sync::Arc;

use mado_pilot_assets::{AssetFaultKind, MANIFEST_PATH, MemoryPackage, PackageSource};
use serde_json::{Value, json};

use support::{ArchiveEntry, TempDir, hex_sha256, load, write_archive};

const DETECTOR_PATH: &str = "models/detector.onnx";
const RECOGNIZER_PATH: &str = "models/recognizer.onnx";
const DETECTOR: &[u8] = b"detector-model-bytes";
const RECOGNIZER: &[u8] = b"recognizer-model-bytes";

fn manifest_value(detector: &[u8], recognizer: &[u8]) -> Value {
    json!({
        "schema_version": 2,
        "package": { "id": "madopilot.test.ocr", "version": "1.0.0" },
        "license": "Apache-2.0",
        "templates": [],
        "ocr_models": [{
            "id": "test-model",
            "version": "1",
            "profile": "g-004-test-profile-v1",
            "language_profile": "japanese-basic-latin-v1",
            "preprocessing": "rapidocr-bgr-db736-v1",
            "decoder": "rapidocr-greedy-ctc-v1",
            "vocabulary": {
                "entries": 18_708,
                "content": {
                    "algorithm": "sha256",
                    "value": "f7aa897ca828a4c7c9e2739c30f9161a33306d532f020bcdb91dcfb664a5507e"
                }
            },
            "detector": {
                "path": DETECTOR_PATH,
                "byte_len": detector.len(),
                "content": { "algorithm": "sha256", "value": hex_sha256(detector) }
            },
            "recognizer": {
                "path": RECOGNIZER_PATH,
                "byte_len": recognizer.len(),
                "content": { "algorithm": "sha256", "value": hex_sha256(recognizer) }
            }
        }]
    })
}

fn manifest(detector: &[u8], recognizer: &[u8]) -> Vec<u8> {
    serde_json::to_vec(&manifest_value(detector, recognizer)).unwrap()
}

fn memory_source(
    manifest: Vec<u8>,
    detector: impl Into<Arc<[u8]>>,
    recognizer: impl Into<Arc<[u8]>>,
) -> PackageSource {
    PackageSource::memory(
        MemoryPackage::new()
            .with_entry(MANIFEST_PATH, manifest)
            .with_entry(DETECTOR_PATH, detector)
            .with_entry(RECOGNIZER_PATH, recognizer),
    )
}

#[test]
fn directory_memory_and_archive_resolve_identical_immutable_models() {
    let manifest = manifest(DETECTOR, RECOGNIZER);
    let directory = TempDir::new("ocr-equivalent");
    directory.write(MANIFEST_PATH, &manifest);
    directory.write(DETECTOR_PATH, DETECTOR);
    directory.write(RECOGNIZER_PATH, RECOGNIZER);

    let memory = load(&memory_source(
        manifest.clone(),
        DETECTOR.to_vec(),
        RECOGNIZER.to_vec(),
    ))
    .unwrap();
    let directory = load(&PackageSource::directory(directory.path())).unwrap();
    let archive = write_archive(
        &[
            ArchiveEntry::file(MANIFEST_PATH, &manifest),
            ArchiveEntry::file(DETECTOR_PATH, DETECTOR),
            ArchiveEntry::file(RECOGNIZER_PATH, RECOGNIZER),
        ],
        None,
    );
    let archive = load(&PackageSource::archive_bytes(archive)).unwrap();

    let expected = memory.resolve_ocr_model("test-model").unwrap();
    assert_eq!(directory.resolve_ocr_model("test-model").unwrap(), expected);
    assert_eq!(archive.resolve_ocr_model("test-model").unwrap(), expected);
    assert_eq!(expected.detector(), DETECTOR);
    assert_eq!(expected.recognizer(), RECOGNIZER);
}

#[test]
fn missing_metadata_and_over_limit_components_fail_before_expansion() {
    let mut missing = manifest_value(DETECTOR, RECOGNIZER);
    missing["ocr_models"][0]
        .as_object_mut()
        .unwrap()
        .remove("decoder");
    let source = PackageSource::memory(
        MemoryPackage::new().with_entry(MANIFEST_PATH, serde_json::to_vec(&missing).unwrap()),
    );
    assert_eq!(
        load(&source).unwrap_err().kind(),
        AssetFaultKind::InvalidOcrModelMetadata
    );

    let mut over_limit = manifest_value(DETECTOR, RECOGNIZER);
    over_limit["ocr_models"][0]["detector"]["byte_len"] = Value::from(64_u64 * 1024 * 1024 + 1);
    let source = PackageSource::memory(
        MemoryPackage::new().with_entry(MANIFEST_PATH, serde_json::to_vec(&over_limit).unwrap()),
    );
    assert_eq!(
        load(&source).unwrap_err().kind(),
        AssetFaultKind::InvalidOcrModelMetadata
    );
}

#[test]
fn truncated_and_hash_drifted_components_publish_no_model() {
    let manifest = manifest(DETECTOR, RECOGNIZER);
    let truncated = memory_source(
        manifest.clone(),
        DETECTOR[..DETECTOR.len() - 1].to_vec(),
        RECOGNIZER.to_vec(),
    );
    assert_eq!(
        load(&truncated).unwrap_err().kind(),
        AssetFaultKind::InvalidOcrModelMetadata
    );

    let mut drifted = DETECTOR.to_vec();
    drifted[0] ^= 1;
    let drifted = memory_source(manifest, drifted, RECOGNIZER.to_vec());
    assert_eq!(
        load(&drifted).unwrap_err().kind(),
        AssetFaultKind::HashMismatch
    );
}

#[test]
fn normalized_duplicate_and_traversing_model_paths_are_rejected() {
    let manifest = manifest(DETECTOR, RECOGNIZER);
    let duplicate = PackageSource::memory(
        MemoryPackage::new()
            .with_entry(MANIFEST_PATH, manifest)
            .with_entry(DETECTOR_PATH, DETECTOR.to_vec())
            .with_entry("models//detector.onnx", DETECTOR.to_vec())
            .with_entry(RECOGNIZER_PATH, RECOGNIZER.to_vec()),
    );
    assert_eq!(
        load(&duplicate).unwrap_err().kind(),
        AssetFaultKind::DuplicatePath
    );

    let mut traversal = manifest_value(DETECTOR, RECOGNIZER);
    traversal["ocr_models"][0]["detector"]["path"] = Value::from("../detector.onnx");
    let traversal = PackageSource::memory(
        MemoryPackage::new().with_entry(MANIFEST_PATH, serde_json::to_vec(&traversal).unwrap()),
    );
    assert_eq!(
        load(&traversal).unwrap_err().kind(),
        AssetFaultKind::UnsafePath
    );
}

#[test]
fn committed_memory_and_directory_models_do_not_follow_source_mutation() {
    let detector: Arc<[u8]> = Arc::from(DETECTOR);
    let recognizer: Arc<[u8]> = Arc::from(RECOGNIZER);
    let source = memory_source(
        manifest(DETECTOR, RECOGNIZER),
        Arc::clone(&detector),
        Arc::clone(&recognizer),
    );
    let package = load(&source).unwrap();
    let model = package.resolve_ocr_model("test-model").unwrap();

    let mut caller_detector = detector;
    Arc::make_mut(&mut caller_detector)[0] ^= 1;
    assert_eq!(model.detector(), DETECTOR);

    let directory = TempDir::new("ocr-mutation");
    directory.write(MANIFEST_PATH, &manifest(DETECTOR, RECOGNIZER));
    directory.write(DETECTOR_PATH, DETECTOR);
    directory.write(RECOGNIZER_PATH, RECOGNIZER);
    let committed = load(&PackageSource::directory(directory.path())).unwrap();
    fs::write(
        directory.path().join(DETECTOR_PATH),
        caller_detector.as_ref(),
    )
    .unwrap();

    assert_eq!(
        committed
            .resolve_ocr_model("test-model")
            .unwrap()
            .detector(),
        DETECTOR
    );
    assert_eq!(
        load(&PackageSource::directory(directory.path()))
            .unwrap_err()
            .kind(),
        AssetFaultKind::HashMismatch
    );
}

#[cfg(unix)]
#[test]
fn model_links_and_special_files_are_rejected_as_entries() {
    use std::os::unix::fs::symlink;

    let linked = TempDir::new("ocr-link");
    linked.write(MANIFEST_PATH, &manifest(DETECTOR, RECOGNIZER));
    linked.write("models/target.onnx", DETECTOR);
    linked.write(RECOGNIZER_PATH, RECOGNIZER);
    symlink("target.onnx", linked.path().join(DETECTOR_PATH)).unwrap();
    assert_eq!(
        load(&PackageSource::directory(linked.path()))
            .unwrap_err()
            .kind(),
        AssetFaultKind::UnsupportedEntryType
    );

    let mut special_detector = ArchiveEntry::file(DETECTOR_PATH, DETECTOR);
    special_detector.mode = 0o140_777;
    let special = write_archive(
        &[
            ArchiveEntry::file(MANIFEST_PATH, &manifest(DETECTOR, RECOGNIZER)),
            special_detector,
            ArchiveEntry::file(RECOGNIZER_PATH, RECOGNIZER),
        ],
        None,
    );
    assert_eq!(
        load(&PackageSource::archive_bytes(special))
            .unwrap_err()
            .kind(),
        AssetFaultKind::UnsupportedEntryType
    );
}
