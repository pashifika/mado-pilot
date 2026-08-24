//! Closed ONNX backend failures and privacy-safe support facts.

use std::fmt;

use mado_pilot_core::{Error, Interruption, Status};

/// One closed failure class owned by the ONNX OCR adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OnnxBackendFault {
    /// The runtime path was not an absolute canonical path to the reviewed filename.
    InvalidRuntimePath,
    /// The model root was not an explicit absolute directory.
    InvalidModelRoot,
    /// The caller-selected model root could not be opened.
    ModelRootUnavailable,
    /// The accepted detector was absent or unreadable at its fixed relative path.
    DetectorUnavailable,
    /// The accepted detector path, size, or digest did not match G-004.
    DetectorMismatch,
    /// The accepted recognizer was absent or unreadable at its fixed relative path.
    RecognizerUnavailable,
    /// The accepted recognizer path, size, or digest did not match G-004.
    RecognizerMismatch,
    /// The controlled runtime file could not be opened.
    RuntimeUnavailable,
    /// The runtime version or API table did not match the reviewed boundary.
    RuntimeIncompatible,
    /// Another process-global ONNX API or environment was initialized first.
    RuntimeConflict,
    /// The source did not carry the exact accepted G-004 identity.
    ProfileMismatch,
    /// The model graph or embedded vocabulary did not match the accepted profile.
    GraphMismatch,
    /// A checked tensor, output, candidate, or text ceiling would be exceeded.
    ResourceLimit,
    /// The single admitted inference slot is occupied.
    Busy,
    /// The backend has already been closed.
    Closed,
    /// Mapped pixels contradicted the accepted BGRA layout or bounds.
    InvalidPixels,
    /// ONNX Runtime or OpenCV rejected backend work.
    NativeFailure,
    /// Native output was malformed, non-finite, or outside the accepted profile.
    MalformedOutput,
    /// The caller cancelled the operation.
    Cancelled,
    /// The caller's absolute deadline passed.
    DeadlineExceeded,
}

impl OnnxBackendFault {
    /// Returns the public status class for this fault.
    #[must_use]
    pub const fn status(self) -> Status {
        match self {
            Self::InvalidRuntimePath
            | Self::InvalidModelRoot
            | Self::ProfileMismatch
            | Self::GraphMismatch => Status::InvalidArgument,
            Self::ModelRootUnavailable
            | Self::DetectorUnavailable
            | Self::RecognizerUnavailable
            | Self::RuntimeUnavailable
            | Self::RuntimeIncompatible
            | Self::RuntimeConflict => Status::Unsupported,
            Self::DetectorMismatch | Self::RecognizerMismatch => Status::AssetInvalid,
            Self::ResourceLimit | Self::Busy => Status::LimitExceeded,
            Self::Closed => Status::Closed,
            Self::InvalidPixels | Self::NativeFailure | Self::MalformedOutput => {
                Status::VisionFailed
            }
            Self::Cancelled => Status::Cancelled,
            Self::DeadlineExceeded => Status::DeadlineExceeded,
        }
    }

    /// Returns a static diagnostic detail with no host path, text, or native message.
    #[must_use]
    pub const fn detail(self) -> &'static str {
        match self {
            Self::InvalidRuntimePath => {
                "ONNX runtime path is not the controlled canonical target file"
            }
            Self::InvalidModelRoot => "accepted OCR model root is not an absolute directory",
            Self::ModelRootUnavailable => "accepted OCR model root is unavailable",
            Self::DetectorUnavailable => {
                "accepted OCR detector is unavailable at rapidocr-v3.9.2/ch_PP-OCRv4_det_mobile.onnx"
            }
            Self::DetectorMismatch => {
                "accepted OCR detector must be 4745517 bytes with SHA-256 d2a7720d45a54257208b1e13e36a8479894cb74155a5efe29462512d42f49da9"
            }
            Self::RecognizerUnavailable => {
                "accepted OCR recognizer is unavailable at rapidocr-v3.9.2/PP-OCRv6_rec_small.onnx"
            }
            Self::RecognizerMismatch => {
                "accepted OCR recognizer must be 21234383 bytes with SHA-256 6f327246b50388f3c176ae304bd95767ea6dc0c9ae92153ef8cbe210b3c14884"
            }
            Self::RuntimeUnavailable => "controlled ONNX runtime is unavailable",
            Self::RuntimeIncompatible => "controlled ONNX runtime is incompatible",
            Self::RuntimeConflict => "a conflicting ONNX runtime is already initialized",
            Self::ProfileMismatch => "OCR source does not match the accepted G-004 profile",
            Self::GraphMismatch => "OCR graph metadata does not match the accepted profile",
            Self::ResourceLimit => "ONNX backend resource ceiling would be exceeded",
            Self::Busy => "ONNX backend inference slot is occupied",
            Self::Closed => "ONNX backend is closed",
            Self::InvalidPixels => "OCR pixels do not match the accepted bounded BGRA layout",
            Self::NativeFailure => "ONNX backend native operation failed",
            Self::MalformedOutput => "ONNX backend produced malformed output",
            Self::Cancelled => "operation was cancelled",
            Self::DeadlineExceeded => "operation deadline passed",
        }
    }
}

impl From<Interruption> for OnnxBackendFault {
    fn from(interruption: Interruption) -> Self {
        match interruption {
            Interruption::Cancelled => Self::Cancelled,
            Interruption::DeadlineExceeded => Self::DeadlineExceeded,
        }
    }
}

impl From<OnnxBackendFault> for Error {
    fn from(fault: OnnxBackendFault) -> Self {
        Self::new(fault.status(), fault.detail())
    }
}

impl fmt::Display for OnnxBackendFault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.detail())
    }
}

impl std::error::Error for OnnxBackendFault {}

/// The only execution provider this backend can select.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OnnxExecutionProvider {
    /// ONNX Runtime's built-in CPU execution provider.
    Cpu,
}

/// The reviewed native compatibility boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OnnxRuntimeCompatibility {
    /// ONNX Runtime 1.29.0 through C API 17.
    Version1_29Api17,
}

/// Privacy-safe, closed observations about one opened backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OnnxBackendFacts {
    provider: OnnxExecutionProvider,
    runtime: OnnxRuntimeCompatibility,
    max_concurrent_inferences: u32,
    max_tensor_bytes: u64,
    max_output_bytes: u64,
    max_detector_candidates: u32,
    recognition_batch: u32,
}

impl OnnxBackendFacts {
    pub(crate) const fn accepted(
        max_tensor_bytes: u64,
        max_output_bytes: u64,
        max_detector_candidates: u32,
        recognition_batch: u32,
    ) -> Self {
        Self {
            provider: OnnxExecutionProvider::Cpu,
            runtime: OnnxRuntimeCompatibility::Version1_29Api17,
            max_concurrent_inferences: 1,
            max_tensor_bytes,
            max_output_bytes,
            max_detector_candidates,
            recognition_batch,
        }
    }

    /// Returns the selected execution provider.
    #[must_use]
    pub const fn provider(self) -> OnnxExecutionProvider {
        self.provider
    }

    /// Returns the reviewed runtime compatibility boundary.
    #[must_use]
    pub const fn runtime(self) -> OnnxRuntimeCompatibility {
        self.runtime
    }

    /// Returns the number of calls admitted concurrently.
    #[must_use]
    pub const fn max_concurrent_inferences(self) -> u32 {
        self.max_concurrent_inferences
    }

    /// Returns the per-input-tensor byte ceiling.
    #[must_use]
    pub const fn max_tensor_bytes(self) -> u64 {
        self.max_tensor_bytes
    }

    /// Returns the per-native-output byte ceiling.
    #[must_use]
    pub const fn max_output_bytes(self) -> u64 {
        self.max_output_bytes
    }

    /// Returns the detector candidate ceiling.
    #[must_use]
    pub const fn max_detector_candidates(self) -> u32 {
        self.max_detector_candidates
    }

    /// Returns the fixed recognition batch ceiling.
    #[must_use]
    pub const fn recognition_batch(self) -> u32 {
        self.recognition_batch
    }
}
