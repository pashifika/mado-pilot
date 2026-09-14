#![cfg_attr(not(any(windows, target_os = "macos")), allow(missing_docs))]
#![cfg(any(windows, target_os = "macos"))]
//! Selected-only native configuration validation without discovery or capture.

use std::time::Duration;

use mado_pilot::{
    CancellationToken, CapturePacingRequest, DefaultOcrConfig, NativeEngineRequest,
    OcrExecutionProviderPolicy, OcrProfile, OcrProfileConfig, OcrProviderConfig,
    OcrProviderProfile, OperationContext, Status,
};
#[cfg(target_os = "macos")]
use mado_pilot::{
    MacosConfig as PlatformConfig, macos_engine as native_engine,
    macos_engine_with_default_ocr as native_default_ocr,
    macos_engine_with_ocr_profile as native_profile_ocr,
    macos_engine_with_ocr_provider as native_provider_ocr,
};
#[cfg(windows)]
use mado_pilot::{
    WindowsConfig as PlatformConfig, windows_engine as native_engine,
    windows_engine_with_default_ocr as native_default_ocr,
    windows_engine_with_ocr_profile as native_profile_ocr,
    windows_engine_with_ocr_provider as native_provider_ocr,
};

fn platform(request: NativeEngineRequest, config: PlatformConfig) -> NativeEngineRequest {
    #[cfg(windows)]
    return request.with_windows_config(config);
    #[cfg(target_os = "macos")]
    return request.with_macos_config(config);
}

fn oversized() -> CapturePacingRequest {
    CapturePacingRequest::preferred(Duration::MAX).expect("positive neutral duration")
}

#[test]
fn selected_os_default_overrides_common_independently_of_setter_order() {
    let config = PlatformConfig::new().with_capture_pacing(
        CapturePacingRequest::required(Duration::from_millis(60)).expect("positive"),
    );
    let common_first = platform(
        NativeEngineRequest::new().with_capture_pacing(oversized()),
        config,
    );
    let os_first = platform(NativeEngineRequest::new(), config).with_capture_pacing(oversized());
    for request in [common_first, os_first] {
        let engine = native_engine(request).expect("only the selected 60ms needs representation");
        drop(engine);
    }
}

#[test]
fn replacing_os_block_exposes_common_but_source_default_bypasses_it() {
    let common = NativeEngineRequest::new().with_capture_pacing(oversized());
    let reset = platform(
        common,
        PlatformConfig::new().with_capture_pacing(CapturePacingRequest::source_default()),
    );
    let independent = reset.clone();
    let inheriting = platform(reset, PlatformConfig::new());
    assert_eq!(
        native_engine(inheriting)
            .expect_err("common overflow becomes selected")
            .status(),
        Status::InvalidArgument,
    );
    let engine = native_engine(independent).expect("other request retains source-default reset");
    drop(engine);
}

#[test]
fn every_native_constructor_rejects_selected_overflow_before_model_loading() {
    let request = NativeEngineRequest::new().with_capture_pacing(oversized());
    let root = std::env::current_dir().expect("working directory");
    let model = root.join("not-installed-pacing-contract-model");
    let runtime = root.join("not-installed-pacing-contract-runtime");
    let default = DefaultOcrConfig::new(&model, &runtime);
    let profile = OcrProfileConfig::new(OcrProfile::BoundedDetector, &model, &runtime);
    let provider = OcrProviderConfig::new(
        OcrProviderProfile::BoundedDetector,
        OcrExecutionProviderPolicy::Cpu,
        &model,
        &runtime,
    );
    let operation = OperationContext::new();
    for result in [
        native_engine(request.clone()),
        native_default_ocr(request.clone(), &default, &operation),
        native_profile_ocr(request.clone(), &profile, &operation),
        native_provider_ocr(request, &provider, &operation),
    ] {
        assert_eq!(
            result.expect_err("unrepresentable selection").status(),
            Status::InvalidArgument
        );
    }
}

#[test]
fn interrupted_construction_keeps_admission_precedence_over_invalid_pacing() {
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let operation = OperationContext::new().with_cancellation(cancellation);
    let config = DefaultOcrConfig::new("not-read", "not-read");
    let request = NativeEngineRequest::new().with_capture_pacing(oversized());
    assert_eq!(
        native_default_ocr(request, &config, &operation)
            .expect_err("interrupted admission")
            .status(),
        Status::Cancelled,
    );
}
