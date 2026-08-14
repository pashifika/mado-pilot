/*
 * MadoPilot C++ wrapper - ABI 1.2.
 *
 * A header-only RAII adapter over the released C ABI. It owns handles, turns
 * statuses into an exception-free `Result`, and copies the text a caller needs
 * after a C handle is gone. It performs no capture, mapping, matching,
 * coordinate, input, diagnostic, status, or error logic of its own: every
 * answer here came from a C table entry.
 *
 * ============================================================================
 * This header declares no ABI of its own.
 *
 * The only ABI is the C one: its complete 1.0 prefix was frozen by
 * docs/adr/0007-phase-1-c-abi-freeze.md, and ABI 1.2 replaces the unreleased
 * 1.1 draft with the suffix reviewed for Phase 2. Nothing below restates a
 * numeric value from that contract: the enumerated types are aliases of the C
 * types, so a caller writes `MADOPILOT_STATUS_OK` and gets whatever the header
 * it compiled against says that is. A hand-written mirror fails silently when
 * the C set grows - it compiles, one value short - and freezing the values does
 * not remove that risk.
 * ============================================================================
 *
 * Requires C++17. See docs/cpp-wrapper.md for the ownership rules and
 * docs/c-abi.md for the contract underneath them.
 */

#ifndef MADOPILOT_MADOPILOT_HPP
#define MADOPILOT_MADOPILOT_HPP

#include "madopilot/madopilot.h"

#include <cassert>
#include <cstddef>
#include <cstdint>
#include <optional>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

/* MSVC reports 199711L in `__cplusplus` unless /Zc:__cplusplus is passed, and
 * puts the real value in `_MSVC_LANG`. Reading the wrong one would reject a
 * conforming compiler. No macro is left defined afterwards. */
#if (defined(_MSVC_LANG) && _MSVC_LANG < 201703L) || \
    (!defined(_MSVC_LANG) && __cplusplus < 201703L)
#  error "madopilot.hpp requires C++17 or later"
#endif

namespace madopilot {

/* ---------------------------------------------------------------------------
 * The C vocabulary, unchanged
 *
 * These are aliases, not new enumerations. Re-declaring the C constants as
 * `enum class` members would create a second vocabulary that has to be reviewed
 * and frozen alongside the first, and it would compile happily while the two
 * drifted apart. A caller writes the `MADOPILOT_*` constant the C header and
 * docs/c-abi.md already document.
 * ------------------------------------------------------------------------ */

using Status = ::madopilot_status_t;
using ErrorCategory = ::madopilot_error_category_t;
using Space = ::madopilot_space_t;
using PixelFormat = ::madopilot_pixel_format_t;
using ClipPolicy = ::madopilot_clip_policy_t;
using Continuity = ::madopilot_continuity_t;
using Suppression = ::madopilot_suppression_t;
using SourceKind = ::madopilot_source_kind_t;
using PackageSourceKind = ::madopilot_package_source_kind_t;
using AssetFault = ::madopilot_asset_fault_t;
using AssetStage = ::madopilot_asset_stage_t;
using PermissionKind = ::madopilot_permission_kind_t;
using PermissionState = ::madopilot_permission_state_t;
using DiagnosticCategory = ::madopilot_diagnostic_category_t;
using TargetKind = ::madopilot_target_kind_t;
using CapabilitySupport = ::madopilot_capability_support_t;
using InputOperationKind = ::madopilot_input_operation_kind_t;
using InputDelivery = ::madopilot_input_delivery_t;
using InputAddressScope = ::madopilot_input_address_scope_t;
using SubmissionEvidence = ::madopilot_submission_evidence_t;
using InputRequirement = ::madopilot_input_requirement_t;
using FocusPolicy = ::madopilot_focus_policy_t;
using GeometryPolicy = ::madopilot_geometry_policy_t;
using PointerButton = ::madopilot_pointer_button_t;
using Key = ::madopilot_key_t;
using Modifier = ::madopilot_modifier_t;
using InputEventKind = ::madopilot_input_event_kind_t;
using SequenceOutcome = ::madopilot_sequence_outcome_t;
using CleanupState = ::madopilot_cleanup_state_t;
using InputFault = ::madopilot_input_fault_t;
using DiagnosticLevel = ::madopilot_diagnostic_level_t;
using DiagnosticDrainState = ::madopilot_diagnostic_drain_state_t;
using DiagnosticKind = ::madopilot_diagnostic_kind_t;
using DiagnosticOperationKind = ::madopilot_diagnostic_operation_kind_t;
using InputRevalidationCategory = ::madopilot_input_revalidation_category_t;
using InputGeometryResult = ::madopilot_input_geometry_result_t;
using SearchDiagnosticOutcome = ::madopilot_search_diagnostic_outcome_t;
using Lifecycle = ::madopilot_lifecycle_t;

/* Structures with no borrowed view are passed through as themselves, for the
 * same reason: their fields are the contract's, and a projection would be a
 * second copy of a frozen layout that could only ever drift from it. */
using Rect = ::madopilot_pixel_rect_t;
using FrameStamp = ::madopilot_frame_stamp_t;
using FrameInfo = ::madopilot_frame_info_t;
using SessionInfo = ::madopilot_session_info_t;
using EffectiveMatchOptions = ::madopilot_match_options_t;

/// True when a status is the success value.
inline bool is_ok(Status status) noexcept { return status == MADOPILOT_STATUS_OK; }

/* ---------------------------------------------------------------------------
 * Borrowed views
 *
 * The C ABI hands back pointer-length views into memory a handle owns. These
 * two types say so in their name and offer the copy that outlives the owner.
 *
 * Every accessor that produces one is declared `const&` with its `const&&`
 * overload deleted, so it cannot be called on a temporary owner. A view taken
 * from a temporary dangles at the end of the full expression that made it, which
 * is a use-after-free that reads correct — `load(...).take().describe()` — and
 * this is what turns it into a compile error instead. Name the owner, then ask
 * it. Accessors that copy, and the ones whose views live in the library's own
 * static storage, are unqualified.
 * ------------------------------------------------------------------------ */

/// UTF-8 text borrowed from a retained owner.
///
/// Valid only while the owner named by the accessor that produced it is
/// retained. Call `to_string()` for text that must outlive it.
class BorrowedStr {
public:
    BorrowedStr() noexcept = default;

    explicit BorrowedStr(::madopilot_str_t view) noexcept : view_(view) {}

    /// The borrowed text. Empty rather than null when the library wrote no view.
    std::string_view view() const noexcept {
        return view_.data == nullptr ? std::string_view{}
                                     : std::string_view(view_.data, view_.len);
    }

    // NOLINTNEXTLINE(google-explicit-constructor) — a borrowed view is a
    // string_view; requiring a cast at every use would only add noise.
    operator std::string_view() const noexcept { return view(); }

    /// An owned copy whose lifetime is independent of the C handle.
    std::string to_string() const { return std::string(view()); }

    const char* data() const noexcept { return view_.data; }
    std::size_t size() const noexcept { return view_.len; }
    bool empty() const noexcept { return view_.len == 0; }

    /// The C view this wraps, for a caller that needs it back.
    ::madopilot_str_t raw() const noexcept { return view_; }

private:
    ::madopilot_str_t view_{nullptr, 0};
};

/// Bytes borrowed from a retained owner, on the same terms as `BorrowedStr`.
class BorrowedBytes {
public:
    BorrowedBytes() noexcept = default;

    explicit BorrowedBytes(::madopilot_bytes_t view) noexcept : view_(view) {}

    const std::uint8_t* data() const noexcept { return view_.data; }
    std::size_t size() const noexcept { return view_.len; }
    bool empty() const noexcept { return view_.len == 0; }
    const std::uint8_t* begin() const noexcept { return view_.data; }
    const std::uint8_t* end() const noexcept {
        return view_.data == nullptr ? nullptr : view_.data + view_.len;
    }

    /// An owned copy whose lifetime is independent of the C handle.
    std::vector<std::uint8_t> to_vector() const {
        return view_.data == nullptr ? std::vector<std::uint8_t>{}
                                     : std::vector<std::uint8_t>(begin(), end());
    }

    ::madopilot_bytes_t raw() const noexcept { return view_; }

private:
    ::madopilot_bytes_t view_{nullptr, 0};
};

namespace detail {

/// Value-initializes a versioned structure and declares its size.
///
/// Every extensible C structure begins with `struct_size`, and a caller sets it
/// to `sizeof` as its own header declares it. Doing that here once means no call
/// site can forget it or write a stale number.
template <class T>
inline T sized() noexcept {
    T value{};
    value.struct_size = static_cast<std::uint32_t>(sizeof(T));
    return value;
}

inline ::madopilot_str_t as_str(std::string_view text) noexcept {
    ::madopilot_str_t view{};
    view.data = text.data();
    view.len = text.size();
    return view;
}

inline ::madopilot_bytes_t as_bytes(const std::uint8_t* data, std::size_t len) noexcept {
    ::madopilot_bytes_t view{};
    view.data = data;
    view.len = len;
    return view;
}

/// True only when both sides of negotiation cover `required`.
///
/// The library reports its own extent in the mandatory table prefix. The
/// wrapper also retains the caller extent passed to negotiation: checking only
/// the larger library table would let a caller that deliberately negotiated a
/// 1.0 prefix invoke a 1.2 member it did not claim to understand.
inline bool has_entry(const ::madopilot_api_t* api, std::size_t negotiated_extent,
                      std::size_t required) noexcept {
    return api != nullptr && negotiated_extent >= required &&
           static_cast<std::size_t>(api->struct_size) >= required;
}

/// Releases one owned C error handle when the scope ends, however it ends.
///
/// Copying an error's text out of the C handle allocates, and an allocation can
/// throw. The release has to happen on that path too — the C header and
/// ADR 0005 both say a described error is released unconditionally — and a
/// `try`/`catch` would state the rule twice and leave a third path, a later
/// `return` before the catch, to state it a third time.
class ErrorGuard {
public:
    ErrorGuard(const ::madopilot_api_t* api, ::madopilot_error_t* error) noexcept
        : api_(api), error_(error) {}

    ErrorGuard(const ErrorGuard&) = delete;
    ErrorGuard& operator=(const ErrorGuard&) = delete;
    ErrorGuard(ErrorGuard&&) = delete;
    ErrorGuard& operator=(ErrorGuard&&) = delete;

    ~ErrorGuard() { api_->error_release(error_); }

private:
    const ::madopilot_api_t* api_;
    ::madopilot_error_t* error_;
};

} // namespace detail

/* ---------------------------------------------------------------------------
 * Errors
 * ------------------------------------------------------------------------ */

class Error;

namespace detail {

/// Describes an owned C error, copies everything out of it, and releases it.
///
/// Declared before `Error` so that `Error` can befriend it by name. Defined
/// below, once `Error` is complete.
inline Error take_error(const ::madopilot_api_t* api, Status status,
                        ::madopilot_error_t* error);

} // namespace detail

/// Which asset rule was broken, and how far loading had got.
///
/// Package loading can distinguish failures which share one status: a bad
/// content hash and an unsafe entry path are both
/// `MADOPILOT_STATUS_ASSET_INVALID`. Collapsing the pair into the status would
/// throw away detail the C ABI keeps on purpose.
struct AssetDetail {
    AssetFault fault = MADOPILOT_ASSET_FAULT_UNKNOWN;
    AssetStage stage = MADOPILOT_ASSET_STAGE_UNKNOWN;
};

/// A failure, owned by C++.
///
/// Constructing one copies everything out of the C error handle and releases it
/// immediately, so nothing here borrows and there is no global or thread-local
/// last-error slot to consult. A failure belongs to the call that produced it.
class Error {
public:
    Error() noexcept = default;

    /// A failure with a status and no error handle, which is what an accessor
    /// that takes no `out_error` reports.
    static Error from_status(Status status) {
        Error error;
        error.status_ = status;
        return error;
    }

    Status status() const noexcept { return status_; }
    ErrorCategory category() const noexcept { return category_; }

    /// Redacted diagnostic text, owned. Never required for control flow, and
    /// never carrying captured pixels or recognized text.
    const std::string& message() const noexcept { return message_; }

    /// The backend that failed, when the library named one.
    const std::optional<std::string>& backend() const noexcept { return backend_; }

    /// The asset fault and stage, when the failure came from package loading.
    const std::optional<AssetDetail>& asset_detail() const noexcept { return asset_; }

    bool ok() const noexcept { return status_ == MADOPILOT_STATUS_OK; }

private:
    friend Error detail::take_error(const ::madopilot_api_t* api, Status status,
                                    ::madopilot_error_t* error);

    Status status_ = MADOPILOT_STATUS_OK;
    ErrorCategory category_ = MADOPILOT_ERROR_CATEGORY_UNSPECIFIED;
    std::string message_;
    std::optional<std::string> backend_;
    std::optional<AssetDetail> asset_;
};

namespace detail {

/// Called on every failing path, including the ones whose caller never looks at
/// the text: the handle is released either way. "Either way" includes the copies
/// below throwing `std::bad_alloc`, which is why the release is a scope guard
/// rather than the last statement.
inline Error take_error(const ::madopilot_api_t* api, Status status,
                        ::madopilot_error_t* error) {
    Error out = Error::from_status(status);
    if (api == nullptr || error == nullptr) {
        return out;
    }
    const detail::ErrorGuard release(api, error);

    ::madopilot_error_detail_t detail = detail::sized<::madopilot_error_detail_t>();
    if (api->error_describe(error, &detail) == MADOPILOT_STATUS_OK) {
        // The status the error carries is the authority; the one the entry
        // returned is the same value, and this keeps them from diverging.
        out.status_ = detail.status;
        out.category_ = detail.category;
        out.message_ = BorrowedStr(detail.message).to_string();
        if ((detail.flags & MADOPILOT_ERROR_HAS_BACKEND) != 0u) {
            out.backend_ = BorrowedStr(detail.backend).to_string();
        }
        if ((detail.flags & MADOPILOT_ERROR_HAS_ASSET_DETAIL) != 0u) {
            out.asset_ = AssetDetail{detail.asset_fault, detail.asset_stage};
        }
    }

    return out;
}

} // namespace detail

/* ---------------------------------------------------------------------------
 * Result
 *
 * The default interface. No wrapper operation throws to report a MadoPilot
 * failure: every failure the library can report arrives as a status in a
 * `Result`, and no status is ever translated into an exception.
 *
 * What the wrapper does throw is what its own allocations throw, `std::bad_alloc`
 * — from `detail::take_error` copying an error's text, from the vector
 * `MatchResult::matches` fills, from the copies a typed request keeps of what its
 * C structure points at, and from an explicit `BorrowedStr::to_string` or
 * `BorrowedBytes::to_vector`. Every one of those is the wrapper making an owned
 * copy for the caller. `detail::take_error` releases its C handle on that path too.
 *
 * That is why `value()` and `take()` carry the precondition `ok()`: an
 * exception-free extractor cannot report a failed result, so checking is the
 * caller's step. `assert` catches the mistake in a build without `NDEBUG`.
 *
 * Both templates are `[[nodiscard]]`. A dropped `Result` is a dropped failure in
 * a surface that reports failures no other way, so the compiler says so.
 * ------------------------------------------------------------------------ */

/// A value or a failure. Move-only when `T` is.
template <class T>
class [[nodiscard]] Result {
public:
    static Result success(T value) {
        Result result;
        result.value_.emplace(std::move(value));
        return result;
    }

    static Result failure(Error error) {
        Result result;
        result.error_ = std::move(error);
        return result;
    }

    bool ok() const noexcept { return value_.has_value(); }
    explicit operator bool() const noexcept { return ok(); }

    /// The C status. `MADOPILOT_STATUS_OK` exactly when `ok()`.
    Status status() const noexcept { return error_.status(); }

    /// The failure. On success this is a default error whose status is OK.
    const Error& error() const noexcept { return error_; }

    /// The value. **Precondition: `ok()`.**
    ///
    /// Reading it on a failed result is undefined behaviour: the value lives in
    /// a `std::optional` that a failure leaves disengaged, and this dereferences
    /// it rather than checking. A build without `NDEBUG` fails the `assert`
    /// instead.
    ///
    /// It stays `noexcept` on purpose. Throwing here would be a second failure
    /// channel in a surface whose whole premise is that a MadoPilot failure
    /// arrives as a status, so the check belongs at the call site — `if (!r)`
    /// before `r.value()`.
    T& value() noexcept {
        assert(value_.has_value() && "madopilot::Result::value() on a failed result");
        return *value_;
    }
    const T& value() const noexcept {
        assert(value_.has_value() && "madopilot::Result::value() on a failed result");
        return *value_;
    }

    /// Moves the value out. **Precondition: `ok()`**, on the same terms as
    /// `value()`.
    T take() {
        assert(value_.has_value() && "madopilot::Result::take() on a failed result");
        return std::move(*value_);
    }

private:
    Result() = default;

    std::optional<T> value_;
    /// Default-constructed, and therefore OK, on the success path.
    Error error_;
};

/// A completed-or-failed operation with no value, such as `Session::close`.
template <>
class [[nodiscard]] Result<void> {
public:
    static Result success() { return Result(); }

    static Result failure(Error error) {
        Result result;
        result.error_ = std::move(error);
        return result;
    }

    bool ok() const noexcept { return error_.ok(); }
    explicit operator bool() const noexcept { return ok(); }
    Status status() const noexcept { return error_.status(); }
    const Error& error() const noexcept { return error_; }

private:
    Result() = default;

    Error error_;
};

namespace detail {

/// The status for "there is no library to ask".
///
/// The wrapper originates a status in exactly two places, both of them this
/// one: an owner that never held a table, and — as
/// `MADOPILOT_STATUS_INTERNAL` — a negotiation that reported success without
/// returning one. Every other status in a `Result` came from a C entry,
/// including the refusal an emptied owner gets: an emptied owner keeps the
/// table it came from and forwards its null handle to it.
template <class T>
inline Result<T> no_table() {
    return Result<T>::failure(Error::from_status(MADOPILOT_STATUS_INVALID_ARGUMENT));
}

template <class T>
inline Result<T> unsupported() {
    return Result<T>::failure(Error::from_status(MADOPILOT_STATUS_UNSUPPORTED));
}

/// A move-only owner of one reference-counted C handle.
///
/// `Derived` supplies `retain_handle` and `release_handle`. Copy operations are
/// deleted so that a refcount bump is never hidden behind an assignment; the
/// explicit way to take a second reference is `clone()`.
template <class Derived, class Handle>
class Owner {
public:
    Owner() noexcept = default;

    Owner(const Owner&) = delete;
    Owner& operator=(const Owner&) = delete;

    Owner(Owner&& other) noexcept
        : api_(other.api_), extent_(other.extent_), handle_(other.handle_) {
        other.handle_ = nullptr;
    }

    Owner& operator=(Owner&& other) noexcept {
        if (this != &other) {
            reset();
            api_ = other.api_;
            extent_ = other.extent_;
            handle_ = other.handle_;
            other.handle_ = nullptr;
        }
        return *this;
    }

    /// Releases the owned reference. Never throws, and never reports: a
    /// destructor cannot answer a caller, which is why a failable operation such
    /// as session close stays explicit.
    ~Owner() { reset(); }

    /// True when this owner holds no reference.
    bool empty() const noexcept { return handle_ == nullptr; }
    explicit operator bool() const noexcept { return handle_ != nullptr; }

    /// The owned handle, or null. Borrowed: it stays valid while this owner does.
    Handle* get() const noexcept { return handle_; }

    /// The negotiated table this owner calls through, or null.
    ///
    /// An emptied owner keeps it, so an operation on an emptied owner is refused
    /// by the C boundary with its own status rather than by the wrapper.
    const ::madopilot_api_t* api() const noexcept { return api_; }

    /// The table prefix this owner was created under.
    std::size_t extent() const noexcept { return extent_; }

    /// Drops the owned reference and leaves this owner empty.
    void reset() noexcept {
        if (handle_ != nullptr) {
            Derived::release_handle(api_, handle_);
            handle_ = nullptr;
        }
    }

    /// Gives up the reference without releasing it. The caller owns it now.
    Handle* release() noexcept {
        Handle* handle = handle_;
        handle_ = nullptr;
        return handle;
    }

    /// Takes a second owned reference, explicitly.
    ///
    /// Both owners remain valid and independent; the referenced state lives
    /// until the last one is destroyed. Cloning an empty owner yields an empty
    /// owner, and so does a retain that did not happen.
    Derived clone() const noexcept {
        if (handle_ == nullptr) {
            Derived copy;
            copy.api_ = api_;
            copy.extent_ = extent_;
            return copy;
        }

        const Status status = Derived::retain_handle(api_, handle_);
        // No released retain reports anything but OK for a non-null handle, so
        // this cannot fire against the current ABI. It is here because the one
        // thing that must not happen if a later one can fail is the second
        // owner being built anyway: it would hold a reference nobody took, and
        // release it when the shorter of the two lifetimes ends.
        assert(is_ok(status) && "madopilot: retain refused a live handle");
        if (!is_ok(status)) {
            Derived copy;
            copy.api_ = api_;
            copy.extent_ = extent_;
            return copy;
        }

        return Derived(api_, extent_, handle_);
    }

protected:
    Owner(const ::madopilot_api_t* api, std::size_t extent, Handle* handle) noexcept
        : api_(api), extent_(extent), handle_(handle) {}

    const ::madopilot_api_t* api_ = nullptr;
    std::size_t extent_ = 0;
    Handle* handle_ = nullptr;
};

} // namespace detail

/* ---------------------------------------------------------------------------
 * Typed requests
 *
 * These own their array and string storage and identify every borrowed handle,
 * so a `to_c()` value is usable for the documented call lifetime.
 * ------------------------------------------------------------------------ */

class Cancellation;

/// A deadline, cancellation token, and optional diagnostic correlation tag
/// carried into every blocking call.
///
/// The deadline is an ABSOLUTE instant in the library's own monotonic domain,
/// read from `Api::clock_now()` and added to. It is not a duration and not a
/// wall clock. A default `Operation` has no deadline, cancellation, or tag.
///
/// An operation borrows its cancellation token: the `Cancellation` owner must
/// outlive every call this operation is passed to.
class Operation {
public:
    Operation() noexcept = default;

    /// Sets the absolute deadline, in nanoseconds of the library's clock domain.
    Operation& deadline(std::uint64_t absolute_nanos) noexcept {
        has_deadline_ = true;
        deadline_ = absolute_nanos;
        return *this;
    }

    Operation& no_deadline() noexcept {
        has_deadline_ = false;
        deadline_ = 0;
        return *this;
    }

    /// Borrows a cancellation token. The token must outlive this operation's use.
    Operation& cancellation(const Cancellation& token) noexcept;

    Operation& no_cancellation() noexcept {
        cancellation_ = nullptr;
        return *this;
    }
    /// Sets an opaque nonzero diagnostic correlation value.
    Operation& activity_tag(std::uint64_t tag) noexcept {
        has_activity_tag_ = true;
        activity_tag_ = tag;
        return *this;
    }

    Operation& no_activity_tag() noexcept {
        has_activity_tag_ = false;
        activity_tag_ = 0;
        return *this;
    }


    ::madopilot_operation_t to_c() const noexcept {
        auto value = detail::sized<::madopilot_operation_t>();
        value.flags = (has_deadline_ ? MADOPILOT_OPERATION_HAS_DEADLINE : 0u) |
                      (has_activity_tag_ ? MADOPILOT_OPERATION_HAS_ACTIVITY_TAG
                                         : 0u);
        value.deadline_nanos = deadline_;
        value.cancellation = cancellation_;
        value.activity_tag = activity_tag_;
        return value;
    }

private:
    bool has_deadline_ = false;
    std::uint64_t deadline_ = 0;
    const ::madopilot_cancellation_t* cancellation_ = nullptr;
    bool has_activity_tag_ = false;
    std::uint64_t activity_tag_ = 0;
};

/// Engine-wide diagnostic configuration. The default allocates no queue.
class EngineOptions {
public:
    EngineOptions() noexcept = default;

    EngineOptions& diagnostics(DiagnosticLevel level,
                               std::uint32_t capacity) noexcept {
        diagnostic_level_ = level;
        diagnostic_capacity_ = capacity;
        return *this;
    }

    EngineOptions& diagnostics_off() noexcept {
        diagnostic_level_ = MADOPILOT_DIAGNOSTIC_LEVEL_OFF;
        diagnostic_capacity_ = 0;
        return *this;
    }

    ::madopilot_engine_options_t to_c() const noexcept {
        auto value = detail::sized<::madopilot_engine_options_t>();
        value.diagnostic_level = diagnostic_level_;
        value.diagnostic_capacity = diagnostic_capacity_;
        return value;
    }

private:
    DiagnosticLevel diagnostic_level_ = MADOPILOT_DIAGNOSTIC_LEVEL_OFF;
    std::uint32_t diagnostic_capacity_ = 0;
};

/// One replay frame supplied as raw pixels.
///
/// The pixels are borrowed until `Api::create_engine` returns, which copies
/// them; the caller's storage is its own again afterwards.
class ReplayFrame {
public:
    ReplayFrame() noexcept = default;

    ReplayFrame& extent(std::uint32_t width, std::uint32_t height) noexcept {
        width_ = width;
        height_ = height;
        return *this;
    }

    ReplayFrame& format(PixelFormat format) noexcept {
        format_ = format;
        return *this;
    }

    ReplayFrame& continuity(Continuity continuity) noexcept {
        continuity_ = continuity;
        return *this;
    }

    ReplayFrame& pixels(const std::uint8_t* data, std::size_t len) noexcept {
        pixels_ = data;
        pixels_len_ = len;
        return *this;
    }

    /// Places the frame at an instant in the replay timeline. Omitted, it sits
    /// at the clock origin.
    ReplayFrame& captured_at(std::uint64_t nanos) noexcept {
        captured_at_ = nanos;
        return *this;
    }

    /// Bytes per row for a padded source. Omitted, rows are packed.
    ReplayFrame& stride(std::uint64_t bytes) noexcept {
        stride_ = bytes;
        return *this;
    }

    ::madopilot_replay_frame_t to_c() const noexcept {
        auto value = detail::sized<::madopilot_replay_frame_t>();
        value.width = width_;
        value.height = height_;
        value.format = format_;
        value.continuity = continuity_;
        value.pixels = detail::as_bytes(pixels_, pixels_len_);
        value.captured_at_nanos = captured_at_;
        value.stride = stride_;
        return value;
    }

private:
    std::uint32_t width_ = 0;
    std::uint32_t height_ = 0;
    PixelFormat format_ = MADOPILOT_PIXEL_FORMAT_RGBA8;
    Continuity continuity_ = MADOPILOT_CONTINUITY_CONTINUOUS;
    const std::uint8_t* pixels_ = nullptr;
    std::size_t pixels_len_ = 0;
    std::uint64_t captured_at_ = 0;
    std::uint64_t stride_ = 0;
};

/// Where an engine's frames come from.
class Source {
public:
    /// The installed Windows desktop capture and input adapter.
    static Source native_windows() noexcept {
        Source source;
        source.kind_ = MADOPILOT_SOURCE_NATIVE_WINDOWS;
        return source;
    }

    /// The installed macOS desktop capture and input adapter.
    static Source native_macos() noexcept {
        Source source;
        source.kind_ = MADOPILOT_SOURCE_NATIVE_MACOS;
        return source;
    }

    /// Frames supplied from memory, under a target name.
    static Source replay_memory(std::string_view target_name) {
        Source source;
        source.kind_ = MADOPILOT_SOURCE_REPLAY_MEMORY;
        source.target_name_ = std::string(target_name);
        return source;
    }

    /// A tracked replay directory. An empty target name takes the default.
    static Source replay_directory(std::string_view directory,
                                   std::string_view target_name = {}) {
        Source source;
        source.kind_ = MADOPILOT_SOURCE_REPLAY_DIRECTORY;
        source.directory_ = std::string(directory);
        source.target_name_ = std::string(target_name);
        return source;
    }

    /// Appends one frame to a memory source.
    Source& frame(const ReplayFrame& frame) {
        frames_.push_back(frame.to_c());
        return *this;
    }

    ::madopilot_source_t to_c() const noexcept {
        auto value = detail::sized<::madopilot_source_t>();
        value.kind = kind_;
        value.directory = detail::as_str(directory_);
        value.frames = frames_.empty() ? nullptr : frames_.data();
        value.frame_count = frames_.size();
        // The stride between elements of an array this header declared. A caller
        // built against an older header has smaller elements, and the library
        // cannot guess the spacing of an array it did not declare.
        value.frame_stride = sizeof(::madopilot_replay_frame_t);
        value.target_name = detail::as_str(target_name_);
        return value;
    }

private:
    SourceKind kind_ = MADOPILOT_SOURCE_REPLAY_MEMORY;
    std::string directory_;
    std::string target_name_;
    std::vector<::madopilot_replay_frame_t> frames_;
};

/// Where an asset package is read from.
class PackageSource {
public:
    static PackageSource directory(std::string_view path) {
        PackageSource source;
        source.kind_ = MADOPILOT_PACKAGE_SOURCE_DIRECTORY;
        source.path_ = std::string(path);
        return source;
    }

    static PackageSource archive_file(std::string_view path) {
        PackageSource source;
        source.kind_ = MADOPILOT_PACKAGE_SOURCE_ARCHIVE_FILE;
        source.path_ = std::string(path);
        return source;
    }

    /// Archive bytes borrowed until the load call returns.
    static PackageSource archive_bytes(const std::uint8_t* data, std::size_t len) {
        PackageSource source;
        source.kind_ = MADOPILOT_PACKAGE_SOURCE_ARCHIVE_BYTES;
        source.archive_ = data;
        source.archive_len_ = len;
        return source;
    }

    ::madopilot_package_source_t to_c() const noexcept {
        auto value = detail::sized<::madopilot_package_source_t>();
        value.kind = kind_;
        value.path = detail::as_str(path_);
        value.archive = detail::as_bytes(archive_, archive_len_);
        return value;
    }

private:
    PackageSourceKind kind_ = MADOPILOT_PACKAGE_SOURCE_DIRECTORY;
    std::string path_;
    const std::uint8_t* archive_ = nullptr;
    std::size_t archive_len_ = 0;
};

/// Input capability requested while opening a capture session.
class InputOpenRequest {
public:
    InputOpenRequest() noexcept = default;

    InputOpenRequest& requirement(InputRequirement requirement) noexcept {
        requirement_ = requirement;
        return *this;
    }

    /// Replaces the mask of operation/delivery pairs that must be accepted.
    InputOpenRequest& require_pairs(std::uint64_t pairs) noexcept {
        required_pairs_ = pairs;
        return *this;
    }

    /// Replaces the additional pair mask the caller would prefer.
    InputOpenRequest& prefer_pairs(std::uint64_t pairs) noexcept {
        preferred_pairs_ = pairs;
        return *this;
    }

    ::madopilot_input_open_request_t to_c() const noexcept {
        auto value = detail::sized<::madopilot_input_open_request_t>();
        value.requirement = requirement_;
        value.required_pairs = required_pairs_;
        value.preferred_pairs = preferred_pairs_;
        return value;
    }

private:
    InputRequirement requirement_ = MADOPILOT_INPUT_OPTIONAL;
    std::uint64_t required_pairs_ = 0;
    std::uint64_t preferred_pairs_ = 0;
};

/// One typed event in a bounded input sequence.
///
/// Factories expose only one active event variant. Text is copied into the
/// value, so the view produced by `to_c()` remains valid while this event does.
class InputEvent {
public:
    static constexpr std::uint32_t max_text_chars =
        MADOPILOT_INPUT_MAX_TEXT_CHARS;
    static constexpr std::size_t max_text_utf8_bytes =
        MADOPILOT_INPUT_MAX_TEXT_UTF8_BYTES;
    static constexpr std::uint64_t max_delay_nanos =
        MADOPILOT_INPUT_MAX_DELAY_NANOS;
    static constexpr std::int32_t max_scroll_notches =
        MADOPILOT_INPUT_MAX_SCROLL_NOTCHES;
    static constexpr std::uint32_t min_function_key =
        MADOPILOT_INPUT_MIN_FUNCTION_KEY;
    static constexpr std::uint32_t max_function_key =
        MADOPILOT_INPUT_MAX_FUNCTION_KEY;

    static InputEvent pointer_move(Space space, double x, double y) noexcept {
        InputEvent event;
        event.kind_ = MADOPILOT_INPUT_EVENT_POINTER_MOVE;
        event.space_ = space;
        event.x_ = x;
        event.y_ = y;
        return event;
    }

    static InputEvent pointer_press(PointerButton button) noexcept {
        return pointer_button(MADOPILOT_INPUT_EVENT_POINTER_PRESS, button);
    }

    static InputEvent pointer_release(PointerButton button) noexcept {
        return pointer_button(MADOPILOT_INPUT_EVENT_POINTER_RELEASE, button);
    }

    static InputEvent pointer_scroll(std::int32_t horizontal,
                                     std::int32_t vertical) noexcept {
        InputEvent event;
        event.kind_ = MADOPILOT_INPUT_EVENT_POINTER_SCROLL;
        event.horizontal_ = horizontal;
        event.vertical_ = vertical;
        return event;
    }

    static InputEvent key_press(Key key, std::uint32_t value = 0) noexcept {
        return key_event(MADOPILOT_INPUT_EVENT_KEY_PRESS, key, value);
    }

    static InputEvent key_release(Key key, std::uint32_t value = 0) noexcept {
        return key_event(MADOPILOT_INPUT_EVENT_KEY_RELEASE, key, value);
    }

    static InputEvent text(std::string_view text) {
        InputEvent event;
        event.kind_ = MADOPILOT_INPUT_EVENT_TEXT;
        event.text_ = std::string(text);
        return event;
    }

    static InputEvent delay(std::uint64_t nanos) noexcept {
        InputEvent event;
        event.kind_ = MADOPILOT_INPUT_EVENT_DELAY;
        event.delay_nanos_ = nanos;
        return event;
    }

    ::madopilot_input_event_t to_c() const noexcept {
        auto value = detail::sized<::madopilot_input_event_t>();
        value.kind = kind_;
        value.space = space_;
        value.button = button_;
        value.key = key_;
        value.key_value = key_value_;
        value.x = x_;
        value.y = y_;
        value.horizontal = horizontal_;
        value.vertical = vertical_;
        value.text = detail::as_str(text_);
        value.delay_nanos = delay_nanos_;
        return value;
    }

private:
    InputEvent() noexcept = default;

    static InputEvent pointer_button(InputEventKind kind,
                                     PointerButton button) noexcept {
        InputEvent event;
        event.kind_ = kind;
        event.button_ = button;
        return event;
    }

    static InputEvent key_event(InputEventKind kind, Key key,
                                std::uint32_t value) noexcept {
        InputEvent event;
        event.kind_ = kind;
        event.key_ = key;
        event.key_value_ = value;
        return event;
    }

    InputEventKind kind_ = MADOPILOT_INPUT_EVENT_UNKNOWN;
    Space space_ = MADOPILOT_SPACE_CAPTURE_PIXELS;
    PointerButton button_ = MADOPILOT_POINTER_BUTTON_UNKNOWN;
    Key key_ = MADOPILOT_KEY_UNKNOWN;
    std::uint32_t key_value_ = 0;
    double x_ = 0.0;
    double y_ = 0.0;
    std::int32_t horizontal_ = 0;
    std::int32_t vertical_ = 0;
    std::string text_;
    std::uint64_t delay_nanos_ = 0;
};

/// How to open a session. Without either format the adapter's own layout is taken.
class OpenRequest {
public:
    OpenRequest() noexcept = default;

    OpenRequest& require_format(PixelFormat format) noexcept {
        flags_ |= MADOPILOT_OPEN_HAS_REQUIRED_FORMAT;
        required_ = format;
        return *this;
    }

    OpenRequest& prefer_format(PixelFormat format) noexcept {
        flags_ |= MADOPILOT_OPEN_HAS_PREFERRED_FORMAT;
        preferred_ = format;
        return *this;
    }

    /// Requests input together with capture. The policy is copied.
    OpenRequest& input(const InputOpenRequest& request) noexcept {
        input_ = request.to_c();
        return *this;
    }

    OpenRequest& no_input() noexcept {
        input_.reset();
        return *this;
    }

    ::madopilot_open_request_t to_c() const noexcept {
        auto value = detail::sized<::madopilot_open_request_t>();
        value.flags = flags_;
        value.required_format = required_;
        value.preferred_format = preferred_;
        return value;
    }

private:
    friend class Engine;
    std::uint32_t flags_ = 0;
    PixelFormat required_ = MADOPILOT_PIXEL_FORMAT_RGBA8;
    PixelFormat preferred_ = MADOPILOT_PIXEL_FORMAT_RGBA8;
    std::optional<::madopilot_input_open_request_t> input_;
};

/// How to map a frame. Without a region the whole frame is mapped.
class MapRequest {
public:
    MapRequest() noexcept = default;

    MapRequest& format(PixelFormat format) noexcept {
        format_ = format;
        return *this;
    }

    /// Maps a sub-rectangle. The rectangle names the space it is measured in.
    MapRequest& region(Rect region) noexcept {
        flags_ |= MADOPILOT_MAP_HAS_REGION;
        region_ = region;
        return *this;
    }

    MapRequest& clip_policy(ClipPolicy policy) noexcept {
        clip_ = policy;
        return *this;
    }

    ::madopilot_map_request_t to_c() const noexcept {
        auto value = detail::sized<::madopilot_map_request_t>();
        value.flags = flags_;
        value.format = format_;
        value.clip_policy = clip_;
        value.region = region_;
        return value;
    }

private:
    std::uint32_t flags_ = 0;
    PixelFormat format_ = MADOPILOT_PIXEL_FORMAT_RGBA8;
    ClipPolicy clip_ = MADOPILOT_CLIP_POLICY_REJECT;
    Rect region_{MADOPILOT_SPACE_CAPTURE_PIXELS, 0, 0, 0, 0};
};

/// Thresholds for one search. Every omitted field takes the template's default.
class MatchOptions {
public:
    MatchOptions() noexcept = default;

    MatchOptions& min_score(double score) noexcept {
        flags_ |= MADOPILOT_MATCH_HAS_MIN_SCORE;
        min_score_ = score;
        return *this;
    }

    MatchOptions& max_results(std::uint32_t limit) noexcept {
        flags_ |= MADOPILOT_MATCH_HAS_MAX_RESULTS;
        max_results_ = limit;
        return *this;
    }

    MatchOptions& suppression(Suppression suppression) noexcept {
        flags_ |= MADOPILOT_MATCH_HAS_SUPPRESSION;
        suppression_ = suppression;
        return *this;
    }

    ::madopilot_match_options_t to_c() const noexcept {
        auto value = detail::sized<::madopilot_match_options_t>();
        value.flags = flags_;
        value.min_score = min_score_;
        value.max_results = max_results_;
        value.suppression = suppression_;
        return value;
    }

private:
    std::uint32_t flags_ = 0;
    double min_score_ = 0.0;
    std::uint32_t max_results_ = 0;
    Suppression suppression_ = MADOPILOT_SUPPRESSION_DROP_OVERLAPPING;
};

/* ---------------------------------------------------------------------------
 * Projections of the C output structures that carry borrowed views
 * ------------------------------------------------------------------------ */

/// Capabilities that apply to the whole configured engine.
struct EngineCapabilities {
    std::uint32_t flags = 0;

    bool delivers_input() const noexcept {
        return (flags & MADOPILOT_ENGINE_DELIVERS_INPUT) != 0u;
    }

    bool reads_permissions() const noexcept {
        return (flags & MADOPILOT_ENGINE_READS_PERMISSIONS) != 0u;
    }
};

/// Redacted permission diagnostic. Both views borrow from the `Engine`.
struct PermissionDiagnostic {
    DiagnosticCategory category = MADOPILOT_DIAGNOSTIC_UNSPECIFIED;
    std::optional<std::int64_t> platform_code;
    BorrowedStr platform_namespace;
    BorrowedStr context;
};

/// The result of one non-prompting authorization probe.
struct Permission {
    PermissionKind kind = MADOPILOT_PERMISSION_KIND_UNSPECIFIED;
    PermissionState state = MADOPILOT_PERMISSION_STATE_UNKNOWN;
    std::optional<PermissionDiagnostic> diagnostic;
};

/// Capability data for one explicit operation and delivery route.
struct InputCapability {
    std::uint64_t target = 0;
    InputOperationKind operation = MADOPILOT_INPUT_OPERATION_UNKNOWN;
    InputDelivery delivery = MADOPILOT_INPUT_DELIVERY_NONE;
    CapabilitySupport support = MADOPILOT_CAPABILITY_UNKNOWN;
    InputAddressScope address_scope = MADOPILOT_INPUT_ADDRESS_NONE;
    std::optional<PermissionKind> permission;
    std::optional<SubmissionEvidence> evidence;
    bool focus_required = false;
    std::uint32_t pointer_spaces = 0;
};

/// What input an engine or an open session knows and accepts.
struct InputDescriptor {
    std::uint64_t target = 0;
    std::uint64_t known_pairs = 0;
    std::uint64_t supported_pairs = 0;
    std::uint64_t unknown_pairs = 0;
    std::uint32_t pointer_spaces = 0;
    std::uint32_t max_events = 0;
};

namespace detail {

inline InputCapability project_input_capability(
    const ::madopilot_input_capability_t& value) noexcept {
    InputCapability out;
    out.target = value.target;
    out.operation = value.operation;
    out.delivery = value.delivery;
    out.support = value.support;
    out.address_scope = value.address_scope;
    if ((value.flags & MADOPILOT_INPUT_CAPABILITY_HAS_PERMISSION) != 0u) {
        out.permission = value.permission;
    }
    if ((value.flags & MADOPILOT_INPUT_CAPABILITY_HAS_EVIDENCE) != 0u) {
        out.evidence = value.evidence;
    }
    out.focus_required = value.focus_required != 0;
    out.pointer_spaces = value.pointer_spaces;
    return out;
}

inline InputDescriptor project_input_descriptor(
    const ::madopilot_input_descriptor_t& value) noexcept {
    return InputDescriptor{
        value.target,
        value.known_pairs,
        value.supported_pairs,
        value.unknown_pairs,
        value.pointer_spaces,
        value.max_events,
    };
}

} // namespace detail

/// Fixed terminal facts retained by an owned input receipt.
struct InputReceiptInfo {
    std::uint64_t target = 0;
    SequenceOutcome outcome = MADOPILOT_SEQUENCE_UNEXECUTED;
    std::optional<InputDelivery> selected_route;
    InputAddressScope address_scope = MADOPILOT_INPUT_ADDRESS_NONE;
    std::uint64_t attempt_count = 0;
    std::uint64_t submitted = 0;
    std::optional<std::uint64_t> last_submitted;
    std::optional<SubmissionEvidence> evidence;
    std::optional<InputFault> fault;
    CleanupState cleanup = MADOPILOT_CLEANUP_NOT_NEEDED;
    std::uint64_t cleanup_released = 0;
    std::uint64_t cleanup_owed = 0;
    bool partial_native_effect = false;
    bool used_fallback = false;

    /// Only the two known safe values prove that no owned state remains held.
    constexpr bool may_leave_state_held() const noexcept {
        return cleanup != MADOPILOT_CLEANUP_NOT_NEEDED &&
               cleanup != MADOPILOT_CLEANUP_COMPLETE;
    }
};

/// One immutable route attempt projected from an input receipt.
struct InputAttempt {
    InputDelivery route = MADOPILOT_INPUT_DELIVERY_NONE;
    InputAddressScope address_scope = MADOPILOT_INPUT_ADDRESS_NONE;
    SequenceOutcome outcome = MADOPILOT_SEQUENCE_UNEXECUTED;
    std::uint64_t submitted = 0;
    std::optional<std::uint64_t> last_submitted;
    std::optional<SubmissionEvidence> evidence;
    std::optional<InputFault> fault;
    bool partial_native_effect = false;
};

namespace detail {

inline InputReceiptInfo project_input_receipt_info(
    const ::madopilot_input_receipt_info_t& value) noexcept {
    InputReceiptInfo out;
    out.target = value.target;
    out.outcome = value.outcome;
    if ((value.flags & MADOPILOT_INPUT_RECEIPT_HAS_SELECTED_ROUTE) != 0u) {
        out.selected_route = value.selected_route;
    }
    out.address_scope = value.address_scope;
    out.attempt_count = value.attempt_count;
    out.submitted = value.submitted;
    if ((value.flags & MADOPILOT_INPUT_RECEIPT_HAS_LAST_SUBMITTED) != 0u) {
        out.last_submitted = value.last_submitted;
    }
    if ((value.flags & MADOPILOT_INPUT_RECEIPT_HAS_EVIDENCE) != 0u) {
        out.evidence = value.evidence;
    }
    if ((value.flags & MADOPILOT_INPUT_RECEIPT_HAS_FAULT) != 0u) {
        out.fault = value.fault;
    }
    out.cleanup = value.cleanup;
    out.cleanup_released = value.cleanup_released;
    out.cleanup_owed = value.cleanup_owed;
    out.partial_native_effect =
        (value.flags & MADOPILOT_INPUT_RECEIPT_PARTIAL_NATIVE_EFFECT) != 0u;
    out.used_fallback =
        (value.flags & MADOPILOT_INPUT_RECEIPT_USED_FALLBACK) != 0u;
    return out;
}

inline InputAttempt project_input_attempt(
    const ::madopilot_input_attempt_t& value) noexcept {
    InputAttempt out;
    out.route = value.route;
    out.address_scope = value.address_scope;
    out.outcome = value.outcome;
    out.submitted = value.submitted;
    if ((value.flags & MADOPILOT_INPUT_ATTEMPT_HAS_LAST_SUBMITTED) != 0u) {
        out.last_submitted = value.last_submitted;
    }
    if ((value.flags & MADOPILOT_INPUT_ATTEMPT_HAS_EVIDENCE) != 0u) {
        out.evidence = value.evidence;
    }
    if ((value.flags & MADOPILOT_INPUT_ATTEMPT_HAS_FAULT) != 0u) {
        out.fault = value.fault;
    }
    out.partial_native_effect =
        (value.flags & MADOPILOT_INPUT_ATTEMPT_PARTIAL_NATIVE_EFFECT) != 0u;
    return out;
}

} // namespace detail

/// Counts retained by one immutable diagnostic batch.
struct DiagnosticBatchInfo {
    std::uint64_t record_count = 0;
    std::uint64_t discarded_normal = 0;
    std::uint64_t discarded_debug = 0;

    bool loss_only() const noexcept {
        return record_count == 0 &&
               (discarded_normal != 0 || discarded_debug != 0);
    }
};

/// One fixed-width, privacy-reviewed diagnostic record.
///
/// Presence-sensitive values retain the C record's `flags`; no payload strings
/// or captured bytes exist in this surface.
struct DiagnosticRecord {
    std::uint32_t flags = 0;
    std::uint64_t sequence = 0;
    std::uint64_t timestamp_nanos = 0;
    std::uint64_t operation_id = 0;
    std::uint64_t activity_tag = 0;
    DiagnosticLevel level = MADOPILOT_DIAGNOSTIC_LEVEL_NORMAL;
    DiagnosticKind kind = MADOPILOT_DIAGNOSTIC_KIND_OPERATION_STARTED;
    DiagnosticOperationKind operation = MADOPILOT_DIAGNOSTIC_OPERATION_DISCOVERY;
    Status status = MADOPILOT_STATUS_OK;
    std::uint64_t target = 0;
    FrameStamp frame{};
    std::uint64_t template_identity = 0;
    Space source_space = MADOPILOT_SPACE_CAPTURE_PIXELS;
    Space destination_space = MADOPILOT_SPACE_CAPTURE_PIXELS;
    Rect region{MADOPILOT_SPACE_CAPTURE_PIXELS, 0, 0, 0, 0};
    InputDelivery route = MADOPILOT_INPUT_DELIVERY_NONE;
    InputAddressScope address_scope = MADOPILOT_INPUT_ADDRESS_NONE;
    SubmissionEvidence evidence = MADOPILOT_SUBMISSION_EVIDENCE_NONE;
    InputFault input_fault = MADOPILOT_INPUT_FAULT_NONE;
    SequenceOutcome input_outcome = MADOPILOT_SEQUENCE_UNEXECUTED;
    CleanupState cleanup = MADOPILOT_CLEANUP_NOT_NEEDED;
    PermissionKind permission_kind = MADOPILOT_PERMISSION_KIND_UNSPECIFIED;
    PermissionState permission_state = MADOPILOT_PERMISSION_STATE_UNKNOWN;
    Lifecycle lifecycle = MADOPILOT_LIFECYCLE_OPEN;
    SearchDiagnosticOutcome search_outcome =
        MADOPILOT_SEARCH_DIAGNOSTIC_NO_MATCH;
    std::uint32_t input_operations = 0;
    InputRevalidationCategory input_revalidation = 0;
    InputGeometryResult input_geometry = 0;
    std::uint64_t input_event_index = 0;
    std::optional<std::uint64_t> candidate_count;
    bool partial_native_effect = false;
    bool used_fallback = false;
    std::uint64_t requested = 0;
    std::uint64_t submitted = 0;
    std::uint64_t result_count = 0;
    std::uint64_t cleanup_released = 0;
    std::uint64_t cleanup_owed = 0;

    bool has(std::uint32_t presence_flag) const noexcept {
        return (flags & presence_flag) != 0u;
    }
};

/// What the loaded library is. Both views are static and valid while it is loaded.
struct BuildInfo {
    std::uint32_t abi_major = 0;
    std::uint32_t abi_minor = 0;
    /// `sizeof` the library's own function table.
    std::uint32_t table_size = 0;
    BorrowedStr library_version;
    BorrowedStr required_backend;
};

/// One discovered capture target. Both views borrow from the `TargetList`.
struct TargetDescriptor {
    std::uint32_t flags = 0;
    std::uint32_t width = 0;
    std::uint32_t height = 0;
    PixelFormat format = MADOPILOT_PIXEL_FORMAT_RGBA8;
    /// A bit set: bit `1 << space` is set when that space converts.
    std::int32_t coordinate_spaces = 0;
    BorrowedStr name;
    BorrowedStr provider;
    /// Engine-local target identity.
    std::uint64_t target = 0;
    TargetKind kind = MADOPILOT_TARGET_KIND_UNKNOWN;
    CapabilitySupport capture = MADOPILOT_CAPABILITY_UNKNOWN;
    PermissionKind capture_permission = MADOPILOT_PERMISSION_KIND_UNSPECIFIED;

    bool supports_placement() const noexcept {
        return (flags & MADOPILOT_TARGET_SUPPORTS_PLACEMENT) != 0u;
    }


    bool has_kind() const noexcept {
        return (flags & MADOPILOT_TARGET_HAS_KIND) != 0u;
    }

    bool has_capture_permission() const noexcept {
        return (flags & MADOPILOT_TARGET_HAS_CAPTURE_PERMISSION) != 0u;
    }
    /// Whether `space` converts for this target.
    ///
    /// `coordinate_spaces` is a bit set in a signed 32-bit field, so the shift
    /// below is defined for `space` in [0, 31): shifting a `1` of type `int` by
    /// 31 reaches the sign bit and is undefined behaviour, and a negative shift
    /// count is too. A space outside that range is not a space this bit set can
    /// describe, so it does not convert. The C ABI allocates space codes from
    /// zero upward and ABI 1.2 currently uses five of them, so the bound is far
    /// from anything the library can report; it is here because the value arrives
    /// from a caller.
    bool supports_space(Space space) const noexcept {
        if (space < 0 || space >= 31) {
            return false;
        }
        return (coordinate_spaces & (1 << space)) != 0;
    }
};

/// A completed mapping. The bytes borrow from the `Mapping`.
struct Image {
    std::uint32_t flags = 0;
    std::uint32_t width = 0;
    std::uint32_t height = 0;
    PixelFormat format = MADOPILOT_PIXEL_FORMAT_RGBA8;
    Space space = MADOPILOT_SPACE_CAPTURE_PIXELS;
    std::uint64_t stride = 0;
    BorrowedBytes bytes;
    Rect region{MADOPILOT_SPACE_CAPTURE_PIXELS, 0, 0, 0, 0};

    /// True when the bytes are shared with the frame rather than copied out.
    bool shared() const noexcept { return (flags & MADOPILOT_IMAGE_SHARED) != 0u; }
};

/// What a loaded package declares. Every view borrows from the `Package`.
struct PackageInfo {
    std::uint64_t template_count = 0;
    BorrowedStr package_id;
    BorrowedStr package_version;
    BorrowedStr license;
};

/// What a prepared template is. Both views borrow from the `Template`.
struct TemplateInfo {
    std::uint32_t width = 0;
    std::uint32_t height = 0;
    double min_score = 0.0;
    BorrowedStr id;
    BorrowedStr backend;
    std::uint32_t max_results = 0;
    Space space = MADOPILOT_SPACE_CAPTURE_PIXELS;
};

/// What one completed search produced. Both views borrow from the `MatchResult`.
///
/// A `match_count` of zero is a successful answer, not a failure.
struct ResultInfo {
    std::uint64_t match_count = 0;
    BorrowedStr backend_id;
    BorrowedStr backend_version;
    Rect searched{MADOPILOT_SPACE_CAPTURE_PIXELS, 0, 0, 0, 0};
};

/// One match. `template_id` borrows from the `MatchResult`.
struct Match {
    double score = 0.0;
    BorrowedStr template_id;
    /// Coordinate-qualified: the rectangle names the space it is measured in.
    Rect bounds{MADOPILOT_SPACE_CAPTURE_PIXELS, 0, 0, 0, 0};
};

/* ---------------------------------------------------------------------------
 * Owners
 * ------------------------------------------------------------------------ */

/// A cancellation token. Cancelling it ends every operation carrying it.
class Cancellation : public detail::Owner<Cancellation, ::madopilot_cancellation_t> {
public:
    Cancellation() noexcept = default;

    Result<void> cancel() const {
        if (api_ == nullptr) {
            return detail::no_table<void>();
        }
        const Status status = api_->cancellation_cancel(handle_);
        return is_ok(status) ? Result<void>::success()
                             : Result<void>::failure(Error::from_status(status));
    }

    Result<bool> is_cancelled() const {
        if (api_ == nullptr) {
            return detail::no_table<bool>();
        }
        std::int32_t cancelled = 0;
        const Status status = api_->cancellation_is_cancelled(handle_, &cancelled);
        return is_ok(status) ? Result<bool>::success(cancelled != 0)
                             : Result<bool>::failure(Error::from_status(status));
    }

private:
    friend class Api;
    friend class detail::Owner<Cancellation, ::madopilot_cancellation_t>;

    Cancellation(const ::madopilot_api_t* api, std::size_t extent,
                 ::madopilot_cancellation_t* handle) noexcept
        : Owner(api, extent, handle) {}

    static Status retain_handle(const ::madopilot_api_t* api,
                                ::madopilot_cancellation_t* handle) noexcept {
        return api->cancellation_retain(handle);
    }

    static void release_handle(const ::madopilot_api_t* api,
                               ::madopilot_cancellation_t* handle) noexcept {
        api->cancellation_release(handle);
    }
};

inline Operation& Operation::cancellation(const Cancellation& token) noexcept {
    cancellation_ = token.get();
    return *this;
}

/// An immutable list of discovered targets. Every string it hands out borrows
/// from it.
class TargetList : public detail::Owner<TargetList, ::madopilot_target_list_t> {
public:
    TargetList() noexcept = default;

    Result<std::size_t> count() const {
        if (api_ == nullptr) {
            return detail::no_table<std::size_t>();
        }
        std::size_t value = 0;
        const Status status = api_->target_list_count(handle_, &value);
        return is_ok(status) ? Result<std::size_t>::success(value)
                             : Result<std::size_t>::failure(Error::from_status(status));
    }

    /// One target. An index at or beyond the count is refused by the C boundary.
    ///
    /// The returned `name` and `provider` stay valid while this list is retained.
    ///
    /// Named `at` rather than `get` because `get()` is already the owner's own
    /// accessor for the handle it holds.
    ///
    /// Lvalue-only, like every accessor here that hands out a borrowed view.
    /// `engine.discover(op).take().at(0)` would release the list at the end of
    /// the full expression and leave `name` and `provider` pointing into freed
    /// memory; the deleted overload turns that into a compile error rather than
    /// a use-after-free. Name the owner, then ask it.
    Result<TargetDescriptor> at(std::size_t index) const& {
        if (api_ == nullptr) {
            return detail::no_table<TargetDescriptor>();
        }
        auto target = detail::sized<::madopilot_target_t>();
        const Status status = api_->target_list_get(handle_, index, &target);
        if (!is_ok(status)) {
            return Result<TargetDescriptor>::failure(Error::from_status(status));
        }

        TargetDescriptor out;
        out.flags = target.flags;
        out.width = target.width;
        out.height = target.height;
        out.format = target.format;
        out.coordinate_spaces = target.coordinate_spaces;
        out.name = BorrowedStr(target.name);
        out.provider = BorrowedStr(target.provider);
        out.target = target.target;
        out.kind = target.kind;
        out.capture = target.capture;
        out.capture_permission = target.capture_permission;

        return Result<TargetDescriptor>::success(out);
    }

    Result<TargetDescriptor> at(std::size_t index) const&& = delete;

    /// Capability data for one explicit operation/route pair.
    Result<InputCapability> input_capability(
        std::size_t index, InputOperationKind operation,
        InputDelivery delivery) const {
        if (api_ == nullptr) {
            return detail::no_table<InputCapability>();
        }
        if (!detail::has_entry(
                api_, extent_,
                MADOPILOT_API_SIZE_TARGET_LIST_INPUT_CAPABILITY)) {
            return detail::unsupported<InputCapability>();
        }

        auto value = detail::sized<::madopilot_input_capability_t>();
        const Status status = api_->target_list_input_capability(
            handle_, index, operation, delivery, &value);
        if (!is_ok(status)) {
            return Result<InputCapability>::failure(Error::from_status(status));
        }

        return Result<InputCapability>::success(
            detail::project_input_capability(value));
    }

private:
    friend class Engine;
    friend class detail::Owner<TargetList, ::madopilot_target_list_t>;

    TargetList(const ::madopilot_api_t* api, std::size_t extent,
               ::madopilot_target_list_t* handle) noexcept
        : Owner(api, extent, handle) {}

    static Status retain_handle(const ::madopilot_api_t* api,
                                ::madopilot_target_list_t* handle) noexcept {
        return api->target_list_retain(handle);
    }

    static void release_handle(const ::madopilot_api_t* api,
                               ::madopilot_target_list_t* handle) noexcept {
        api->target_list_release(handle);
    }
};

/// An immutable loaded asset package. Its strings borrow from it.
class Package : public detail::Owner<Package, ::madopilot_package_t> {
public:
    Package() noexcept = default;

    Result<PackageInfo> describe() const& {
        if (api_ == nullptr) {
            return detail::no_table<PackageInfo>();
        }
        auto info = detail::sized<::madopilot_package_info_t>();
        const Status status = api_->package_describe(handle_, &info);
        if (!is_ok(status)) {
            return Result<PackageInfo>::failure(Error::from_status(status));
        }

        PackageInfo out;
        out.template_count = info.template_count;
        out.package_id = BorrowedStr(info.package_id);
        out.package_version = BorrowedStr(info.package_version);
        out.license = BorrowedStr(info.license);

        return Result<PackageInfo>::success(out);
    }

    /// Deleted for a temporary owner: the views it hands out would dangle.
    Result<PackageInfo> describe() const&& = delete;

    /// One declared template identity, borrowed from this package.
    Result<BorrowedStr> template_id(std::size_t index) const& {
        if (api_ == nullptr) {
            return detail::no_table<BorrowedStr>();
        }
        ::madopilot_str_t id{nullptr, 0};
        const Status status = api_->package_template_id(handle_, index, &id);
        return is_ok(status) ? Result<BorrowedStr>::success(BorrowedStr(id))
                             : Result<BorrowedStr>::failure(Error::from_status(status));
    }

    /// Deleted for a temporary owner: the views it hands out would dangle.
    Result<BorrowedStr> template_id(std::size_t index) const&& = delete;

private:
    friend class Engine;
    friend class detail::Owner<Package, ::madopilot_package_t>;

    Package(const ::madopilot_api_t* api, std::size_t extent,
            ::madopilot_package_t* handle) noexcept
        : Owner(api, extent, handle) {}

    static Status retain_handle(const ::madopilot_api_t* api,
                                ::madopilot_package_t* handle) noexcept {
        return api->package_retain(handle);
    }

    static void release_handle(const ::madopilot_api_t* api,
                               ::madopilot_package_t* handle) noexcept {
        api->package_release(handle);
    }
};

/// An immutable prepared template. It outlives the package it was compiled from.
class Template : public detail::Owner<Template, ::madopilot_template_t> {
public:
    Template() noexcept = default;

    Result<TemplateInfo> describe() const& {
        if (api_ == nullptr) {
            return detail::no_table<TemplateInfo>();
        }
        auto info = detail::sized<::madopilot_template_info_t>();
        const Status status = api_->template_describe(handle_, &info);
        if (!is_ok(status)) {
            return Result<TemplateInfo>::failure(Error::from_status(status));
        }

        TemplateInfo out;
        out.width = info.width;
        out.height = info.height;
        out.min_score = info.min_score;
        out.id = BorrowedStr(info.id);
        out.backend = BorrowedStr(info.backend);
        out.max_results = info.max_results;
        out.space = info.space;

        return Result<TemplateInfo>::success(out);
    }

    /// Deleted for a temporary owner: the views it hands out would dangle.
    Result<TemplateInfo> describe() const&& = delete;

private:
    friend class Engine;
    friend class detail::Owner<Template, ::madopilot_template_t>;

    Template(const ::madopilot_api_t* api, std::size_t extent,
             ::madopilot_template_t* handle) noexcept
        : Owner(api, extent, handle) {}

    static Status retain_handle(const ::madopilot_api_t* api,
                                ::madopilot_template_t* handle) noexcept {
        return api->template_retain(handle);
    }

    static void release_handle(const ::madopilot_api_t* api,
                               ::madopilot_template_t* handle) noexcept {
        api->template_release(handle);
    }
};

/// A completed CPU mapping. Its bytes stay readable while it is retained, after
/// the frame, the session, and the engine are gone.
class Mapping : public detail::Owner<Mapping, ::madopilot_mapping_t> {
public:
    Mapping() noexcept = default;

    /// The descriptor and the borrowed bytes. The bytes are valid while this
    /// mapping is retained and become invalid at its final release.
    Result<Image> describe() const& {
        if (api_ == nullptr) {
            return detail::no_table<Image>();
        }
        auto image = detail::sized<::madopilot_image_t>();
        const Status status = api_->mapping_describe(handle_, &image);
        if (!is_ok(status)) {
            return Result<Image>::failure(Error::from_status(status));
        }

        Image out;
        out.flags = image.flags;
        out.width = image.width;
        out.height = image.height;
        out.format = image.format;
        out.space = image.space;
        out.stride = image.stride;
        out.bytes = BorrowedBytes(image.bytes);
        out.region = image.region;

        return Result<Image>::success(out);
    }

    /// Deleted for a temporary owner: the views it hands out would dangle.
    Result<Image> describe() const&& = delete;

    /// The complete identity of the frame this mapping came from.
    Result<FrameStamp> stamp() const {
        if (api_ == nullptr) {
            return detail::no_table<FrameStamp>();
        }
        auto value = detail::sized<FrameStamp>();
        const Status status = api_->mapping_stamp(handle_, &value);
        return is_ok(status) ? Result<FrameStamp>::success(value)
                             : Result<FrameStamp>::failure(Error::from_status(status));
    }

private:
    friend class Frame;
    friend class detail::Owner<Mapping, ::madopilot_mapping_t>;

    Mapping(const ::madopilot_api_t* api, std::size_t extent,
            ::madopilot_mapping_t* handle) noexcept
        : Owner(api, extent, handle) {}

    static Status retain_handle(const ::madopilot_api_t* api,
                                ::madopilot_mapping_t* handle) noexcept {
        return api->mapping_retain(handle);
    }

    static void release_handle(const ::madopilot_api_t* api,
                               ::madopilot_mapping_t* handle) noexcept {
        api->mapping_release(handle);
    }
};

/// One published immutable frame.
class Frame : public detail::Owner<Frame, ::madopilot_frame_t> {
public:
    Frame() noexcept = default;

    /// Stream, epoch, sequence, and geometry revision: the complete identity.
    Result<FrameStamp> stamp() const {
        if (api_ == nullptr) {
            return detail::no_table<FrameStamp>();
        }
        auto value = detail::sized<FrameStamp>();
        const Status status = api_->frame_stamp(handle_, &value);
        return is_ok(status) ? Result<FrameStamp>::success(value)
                             : Result<FrameStamp>::failure(Error::from_status(status));
    }

    Result<FrameInfo> describe() const {
        if (api_ == nullptr) {
            return detail::no_table<FrameInfo>();
        }
        auto value = detail::sized<FrameInfo>();
        const Status status = api_->frame_describe(handle_, &value);
        return is_ok(status) ? Result<FrameInfo>::success(value)
                             : Result<FrameInfo>::failure(Error::from_status(status));
    }

    /// Maps pixels out of this frame. The mapping outlives this frame.
    Result<Mapping> map(const MapRequest& request, const Operation& operation) const {
        if (api_ == nullptr) {
            return detail::no_table<Mapping>();
        }
        const auto request_c = request.to_c();
        const auto operation_c = operation.to_c();
        ::madopilot_mapping_t* mapping = nullptr;
        ::madopilot_error_t* error = nullptr;
        const Status status =
            api_->frame_map(handle_, &request_c, &operation_c, &mapping, &error);
        if (!is_ok(status)) {
            return Result<Mapping>::failure(detail::take_error(api_, status, error));
        }

        return Result<Mapping>::success(Mapping(api_, extent_, mapping));
    }

private:
    friend class Session;
    friend class detail::Owner<Frame, ::madopilot_frame_t>;

    Frame(const ::madopilot_api_t* api, std::size_t extent,
          ::madopilot_frame_t* handle) noexcept
        : Owner(api, extent, handle) {}

    static Status retain_handle(const ::madopilot_api_t* api,
                                ::madopilot_frame_t* handle) noexcept {
        return api->frame_retain(handle);
    }

    static void release_handle(const ::madopilot_api_t* api,
                               ::madopilot_frame_t* handle) noexcept {
        api->frame_release(handle);
    }
};

/// One bounded input sequence and the policies that govern its delivery.
///
/// Events and delivery order are copied into this value. A source frame is
/// borrowed and must stay retained until `Session::send_input` returns.
class InputRequest {
public:
    class CView;

    /// The ABI ceiling; an input descriptor may advertise a lower value.
    static constexpr std::size_t abi_max_events =
        MADOPILOT_INPUT_MAX_EVENTS;
    static constexpr std::uint32_t max_cleanup_events =
        MADOPILOT_INPUT_MAX_CLEANUP_EVENTS;
    static constexpr std::uint64_t max_cleanup_timeout_nanos =
        MADOPILOT_INPUT_MAX_CLEANUP_NANOS;

    InputRequest() noexcept = default;

    InputRequest& event(InputEvent event) {
        events_.push_back(std::move(event));
        return *this;
    }

    InputRequest& delivery(InputDelivery delivery) {
        deliveries_.push_back(delivery);
        return *this;
    }

    InputRequest& focus_policy(FocusPolicy policy) noexcept {
        focus_ = policy;
        return *this;
    }

    InputRequest& geometry_policy(GeometryPolicy policy) noexcept {
        geometry_ = policy;
        return *this;
    }

    InputRequest& source_frame(const Frame& frame) noexcept {
        source_frame_ = frame.get();
        return *this;
    }

    InputRequest& no_source_frame() noexcept {
        source_frame_ = nullptr;
        return *this;
    }

    InputRequest& cleanup_budget(std::uint32_t max_events,
                                 std::uint64_t timeout_nanos) noexcept {
        has_cleanup_budget_ = true;
        cleanup_max_events_ = max_events;
        cleanup_timeout_nanos_ = timeout_nanos;
        return *this;
    }

    InputRequest& no_cleanup_budget() noexcept {
        has_cleanup_budget_ = false;
        cleanup_max_events_ = 0;
        cleanup_timeout_nanos_ = 0;
        return *this;
    }

    [[nodiscard]] CView to_c() const;

private:
    std::vector<InputEvent> events_;
    std::vector<InputDelivery> deliveries_;
    FocusPolicy focus_ = MADOPILOT_FOCUS_PRESERVE;
    GeometryPolicy geometry_ = MADOPILOT_GEOMETRY_REPROJECT_CURRENT;
    const ::madopilot_frame_t* source_frame_ = nullptr;
    bool has_cleanup_budget_ = false;
    std::uint32_t cleanup_max_events_ = 0;
    std::uint64_t cleanup_timeout_nanos_ = 0;
};

/// A call-local C projection of an input request.
///
/// Every `to_c()` call owns a fresh event-record array, and moving a projection
/// rebinds its C pointer to the transferred array. Event text and delivery order
/// still borrow the `InputRequest`, which must outlive the complete C call.
class InputRequest::CView {
public:
    explicit CView(const InputRequest& request) {
        event_records_.reserve(request.events_.size());
        for (const InputEvent& event : request.events_) {
            event_records_.push_back(event.to_c());
        }

        value_ = detail::sized<::madopilot_input_request_t>();
        value_.flags = request.has_cleanup_budget_
                           ? MADOPILOT_INPUT_REQUEST_HAS_CLEANUP_BUDGET
                           : 0u;
        value_.events = event_records_.empty() ? nullptr : event_records_.data();
        value_.event_count = event_records_.size();
        value_.event_stride = sizeof(::madopilot_input_event_t);
        value_.deliveries =
            request.deliveries_.empty() ? nullptr : request.deliveries_.data();
        value_.delivery_count = request.deliveries_.size();
        value_.focus_policy = request.focus_;
        value_.geometry_policy = request.geometry_;
        value_.source_frame = request.source_frame_;
        value_.cleanup_max_events = request.cleanup_max_events_;
        value_.cleanup_timeout_nanos = request.cleanup_timeout_nanos_;
    }

    CView(const CView&) = delete;
    CView& operator=(const CView&) = delete;

    CView(CView&& other) noexcept
        : event_records_(std::move(other.event_records_)), value_(other.value_) {
        rebind_events();
        other.rebind_events();
    }

    CView& operator=(CView&& other) noexcept {
        if (this != &other) {
            event_records_ = std::move(other.event_records_);
            value_ = other.value_;
            rebind_events();
            other.rebind_events();
        }
        return *this;
    }

    const ::madopilot_input_request_t& value() const& noexcept { return value_; }
    const ::madopilot_input_request_t& value() const&& = delete;

    const ::madopilot_input_request_t* get() const& noexcept { return &value_; }
    const ::madopilot_input_request_t* get() const&& = delete;

private:
    void rebind_events() noexcept {
        value_.events = event_records_.empty() ? nullptr : event_records_.data();
        value_.event_count = event_records_.size();
    }

    std::vector<::madopilot_input_event_t> event_records_;
    ::madopilot_input_request_t value_{};
};

inline InputRequest::CView InputRequest::to_c() const { return CView(*this); }

/// One template search.
///
/// The word `template` cannot name a member in C++, so the template to search
/// for is supplied by `search_for`. The request borrows both handles: the
/// `Frame` and `Template` owners must outlive the call it is passed to.
class FindRequest {
public:
    FindRequest() noexcept = default;

    /// Searches this exact frame. Without it, the session's latest frame is
    /// searched — a different question as soon as a second frame is published.
    FindRequest& frame(const Frame& frame) noexcept {
        frame_ = frame.get();
        return *this;
    }

    FindRequest& latest_frame() noexcept {
        frame_ = nullptr;
        return *this;
    }

    /// The prepared template to look for. Required.
    FindRequest& search_for(const Template& prepared) noexcept {
        template_ = prepared.get();
        return *this;
    }

    /// Restricts the search to a sub-rectangle of the frame.
    FindRequest& region(Rect region) noexcept {
        flags_ |= MADOPILOT_FIND_HAS_REGION;
        region_ = region;
        return *this;
    }

    FindRequest& clip_policy(ClipPolicy policy) noexcept {
        clip_ = policy;
        return *this;
    }

    /// Overrides the prepared template's own defaults for this search.
    FindRequest& options(const MatchOptions& options) {
        options_ = options.to_c();
        return *this;
    }

    ::madopilot_find_request_t to_c() const noexcept {
        auto value = detail::sized<::madopilot_find_request_t>();
        value.flags = flags_;
        value.frame = frame_;
        value.tmpl = template_;
        value.options = options_.has_value() ? &*options_ : nullptr;
        value.region = region_;
        value.clip_policy = clip_;
        return value;
    }

private:
    std::uint32_t flags_ = 0;
    const ::madopilot_frame_t* frame_ = nullptr;
    const ::madopilot_template_t* template_ = nullptr;
    std::optional<::madopilot_match_options_t> options_;
    Rect region_{MADOPILOT_SPACE_CAPTURE_PIXELS, 0, 0, 0, 0};
    ClipPolicy clip_ = MADOPILOT_CLIP_POLICY_REJECT;
};

/// An immutable completed search.
///
/// It owns the exact frame it searched, so it stays correlated after the
/// session, the template, the package, and the engine are gone.
class MatchResult : public detail::Owner<MatchResult, ::madopilot_result_t> {
public:
    MatchResult() noexcept = default;

    /// Match count, backend identity, and the searched rectangle. The two
    /// backend views borrow from this result.
    Result<ResultInfo> describe() const& {
        if (api_ == nullptr) {
            return detail::no_table<ResultInfo>();
        }
        auto info = detail::sized<::madopilot_result_info_t>();
        const Status status = api_->result_describe(handle_, &info);
        if (!is_ok(status)) {
            return Result<ResultInfo>::failure(Error::from_status(status));
        }

        ResultInfo out;
        out.match_count = info.match_count;
        out.backend_id = BorrowedStr(info.backend_id);
        out.backend_version = BorrowedStr(info.backend_version);
        out.searched = info.searched;

        return Result<ResultInfo>::success(out);
    }

    /// Deleted for a temporary owner: the views it hands out would dangle.
    Result<ResultInfo> describe() const&& = delete;

    /// The complete identity of the frame that was searched.
    Result<FrameStamp> stamp() const {
        if (api_ == nullptr) {
            return detail::no_table<FrameStamp>();
        }
        auto value = detail::sized<FrameStamp>();
        const Status status = api_->result_stamp(handle_, &value);
        return is_ok(status) ? Result<FrameStamp>::success(value)
                             : Result<FrameStamp>::failure(Error::from_status(status));
    }

    /// The options the search actually ran under, not the ones requested.
    Result<EffectiveMatchOptions> options() const {
        if (api_ == nullptr) {
            return detail::no_table<EffectiveMatchOptions>();
        }
        auto value = detail::sized<EffectiveMatchOptions>();
        const Status status = api_->result_options(handle_, &value);
        return is_ok(status)
                   ? Result<EffectiveMatchOptions>::success(value)
                   : Result<EffectiveMatchOptions>::failure(Error::from_status(status));
    }

    /// One match. An index at or beyond the count is refused by the C boundary.
    ///
    /// The returned `template_id` stays valid while this result is retained.
    Result<Match> match_at(std::size_t index) const& {
        if (api_ == nullptr) {
            return detail::no_table<Match>();
        }
        auto match = detail::sized<::madopilot_match_t>();
        const Status status = api_->result_match(handle_, index, &match);
        if (!is_ok(status)) {
            return Result<Match>::failure(Error::from_status(status));
        }

        Match out;
        out.score = match.score;
        out.template_id = BorrowedStr(match.template_id);
        out.bounds = match.bounds;

        return Result<Match>::success(out);
    }

    /// Deleted for a temporary owner: the views it hands out would dangle.
    Result<Match> match_at(std::size_t index) const&& = delete;

    /// The highest-scoring match, or nothing.
    ///
    /// A search that qualified nothing is a successful answer to a well-formed
    /// question, so this succeeds with an empty optional rather than failing.
    Result<std::optional<Match>> first_match() const& {
        auto info = describe();
        if (!info) {
            return Result<std::optional<Match>>::failure(info.error());
        }
        if (info.value().match_count == 0) {
            return Result<std::optional<Match>>::success(std::optional<Match>{});
        }

        auto match = match_at(0);
        if (!match) {
            return Result<std::optional<Match>>::failure(match.error());
        }

        return Result<std::optional<Match>>::success(std::optional<Match>(match.take()));
    }

    /// Deleted for a temporary owner: the views it hands out would dangle.
    Result<std::optional<Match>> first_match() const&& = delete;

    /// Every match, in the order the backend's canonical ordering produced.
    ///
    /// Each `template_id` borrows from this result, exactly as `match_at` does.
    Result<std::vector<Match>> matches() const& {
        auto info = describe();
        if (!info) {
            return Result<std::vector<Match>>::failure(info.error());
        }

        std::vector<Match> out;
        out.reserve(static_cast<std::size_t>(info.value().match_count));
        for (std::uint64_t index = 0; index < info.value().match_count; ++index) {
            auto match = match_at(static_cast<std::size_t>(index));
            if (!match) {
                return Result<std::vector<Match>>::failure(match.error());
            }
            out.push_back(match.take());
        }

        return Result<std::vector<Match>>::success(std::move(out));
    }

    /// Deleted for a temporary owner: the views it hands out would dangle.
    Result<std::vector<Match>> matches() const&& = delete;

private:
    friend class Session;
    friend class detail::Owner<MatchResult, ::madopilot_result_t>;

    MatchResult(const ::madopilot_api_t* api, std::size_t extent,
                ::madopilot_result_t* handle) noexcept
        : Owner(api, extent, handle) {}

    static Status retain_handle(const ::madopilot_api_t* api,
                                ::madopilot_result_t* handle) noexcept {
        return api->result_retain(handle);
    }

    static void release_handle(const ::madopilot_api_t* api,
                               ::madopilot_result_t* handle) noexcept {
        api->result_release(handle);
    }
};

/// An immutable terminal account of one admitted input sequence.
class InputReceipt
    : public detail::Owner<InputReceipt, ::madopilot_input_receipt_t> {
public:
    InputReceipt() noexcept = default;

    Result<InputReceiptInfo> describe() const {
        if (api_ == nullptr) {
            return detail::no_table<InputReceiptInfo>();
        }
        auto value = detail::sized<::madopilot_input_receipt_info_t>();
        const Status status = api_->input_receipt_info(handle_, &value);
        if (!is_ok(status)) {
            return Result<InputReceiptInfo>::failure(Error::from_status(status));
        }

        return Result<InputReceiptInfo>::success(
            detail::project_input_receipt_info(value));
    }

    Result<std::size_t> attempt_count() const {
        if (api_ == nullptr) {
            return detail::no_table<std::size_t>();
        }
        std::size_t count = 0;
        const Status status =
            api_->input_receipt_attempt_count(handle_, &count);
        return is_ok(status)
                   ? Result<std::size_t>::success(count)
                   : Result<std::size_t>::failure(Error::from_status(status));
    }

    Result<InputAttempt> attempt_at(std::size_t index) const {
        if (api_ == nullptr) {
            return detail::no_table<InputAttempt>();
        }
        auto value = detail::sized<::madopilot_input_attempt_t>();
        const Status status =
            api_->input_receipt_attempt_at(handle_, index, &value);
        if (!is_ok(status)) {
            return Result<InputAttempt>::failure(Error::from_status(status));
        }

        return Result<InputAttempt>::success(
            detail::project_input_attempt(value));
    }

private:
    friend class Session;
    friend class detail::Owner<InputReceipt, ::madopilot_input_receipt_t>;

    InputReceipt(const ::madopilot_api_t* api, std::size_t extent,
                 ::madopilot_input_receipt_t* handle) noexcept
        : Owner(api, extent, handle) {}

    static Status retain_handle(const ::madopilot_api_t* api,
                                ::madopilot_input_receipt_t* handle) noexcept {
        return api->input_receipt_retain(handle);
    }

    static void release_handle(const ::madopilot_api_t* api,
                               ::madopilot_input_receipt_t* handle) noexcept {
        api->input_receipt_release(handle);
    }
};

/// One immutable batch of diagnostic records and exact pending-loss counts.
class DiagnosticBatch
    : public detail::Owner<DiagnosticBatch, ::madopilot_diagnostic_batch_t> {
public:
    DiagnosticBatch() noexcept = default;

    Result<DiagnosticBatchInfo> describe() const {
        if (api_ == nullptr) {
            return detail::no_table<DiagnosticBatchInfo>();
        }
        auto value = detail::sized<::madopilot_diagnostic_batch_info_t>();
        const Status status = api_->diagnostic_batch_info(handle_, &value);
        if (!is_ok(status)) {
            return Result<DiagnosticBatchInfo>::failure(Error::from_status(status));
        }
        return Result<DiagnosticBatchInfo>::success(DiagnosticBatchInfo{
            value.record_count,
            value.discarded_normal,
            value.discarded_debug,
        });
    }

    Result<DiagnosticRecord> record_at(std::size_t index) const {
        if (api_ == nullptr) {
            return detail::no_table<DiagnosticRecord>();
        }
        auto value = detail::sized<::madopilot_diagnostic_record_t>();
        const Status status =
            api_->diagnostic_batch_record_at(handle_, index, &value);
        if (!is_ok(status)) {
            return Result<DiagnosticRecord>::failure(Error::from_status(status));
        }

        DiagnosticRecord out;
        out.flags = value.flags;
        out.sequence = value.sequence;
        out.timestamp_nanos = value.timestamp_nanos;
        out.operation_id = value.operation_id;
        out.activity_tag = value.activity_tag;
        out.level = value.level;
        out.kind = value.kind;
        out.operation = value.operation;
        out.status = value.status;
        out.target = value.target;
        out.frame = value.frame;
        out.template_identity = value.template_identity;
        out.source_space = value.source_space;
        out.destination_space = value.destination_space;
        out.region = value.region;
        out.route = value.route;
        out.address_scope = value.address_scope;
        out.evidence = value.evidence;
        out.input_fault = value.input_fault;
        out.input_outcome = value.input_outcome;
        out.cleanup = value.cleanup;
        out.permission_kind = value.permission_kind;
        out.permission_state = value.permission_state;
        out.lifecycle = value.lifecycle;
        out.search_outcome = value.search_outcome;
        out.input_operations = value.input_operations;
        out.partial_native_effect = value.partial_native_effect != 0;
        out.used_fallback = value.used_fallback != 0;
        if (value.kind == MADOPILOT_DIAGNOSTIC_KIND_INPUT_EVENT) {
            out.input_event_index = value.cleanup_released;
        }
        if ((value.flags &
             MADOPILOT_DIAGNOSTIC_RECORD_HAS_INPUT_EVENT_DETAIL) != 0u) {
            out.input_revalidation =
                static_cast<InputRevalidationCategory>(
                    value.reserved &
                    MADOPILOT_DIAGNOSTIC_INPUT_EVENT_REVALIDATION_MASK);
            out.input_geometry = static_cast<InputGeometryResult>(
                (value.reserved &
                 MADOPILOT_DIAGNOSTIC_INPUT_EVENT_GEOMETRY_MASK) >>
                MADOPILOT_DIAGNOSTIC_INPUT_EVENT_GEOMETRY_SHIFT);
        }
        if ((value.flags &
             MADOPILOT_DIAGNOSTIC_RECORD_HAS_CANDIDATE_COUNT) != 0u) {
            out.candidate_count = value.result_count;
        }
        out.requested = value.requested;
        out.submitted = value.submitted;
        out.result_count = value.result_count;
        out.cleanup_released = value.cleanup_released;
        out.cleanup_owed = value.cleanup_owed;
        return Result<DiagnosticRecord>::success(out);
    }

private:
    friend class DiagnosticReader;
    friend class detail::Owner<DiagnosticBatch, ::madopilot_diagnostic_batch_t>;

    DiagnosticBatch(const ::madopilot_api_t* api, std::size_t extent,
                    ::madopilot_diagnostic_batch_t* handle) noexcept
        : Owner(api, extent, handle) {}

    static Status retain_handle(const ::madopilot_api_t* api,
                                ::madopilot_diagnostic_batch_t* handle) noexcept {
        return api->diagnostic_batch_retain(handle);
    }

    static void release_handle(const ::madopilot_api_t* api,
                               ::madopilot_diagnostic_batch_t* handle) noexcept {
        api->diagnostic_batch_release(handle);
    }
};

/// One non-blocking diagnostic drain result.
struct DiagnosticDrain {
    DiagnosticDrainState state = MADOPILOT_DIAGNOSTIC_DRAIN_OPEN_EMPTY;
    std::optional<DiagnosticBatch> batch;
};

/// The engine's single pull-based diagnostic consumer.
class DiagnosticReader
    : public detail::Owner<DiagnosticReader, ::madopilot_diagnostic_reader_t> {
public:
    DiagnosticReader() noexcept = default;

    Result<DiagnosticDrain> drain() const {
        if (api_ == nullptr) {
            return detail::no_table<DiagnosticDrain>();
        }
        DiagnosticDrainState state = MADOPILOT_DIAGNOSTIC_DRAIN_OPEN_EMPTY;
        ::madopilot_diagnostic_batch_t* batch = nullptr;
        const Status status =
            api_->diagnostic_reader_drain(handle_, &state, &batch);
        if (!is_ok(status)) {
            return Result<DiagnosticDrain>::failure(Error::from_status(status));
        }
        if ((state == MADOPILOT_DIAGNOSTIC_DRAIN_BATCH) != (batch != nullptr)) {
            if (batch != nullptr) {
                api_->diagnostic_batch_release(batch);
            }
            return Result<DiagnosticDrain>::failure(
                Error::from_status(MADOPILOT_STATUS_INTERNAL));
        }

        DiagnosticDrain out;
        out.state = state;
        if (batch != nullptr) {
            out.batch = DiagnosticBatch(api_, extent_, batch);
        }
        return Result<DiagnosticDrain>::success(std::move(out));
    }

private:
    friend class Engine;
    friend class detail::Owner<DiagnosticReader, ::madopilot_diagnostic_reader_t>;

    DiagnosticReader(const ::madopilot_api_t* api, std::size_t extent,
                     ::madopilot_diagnostic_reader_t* handle) noexcept
        : Owner(api, extent, handle) {}

    static Status retain_handle(
        const ::madopilot_api_t* api,
        ::madopilot_diagnostic_reader_t* handle) noexcept {
        return api->diagnostic_reader_retain(handle);
    }

    static void release_handle(
        const ::madopilot_api_t* api,
        ::madopilot_diagnostic_reader_t* handle) noexcept {
        api->diagnostic_reader_release(handle);
    }
};

/// An open capture session.
///
/// Destroying a session releases the reference but does not close it. Close is
/// explicit and status-returning because a destructor cannot report a failed
/// drain.
class Session : public detail::Owner<Session, ::madopilot_session_t> {
public:
    Session() noexcept = default;

    Result<SessionInfo> describe() const {
        if (api_ == nullptr) {
            return detail::no_table<SessionInfo>();
        }
        auto value = detail::sized<SessionInfo>();
        const Status status = api_->session_describe(handle_, &value);
        return is_ok(status) ? Result<SessionInfo>::success(value)
                             : Result<SessionInfo>::failure(Error::from_status(status));
    }

    /// Closes the session and reports the outcome.
    ///
    /// Idempotent: a later call observes the C ABI's own idempotent behaviour.
    /// A failure here is returned, never swallowed by destruction.
    Result<void> close(const Operation& operation) const {
        if (api_ == nullptr) {
            return detail::no_table<void>();
        }
        const auto operation_c = operation.to_c();
        ::madopilot_error_t* error = nullptr;
        const Status status = api_->session_close(handle_, &operation_c, &error);
        if (!is_ok(status)) {
            return Result<void>::failure(detail::take_error(api_, status, error));
        }

        return Result<void>::success();
    }

    Result<bool> is_closed() const {
        if (api_ == nullptr) {
            return detail::no_table<bool>();
        }
        std::int32_t closed = 0;
        const Status status = api_->session_is_closed(handle_, &closed);
        return is_ok(status) ? Result<bool>::success(closed != 0)
                             : Result<bool>::failure(Error::from_status(status));
    }

    /// The session's latest published frame.
    ///
    /// A verb, because this waits: the accessors on this type are nouns and
    /// none of them blocks.
    Result<Frame> acquire_frame(const Operation& operation) const {
        if (api_ == nullptr) {
            return detail::no_table<Frame>();
        }
        const auto operation_c = operation.to_c();
        ::madopilot_frame_t* frame = nullptr;
        ::madopilot_error_t* error = nullptr;
        const Status status = api_->session_acquire_frame(handle_, &operation_c, &frame, &error);
        if (!is_ok(status)) {
            return Result<Frame>::failure(detail::take_error(api_, status, error));
        }

        return Result<Frame>::success(Frame(api_, extent_, frame));
    }

    /// Searches a frame for one prepared template.
    ///
    /// A search that qualified nothing succeeds with a zero-match result,
    /// correlated with the searched frame exactly as a non-empty one is.
    Result<MatchResult> find(const FindRequest& request, const Operation& operation) const {
        if (api_ == nullptr) {
            return detail::no_table<MatchResult>();
        }
        const auto request_c = request.to_c();
        const auto operation_c = operation.to_c();
        ::madopilot_result_t* result = nullptr;
        ::madopilot_error_t* error = nullptr;
        const Status status =
            api_->session_find(handle_, &request_c, &operation_c, &result, &error);
        if (!is_ok(status)) {
            return Result<MatchResult>::failure(detail::take_error(api_, status, error));
        }

        return Result<MatchResult>::success(MatchResult(api_, extent_, result));
    }

    /// The immutable input capability accepted when this session opened.
    Result<InputDescriptor> input_descriptor() const {
        if (api_ == nullptr) {
            return detail::no_table<InputDescriptor>();
        }
        if (!detail::has_entry(api_, extent_,
                               MADOPILOT_API_SIZE_SESSION_INPUT_DESCRIPTOR)) {
            return detail::unsupported<InputDescriptor>();
        }

        auto value = detail::sized<::madopilot_input_descriptor_t>();
        const Status status = api_->session_input_descriptor(handle_, &value);
        if (!is_ok(status)) {
            return Result<InputDescriptor>::failure(Error::from_status(status));
        }

        return Result<InputDescriptor>::success(
            detail::project_input_descriptor(value));
    }

    /// Sends one sequence. Partial and unexecuted outcomes are successful
    /// receipts. Validation, an unavailable ABI entry, a pre-admission refusal,
    /// or a contained boundary failure produces a failed `Result`.
    Result<InputReceipt> send_input(const InputRequest& request,
                                    const Operation& operation) const {
        if (api_ == nullptr) {
            return detail::no_table<InputReceipt>();
        }
        if (!detail::has_entry(api_, extent_,
                               MADOPILOT_API_SIZE_SESSION_SEND_INPUT)) {
            return detail::unsupported<InputReceipt>();
        }

        const auto request_c = request.to_c();
        const auto operation_c = operation.to_c();
        ::madopilot_input_receipt_t* receipt = nullptr;
        ::madopilot_error_t* error = nullptr;
        const Status status = api_->session_send_input(
            handle_, request_c.get(), &operation_c, &receipt, &error);
        if (!is_ok(status)) {
            return Result<InputReceipt>::failure(
                detail::take_error(api_, status, error));
        }
        if (receipt == nullptr) {
            return Result<InputReceipt>::failure(
                Error::from_status(MADOPILOT_STATUS_INTERNAL));
        }
        return Result<InputReceipt>::success(
            InputReceipt(api_, extent_, receipt));
    }

private:
    friend class Engine;
    friend class detail::Owner<Session, ::madopilot_session_t>;

    Session(const ::madopilot_api_t* api, std::size_t extent,
            ::madopilot_session_t* handle) noexcept
        : Owner(api, extent, handle) {}

    static Status retain_handle(const ::madopilot_api_t* api,
                                ::madopilot_session_t* handle) noexcept {
        return api->session_retain(handle);
    }

    static void release_handle(const ::madopilot_api_t* api,
                               ::madopilot_session_t* handle) noexcept {
        api->session_release(handle);
    }
};

/// The root handle: a configured deterministic source and the backends it wires.
class Engine : public detail::Owner<Engine, ::madopilot_engine_t> {
public:
    Engine() noexcept = default;

    Result<EngineCapabilities> capabilities() const {
        if (api_ == nullptr) {
            return detail::no_table<EngineCapabilities>();
        }
        if (!detail::has_entry(api_, extent_,
                               MADOPILOT_API_SIZE_ENGINE_CAPABILITIES)) {
            return detail::unsupported<EngineCapabilities>();
        }

        auto value = detail::sized<::madopilot_engine_capabilities_t>();
        const Status status = api_->engine_capabilities(handle_, &value);
        return is_ok(status)
                   ? Result<EngineCapabilities>::success(
                         EngineCapabilities{value.flags})
                   : Result<EngineCapabilities>::failure(
                         Error::from_status(status));
    }

    /// Takes the engine's single diagnostic reader.
    ///
    /// Diagnostics-off engines and repeated takes return an empty optional.
    Result<std::optional<DiagnosticReader>> take_diagnostic_reader() const {
        if (api_ == nullptr) {
            return detail::no_table<std::optional<DiagnosticReader>>();
        }
        if (!detail::has_entry(
                api_, extent_,
                MADOPILOT_API_SIZE_ENGINE_TAKE_DIAGNOSTIC_READER)) {
            return detail::unsupported<std::optional<DiagnosticReader>>();
        }

        ::madopilot_diagnostic_reader_t* reader = nullptr;
        const Status status =
            api_->engine_take_diagnostic_reader(handle_, &reader);
        if (!is_ok(status)) {
            return Result<std::optional<DiagnosticReader>>::failure(
                Error::from_status(status));
        }

        std::optional<DiagnosticReader> out;
        if (reader != nullptr) {
            out = DiagnosticReader(api_, extent_, reader);
        }
        return Result<std::optional<DiagnosticReader>>::success(std::move(out));
    }

    /// Runs a non-prompting permission probe. Diagnostic views borrow from this
    /// engine, so the accessor is lvalue-only.
    Result<Permission> permission(PermissionKind kind,
                                  const Operation& operation) const& {
        if (api_ == nullptr) {
            return detail::no_table<Permission>();
        }
        if (!detail::has_entry(api_, extent_,
                               MADOPILOT_API_SIZE_ENGINE_PERMISSION)) {
            return detail::unsupported<Permission>();
        }

        const auto operation_c = operation.to_c();
        auto value = detail::sized<::madopilot_permission_t>();
        ::madopilot_error_t* error = nullptr;
        const Status status = api_->engine_permission(
            handle_, kind, &operation_c, &value, &error);
        if (!is_ok(status)) {
            return Result<Permission>::failure(
                detail::take_error(api_, status, error));
        }

        Permission out;
        out.kind = value.kind;
        out.state = value.state;
        if ((value.flags & (MADOPILOT_PERMISSION_HAS_DIAGNOSTIC |
                            MADOPILOT_PERMISSION_HAS_PLATFORM_CODE)) != 0u) {
            PermissionDiagnostic diagnostic;
            diagnostic.category = value.diagnostic_category;
            if ((value.flags & MADOPILOT_PERMISSION_HAS_PLATFORM_CODE) != 0u) {
                diagnostic.platform_code = value.platform_code;
            }
            diagnostic.platform_namespace =
                BorrowedStr(value.platform_namespace);
            diagnostic.context = BorrowedStr(value.context);
            out.diagnostic = diagnostic;
        }
        return Result<Permission>::success(out);
    }

    Result<Permission> permission(PermissionKind kind,
                                  const Operation& operation) const&& = delete;

    /// Queries a target's input descriptor without opening it.
    Result<InputDescriptor> input_descriptor(const TargetList& targets,
                                             std::size_t index,
                                             const Operation& operation) const {
        if (api_ == nullptr) {
            return detail::no_table<InputDescriptor>();
        }
        if (!detail::has_entry(api_, extent_,
                               MADOPILOT_API_SIZE_ENGINE_INPUT_DESCRIPTOR)) {
            return detail::unsupported<InputDescriptor>();
        }

        const auto operation_c = operation.to_c();
        auto value = detail::sized<::madopilot_input_descriptor_t>();
        ::madopilot_error_t* error = nullptr;
        const Status status = api_->engine_input_descriptor(
            handle_, targets.get(), index, &operation_c, &value, &error);
        if (!is_ok(status)) {
            return Result<InputDescriptor>::failure(
                detail::take_error(api_, status, error));
        }
        return Result<InputDescriptor>::success(
            detail::project_input_descriptor(value));
    }

    Result<TargetList> discover(const Operation& operation) const {
        if (api_ == nullptr) {
            return detail::no_table<TargetList>();
        }
        const auto operation_c = operation.to_c();
        ::madopilot_target_list_t* targets = nullptr;
        ::madopilot_error_t* error = nullptr;
        const Status status =
            api_->engine_discover(handle_, &operation_c, &targets, &error);
        if (!is_ok(status)) {
            return Result<TargetList>::failure(detail::take_error(api_, status, error));
        }

        return Result<TargetList>::success(TargetList(api_, extent_, targets));
    }

    /// Opens a session on one discovered target.
    ///
    /// The target identity and input policy are copied, so the list and request
    /// may be released immediately. Capture-only opens remain available through
    /// an ABI 1.0 table; input requires the ABI 1.2 entry.
    Result<Session> open_session(const TargetList& targets, std::size_t index,
                                 const OpenRequest& request,
                                 const Operation& operation) const {
        if (api_ == nullptr) {
            return detail::no_table<Session>();
        }
        const auto request_c = request.to_c();
        const auto operation_c = operation.to_c();
        ::madopilot_session_t* session = nullptr;
        ::madopilot_error_t* error = nullptr;
        Status status = MADOPILOT_STATUS_UNSUPPORTED;
        if (request.input_.has_value()) {
            if (!detail::has_entry(api_, extent_,
                                   MADOPILOT_API_SIZE_SESSION_OPEN_WITH_INPUT)) {
                return detail::unsupported<Session>();
            }
            status = api_->session_open_with_input(
                handle_, targets.get(), index, &request_c, &*request.input_,
                &operation_c, &session, &error);
        } else {
            status = api_->session_open(handle_, targets.get(), index, &request_c,
                                        &operation_c, &session, &error);
        }
        if (!is_ok(status)) {
            return Result<Session>::failure(detail::take_error(api_, status, error));
        }

        return Result<Session>::success(Session(api_, extent_, session));
    }

    /// Loads and validates one asset package.
    ///
    /// A failure here is the one that carries `AssetDetail`: which rule was
    /// broken and how far loading had got.
    Result<Package> load_package(const PackageSource& source,
                                 const Operation& operation) const {
        if (api_ == nullptr) {
            return detail::no_table<Package>();
        }
        const auto source_c = source.to_c();
        const auto operation_c = operation.to_c();
        ::madopilot_package_t* package = nullptr;
        ::madopilot_error_t* error = nullptr;
        const Status status =
            api_->package_load(handle_, &source_c, &operation_c, &package, &error);
        if (!is_ok(status)) {
            return Result<Package>::failure(detail::take_error(api_, status, error));
        }

        return Result<Package>::success(Package(api_, extent_, package));
    }

    /// Prepares one template from a loaded package. The result outlives the
    /// package.
    Result<Template> prepare_from_package(const Package& package, std::string_view id,
                                      const Operation& operation) const {
        if (api_ == nullptr) {
            return detail::no_table<Template>();
        }
        const auto operation_c = operation.to_c();
        ::madopilot_template_t* prepared = nullptr;
        ::madopilot_error_t* error = nullptr;
        const Status status = api_->template_prepare_from_package(handle_, package.get(),
                                                     detail::as_str(id), &operation_c,
                                                     &prepared, &error);
        if (!is_ok(status)) {
            return Result<Template>::failure(detail::take_error(api_, status, error));
        }

        return Result<Template>::success(Template(api_, extent_, prepared));
    }

private:
    friend class Api;
    friend class detail::Owner<Engine, ::madopilot_engine_t>;

    Engine(const ::madopilot_api_t* api, std::size_t extent,
           ::madopilot_engine_t* handle) noexcept
        : Owner(api, extent, handle) {}

    static Status retain_handle(const ::madopilot_api_t* api,
                                ::madopilot_engine_t* handle) noexcept {
        return api->engine_retain(handle);
    }

    static void release_handle(const ::madopilot_api_t* api,
                               ::madopilot_engine_t* handle) noexcept {
        api->engine_release(handle);
    }
};

/* ---------------------------------------------------------------------------
 * The negotiated table
 * ------------------------------------------------------------------------ */

/// The negotiated function table, and the root of everything else.
///
/// This is the one copyable type here. It owns nothing: the table belongs to the
/// library, is valid while it is loaded, and is never released. Every owner
/// keeps its own pointer to it, so the wrapper needs no global state.
class Api {
public:
    Api() noexcept = default;

    /// Negotiates the ABI this header declares.
    static Result<Api> load() {
        return load(MADOPILOT_ABI_MAJOR, MADOPILOT_ABI_MINOR, sizeof(::madopilot_api_t));
    }

    /// Negotiates explicitly, for a caller that accepts an older minor or that
    /// declares it understands only a prefix of the table.
    static Result<Api> load(std::uint32_t abi_major, std::uint32_t min_abi_minor,
                            std::size_t caller_struct_size) {
        const ::madopilot_api_t* table = nullptr;
        const Status status =
            ::madopilot_get_api(abi_major, min_abi_minor, caller_struct_size, &table);
        if (!is_ok(status) || table == nullptr) {
            return Result<Api>::failure(Error::from_status(
                is_ok(status) ? MADOPILOT_STATUS_INTERNAL : status));
        }

        const std::size_t library_extent = static_cast<std::size_t>(table->struct_size);
        const std::size_t negotiated_extent =
            caller_struct_size < library_extent ? caller_struct_size : library_extent;
        return Result<Api>::success(Api(table, negotiated_extent));
    }

    bool empty() const noexcept { return table_ == nullptr; }
    explicit operator bool() const noexcept { return table_ != nullptr; }

    /// The negotiated table, for a caller that needs an entry this wrapper does
    /// not expose.
    const ::madopilot_api_t* table() const noexcept { return table_; }

    /// The table prefix both sides agreed may be used.
    std::size_t extent() const noexcept { return extent_; }

    /// What the loaded library is. Both views are valid while it is loaded.
    Result<BuildInfo> describe_build() const {
        if (table_ == nullptr) {
            return detail::no_table<BuildInfo>();
        }
        auto build = detail::sized<::madopilot_build_info_t>();
        const Status status = table_->describe_build(&build);
        if (!is_ok(status)) {
            return Result<BuildInfo>::failure(Error::from_status(status));
        }

        BuildInfo out;
        out.abi_major = build.abi_major;
        out.abi_minor = build.abi_minor;
        out.table_size = build.table_size;
        out.library_version = BorrowedStr(build.library_version);
        out.required_backend = BorrowedStr(build.required_backend);

        return Result<BuildInfo>::success(out);
    }

    /// The current instant in the library's monotonic domain. Add to it to build
    /// the absolute deadline an `Operation` carries.
    Result<std::uint64_t> clock_now() const {
        if (table_ == nullptr) {
            return detail::no_table<std::uint64_t>();
        }
        std::uint64_t nanos = 0;
        const Status status = table_->clock_now(&nanos);
        return is_ok(status) ? Result<std::uint64_t>::success(nanos)
                             : Result<std::uint64_t>::failure(Error::from_status(status));
    }

    /// A stable lowercase slug for a status, borrowed from the library's static
    /// storage and therefore valid while it is loaded.
    BorrowedStr status_text(Status status) const noexcept {
        ::madopilot_str_t text{nullptr, 0};
        if (table_ != nullptr) {
            table_->status_text(status, &text);
        }
        return BorrowedStr(text);
    }

    Result<Cancellation> create_cancellation() const {
        if (table_ == nullptr) {
            return detail::no_table<Cancellation>();
        }
        ::madopilot_cancellation_t* cancellation = nullptr;
        const Status status = table_->cancellation_create(&cancellation);
        if (!is_ok(status)) {
            return Result<Cancellation>::failure(Error::from_status(status));
        }

        return Result<Cancellation>::success(
            Cancellation(table_, extent_, cancellation));
    }

    /// Builds an engine with diagnostics off. This remains available through
    /// the frozen ABI 1.0 prefix.
    Result<Engine> create_engine(const Source& source,
                                 const Operation& operation) const {
        if (table_ == nullptr) {
            return detail::no_table<Engine>();
        }
        const auto source_c = source.to_c();
        const auto operation_c = operation.to_c();
        ::madopilot_engine_t* engine = nullptr;
        ::madopilot_error_t* error = nullptr;
        const Status status =
            table_->engine_create(&source_c, &operation_c, &engine, &error);
        if (!is_ok(status)) {
            return Result<Engine>::failure(
                detail::take_error(table_, status, error));
        }
        return Result<Engine>::success(Engine(table_, extent_, engine));
    }

    /// Builds an engine with explicit bounded diagnostic options.
    Result<Engine> create_engine(const Source& source,
                                 const EngineOptions& options,
                                 const Operation& operation) const {
        if (table_ == nullptr) {
            return detail::no_table<Engine>();
        }
        if (!detail::has_entry(
                table_, extent_,
                MADOPILOT_API_SIZE_ENGINE_CREATE_WITH_OPTIONS)) {
            return detail::unsupported<Engine>();
        }
        const auto source_c = source.to_c();
        const auto options_c = options.to_c();
        const auto operation_c = operation.to_c();
        ::madopilot_engine_t* engine = nullptr;
        ::madopilot_error_t* error = nullptr;
        const Status status = table_->engine_create_with_options(
            &source_c, &options_c, &operation_c, &engine, &error);
        if (!is_ok(status)) {
            return Result<Engine>::failure(
                detail::take_error(table_, status, error));
        }

        return Result<Engine>::success(Engine(table_, extent_, engine));
    }

private:
    Api(const ::madopilot_api_t* table, std::size_t extent) noexcept
        : table_(table), extent_(extent) {}

    const ::madopilot_api_t* table_ = nullptr;
    std::size_t extent_ = 0;
};

} // namespace madopilot

#endif /* MADOPILOT_MADOPILOT_HPP */
