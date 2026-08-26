/*
 * The C++ wrapper's ownership contract, proven at compile time and at run time.
 *
 * The compile-time half is a block of `static_assert`s: every owning type is
 * move-only, nothrow-movable, and nothrow-destructible, and the two types that
 * are deliberately copyable say so here rather than by accident. Nothing in this
 * file needs to run for those to hold; a build that produces the program has
 * already proved them.
 *
 * The run-time half exercises what a static assertion cannot see: that a move
 * leaves one owner, that a clone survives its origin, that a child outlives its
 * parent, that a borrowed view stays readable exactly as long as its owner does,
 * that a zero-match search is a success, that a failure carries owned text and
 * leaves no residue behind it, and that none of it throws.
 *
 * Run by `cargo run --package mado-pilot-capi --example c-abi-check`.
 *
 *   usage: madopilot-cpp-ownership --package <dir>
 */

#include <atomic>
#include <cstdio>
#include <cstdlib>
#include <new>
#include <string>
#include <string_view>
#include <thread>
#include <type_traits>
#include <utility>
#include <vector>

#include "deterministic-scene.h"
#include "madopilot/madopilot.hpp"

/* ---------------------------------------------------------------------------
 * A starvable allocator
 *
 * Checks need allocations to fail either immediately or after one successful
 * allocation. They cover owned error-text cleanup and the strong exception
 * guarantee of C-record projections. The replacement forwards to malloc unless
 * a check has armed it, so every other allocation behaves normally. Each check
 * disarms it before reporting.
 * ------------------------------------------------------------------------ */

namespace {
bool starve_allocations = false;
std::atomic<int> allocations_before_failure{-1};
}

void* operator new(std::size_t size)
{
    if (starve_allocations) {
        throw std::bad_alloc();
    }
    const int remaining =
        allocations_before_failure.load(std::memory_order_relaxed);
    if (remaining == 0) {
        throw std::bad_alloc();
    }
    if (remaining > 0) {
        allocations_before_failure.fetch_sub(1, std::memory_order_relaxed);
    }
    // Zero bytes still has to yield a distinct address, which malloc is allowed
    // to refuse to provide.
    void* memory = std::malloc(size == 0 ? 1 : size);
    if (memory == nullptr) {
        throw std::bad_alloc();
    }
    return memory;
}

void operator delete(void* memory) noexcept { std::free(memory); }
void operator delete(void* memory, std::size_t) noexcept { std::free(memory); }

/* ---------------------------------------------------------------------------
 * Compile-time: the ownership shape
 * ------------------------------------------------------------------------ */

namespace {

/// Every type that owns a C handle has exactly this shape.
template <class T>
constexpr bool is_move_only_owner()
{
    return !std::is_copy_constructible_v<T> && !std::is_copy_assignable_v<T> &&
           std::is_move_constructible_v<T> && std::is_move_assignable_v<T> &&
           std::is_nothrow_move_constructible_v<T> &&
           std::is_nothrow_move_assignable_v<T> && std::is_nothrow_destructible_v<T> &&
           std::is_default_constructible_v<T>;
}

} // namespace

static_assert(is_move_only_owner<madopilot::Cancellation>(), "Cancellation is move-only");
static_assert(is_move_only_owner<madopilot::Engine>(), "Engine is move-only");
static_assert(is_move_only_owner<madopilot::TargetList>(), "TargetList is move-only");
static_assert(is_move_only_owner<madopilot::Package>(), "Package is move-only");
static_assert(is_move_only_owner<madopilot::Template>(), "Template is move-only");
static_assert(is_move_only_owner<madopilot::Session>(), "Session is move-only");
static_assert(is_move_only_owner<madopilot::Frame>(), "Frame is move-only");
static_assert(is_move_only_owner<madopilot::Mapping>(), "Mapping is move-only");
static_assert(is_move_only_owner<madopilot::MatchResult>(), "MatchResult is move-only");
static_assert(is_move_only_owner<madopilot::OcrResult>(), "OcrResult is move-only");
static_assert(is_move_only_owner<madopilot::ZoneScanOcrResult>(),
              "ZoneScanOcrResult is move-only");
static_assert(is_move_only_owner<madopilot::InputReceipt>(),
              "InputReceipt is move-only");
static_assert(is_move_only_owner<madopilot::DiagnosticReader>(),
              "DiagnosticReader is move-only");
static_assert(is_move_only_owner<madopilot::DiagnosticBatch>(),
              "DiagnosticBatch is move-only");

/* The two copyable types, asserted rather than assumed. `Api` owns nothing — the
 * table belongs to the library and is never released — and `Error` owns only its
 * own copies. */
static_assert(std::is_copy_constructible_v<madopilot::Api>,
              "Api owns nothing and is a value");
static_assert(std::is_copy_constructible_v<madopilot::Error>,
              "Error owns its own copies and is a value");

/* A `Result` is move-only exactly when its value is. */
static_assert(!std::is_copy_constructible_v<madopilot::Result<madopilot::Session>>,
              "a Result carrying an owner is move-only");
static_assert(std::is_copy_constructible_v<madopilot::Result<madopilot::Match>>,
              "a Result carrying a plain value is copyable");

/* Borrowed views are trivially copyable: copying one copies a pointer and a
 * length, and never a reference count. */
static_assert(std::is_trivially_copyable_v<madopilot::BorrowedStr>,
              "a borrowed string view is a pointer and a length");
static_assert(std::is_trivially_copyable_v<madopilot::BorrowedBytes>,
              "a borrowed byte view is a pointer and a length");

/* An accessor that hands out a borrowed view is callable on a named owner and
 * not on a temporary one. `load(...).take().describe()` reads correctly and
 * leaves every view in the result pointing into a package released at the end of
 * the full expression, so the rvalue overloads are deleted and the mistake is a
 * compile error. These detect that: `declval<T>()` is an rvalue and
 * `declval<T&>()` an lvalue, and a deleted overload makes the expression
 * ill-formed in the immediate context rather than failing the build here. */
namespace {

template <class T, class = void>
struct describes : std::false_type {};

template <class T>
struct describes<T, std::void_t<decltype(std::declval<T>().describe())>> : std::true_type {};
template <class T, class = void>
struct reads_ocr_text : std::false_type {};

template <class T>
struct reads_ocr_text<T, std::void_t<decltype(std::declval<T>().text_at(0))>>
    : std::true_type {};
template <class T, class = void>
struct reads_zone_ocr_text : std::false_type {};

template <class T>
struct reads_zone_ocr_text<
    T, std::void_t<decltype(std::declval<T>().text_at(0, 0))>>
    : std::true_type {};


template <class T, class = void>
struct indexes : std::false_type {};

template <class T>
struct indexes<T, std::void_t<decltype(std::declval<T>().at(0))>> : std::true_type {};

template <class T, class = void>
struct probes_permission : std::false_type {};

template <class T>
struct probes_permission<
    T, std::void_t<decltype(std::declval<T>().permission(
           MADOPILOT_PERMISSION_KIND_SCREEN_CAPTURE, std::declval<madopilot::Operation>()))>>
    : std::true_type {};

template <class T, class = void>
struct reads_ocr_descriptor : std::false_type {};

template <class T>
struct reads_ocr_descriptor<
    T, std::void_t<decltype(std::declval<T>().ocr_descriptor())>>
    : std::true_type {};

template <class T, class = void>
struct reads_ocr_provider_descriptor : std::false_type {};

template <class T>
struct reads_ocr_provider_descriptor<
    T, std::void_t<decltype(
           std::declval<T>().ocr_provider_descriptor())>>
    : std::true_type {};

} // namespace

static_assert(describes<madopilot::Package&>::value, "a named package describes itself");
static_assert(!describes<madopilot::Package>::value,
              "a temporary package does not, because its strings would dangle");
static_assert(describes<madopilot::Template&>::value, "a named template describes itself");
static_assert(!describes<madopilot::Template>::value, "a temporary template does not");
static_assert(describes<madopilot::Mapping&>::value, "a named mapping describes itself");
static_assert(!describes<madopilot::Mapping>::value, "a temporary mapping does not");
static_assert(describes<madopilot::MatchResult&>::value, "a named result describes itself");
static_assert(!describes<madopilot::MatchResult>::value, "a temporary result does not");
static_assert(describes<madopilot::OcrResult&>::value, "a named OCR result describes itself");
static_assert(!describes<madopilot::OcrResult>::value,
              "a temporary OCR result does not expose borrowed description views");
static_assert(reads_ocr_text<madopilot::OcrResult&>::value,
              "a named OCR result exposes borrowed text");
static_assert(!reads_ocr_text<madopilot::OcrResult>::value,
              "a temporary OCR result cannot expose borrowed text");
static_assert(describes<madopilot::ZoneScanOcrResult&>::value,
              "a named grouped OCR result describes itself");
static_assert(!describes<madopilot::ZoneScanOcrResult>::value,
              "a temporary grouped OCR result exposes no borrowed description");
static_assert(reads_zone_ocr_text<madopilot::ZoneScanOcrResult&>::value,
              "a named grouped OCR result exposes borrowed text");
static_assert(!reads_zone_ocr_text<madopilot::ZoneScanOcrResult>::value,
              "a temporary grouped OCR result cannot expose borrowed text");
static_assert(indexes<madopilot::TargetList&>::value, "a named target list is indexable");
static_assert(!indexes<madopilot::TargetList>::value, "a temporary target list is not");
static_assert(probes_permission<madopilot::Engine&>::value,
              "a named engine may return a borrowed permission diagnostic");
static_assert(!probes_permission<madopilot::Engine>::value,
              "a temporary engine may not return borrowed permission views");
static_assert(reads_ocr_descriptor<madopilot::Engine&>::value,
              "a named engine may return borrowed OCR descriptor views");
static_assert(!reads_ocr_descriptor<madopilot::Engine>::value,
              "a temporary engine may not return borrowed OCR descriptor views");
static_assert(reads_ocr_provider_descriptor<madopilot::Engine&>::value,
              "a named engine may return borrowed provider descriptor views");
static_assert(!reads_ocr_provider_descriptor<madopilot::Engine>::value,
              "a temporary engine may not return borrowed provider views");

/* Requests and fixed-width projections are values a caller composes, copies,
 * and reuses. */
static_assert(std::is_copy_constructible_v<madopilot::Operation>, "Operation is a value");
static_assert(std::is_copy_constructible_v<madopilot::EngineOptions>,
              "EngineOptions is a value");
static_assert(std::is_copy_constructible_v<madopilot::DefaultOcrOptions>,
              "DefaultOcrOptions owns both controlled paths");
static_assert(std::is_copy_constructible_v<madopilot::OcrProfileOptions>,
              "OcrProfileOptions owns both controlled paths");
static_assert(std::is_copy_constructible_v<madopilot::OcrProfileOptions::CView>,
              "each profile C projection repairs its own paths after copy");
static_assert(
    std::is_nothrow_move_constructible_v<madopilot::OcrProfileOptions::CView>,
    "profile C projection move repairs views without throwing");
static_assert(std::is_copy_constructible_v<madopilot::OcrProviderOptions>,
              "OcrProviderOptions owns its controlled provider root");
static_assert(std::is_copy_constructible_v<madopilot::OcrProviderOptions::CView>,
              "each provider C projection repairs its own root after copy");
static_assert(
    std::is_nothrow_move_constructible_v<
        madopilot::OcrProviderOptions::CView>,
    "provider C projection move repairs views without throwing");
static_assert(std::is_copy_constructible_v<madopilot::FindRequest>,
              "FindRequest is a value");
static_assert(std::is_copy_constructible_v<madopilot::OcrRequest>,
              "OcrRequest owns its string values");
static_assert(std::is_copy_constructible_v<madopilot::OcrRequest::CView>,
              "each OCR C projection repairs its own views after copy");
static_assert(std::is_nothrow_move_constructible_v<madopilot::OcrRequest::CView>,
              "OCR C projection move repairs views without throwing");
static_assert(std::is_copy_constructible_v<madopilot::ZoneScanOcrRequest>,
              "ZoneScanOcrRequest owns identities and zones");
static_assert(
    std::is_copy_constructible_v<madopilot::ZoneScanOcrRequest::CView>,
    "each grouped OCR C projection repairs strings and zones after copy");
static_assert(
    std::is_nothrow_move_constructible_v<
        madopilot::ZoneScanOcrRequest::CView>,
    "grouped OCR C projection move repairs views without throwing");
static_assert(std::is_copy_constructible_v<madopilot::Source>, "Source is a value");
static_assert(std::is_copy_constructible_v<madopilot::InputOpenRequest>,
              "InputOpenRequest is a value");
static_assert(std::is_copy_constructible_v<madopilot::InputEvent>,
              "InputEvent owns its variable data");
static_assert(std::is_copy_constructible_v<madopilot::InputRequest>,
              "InputRequest owns its event and delivery arrays");
static_assert(
    !std::is_copy_constructible_v<madopilot::InputRequest::CView> &&
        !std::is_copy_assignable_v<madopilot::InputRequest::CView> &&
        std::is_nothrow_move_constructible_v<madopilot::InputRequest::CView> &&
        std::is_nothrow_move_assignable_v<madopilot::InputRequest::CView>,
    "each call-local input projection is a move-only owner");
static_assert(std::is_copy_constructible_v<madopilot::EngineCapabilities>,
              "EngineCapabilities is a value");
static_assert(std::is_copy_constructible_v<madopilot::OcrEngineDescriptor>,
              "OcrEngineDescriptor is a value with engine-borrowed views");
static_assert(std::is_copy_constructible_v<madopilot::OpenRequest>,
              "OpenRequest owns its input policy");
static_assert(std::is_copy_constructible_v<madopilot::Permission>,
              "Permission is a value with explicitly borrowed diagnostic views");
static_assert(std::is_copy_constructible_v<madopilot::InputCapability>,
              "InputCapability contains no borrowed storage");
static_assert(std::is_copy_constructible_v<madopilot::InputDescriptor>,
              "InputDescriptor contains no borrowed storage");
static_assert(std::is_copy_constructible_v<madopilot::InputReceiptInfo>,
              "InputReceiptInfo is fixed-width receipt data");
static_assert(std::is_copy_constructible_v<madopilot::InputAttempt>,
              "InputAttempt is fixed-width route data");
static_assert(std::is_copy_constructible_v<madopilot::DiagnosticRecord>,
              "DiagnosticRecord contains no sensitive borrowed payload");
static_assert(
    std::is_same_v<decltype(madopilot::InputReceiptInfo::attempt_count),
                   std::uint64_t> &&
        std::is_same_v<decltype(madopilot::InputReceiptInfo::submitted),
                       std::uint64_t> &&
        std::is_same_v<decltype(madopilot::InputReceiptInfo::last_submitted),
                       std::optional<std::uint64_t>> &&
        std::is_same_v<decltype(madopilot::InputReceiptInfo::cleanup_released),
                       std::uint64_t> &&
        std::is_same_v<decltype(madopilot::InputReceiptInfo::cleanup_owed),
                       std::uint64_t> &&
        std::is_same_v<decltype(madopilot::InputAttempt::submitted),
                       std::uint64_t> &&
        std::is_same_v<decltype(madopilot::InputAttempt::last_submitted),
                       std::optional<std::uint64_t>>,
    "receipt and route-attempt semantic counts stay 64-bit");
static_assert(std::is_same_v<decltype(madopilot::DiagnosticRecord::region),
                             madopilot::Rect>,
              "search diagnostics expose the exact pixel rectangle");
static_assert(
    std::is_same_v<decltype(&madopilot::InputReceipt::attempt_count),
                   madopilot::Result<std::size_t> (
                       madopilot::InputReceipt::*)() const> &&
        std::is_same_v<decltype(&madopilot::InputReceipt::attempt_at),
                       madopilot::Result<madopilot::InputAttempt> (
                           madopilot::InputReceipt::*)(std::size_t) const>,
    "attempt access remains indexed by size_t");
static_assert(madopilot::InputEvent::max_text_chars ==
                  MADOPILOT_INPUT_MAX_TEXT_CHARS &&
                  madopilot::InputEvent::max_text_utf8_bytes ==
                      MADOPILOT_INPUT_MAX_TEXT_UTF8_BYTES &&
                  madopilot::InputEvent::max_delay_nanos ==
                      MADOPILOT_INPUT_MAX_DELAY_NANOS &&
                  madopilot::InputEvent::max_scroll_notches ==
                      MADOPILOT_INPUT_MAX_SCROLL_NOTCHES &&
                  madopilot::InputEvent::min_function_key ==
                      MADOPILOT_INPUT_MIN_FUNCTION_KEY &&
                  madopilot::InputEvent::max_function_key ==
                      MADOPILOT_INPUT_MAX_FUNCTION_KEY,
              "InputEvent exposes the C contract ceilings without restating them");
static_assert(madopilot::InputRequest::abi_max_events ==
                  MADOPILOT_INPUT_MAX_EVENTS &&
                  madopilot::InputRequest::max_cleanup_events ==
                      MADOPILOT_INPUT_MAX_CLEANUP_EVENTS &&
                  madopilot::InputRequest::max_cleanup_timeout_nanos ==
                      MADOPILOT_INPUT_MAX_CLEANUP_NANOS,
              "InputRequest exposes the C sequence and cleanup ceilings");
static_assert(std::is_same_v<madopilot::PermissionState,
                             ::madopilot_permission_state_t> &&
                  std::is_same_v<madopilot::CapabilitySupport,
                                 ::madopilot_capability_support_t> &&
                  std::is_same_v<madopilot::InputDelivery,
                                 ::madopilot_input_delivery_t> &&
                  std::is_same_v<madopilot::InputAddressScope,
                                 ::madopilot_input_address_scope_t> &&
                  std::is_same_v<madopilot::SubmissionEvidence,
                                 ::madopilot_submission_evidence_t> &&
                  std::is_same_v<madopilot::SequenceOutcome,
                                 ::madopilot_sequence_outcome_t> &&
                  std::is_same_v<madopilot::InputFault,
                                 ::madopilot_input_fault_t>,
              "C++ preserves unknown C values instead of narrowing them");
/* ---------------------------------------------------------------------------
 * Run-time: the ownership behaviour
 * ------------------------------------------------------------------------ */

namespace {

int failures = 0;
const char* current = "";

bool check(bool condition, const char* what)
{
    if (!condition) {
        std::printf("FAIL: %s: %s\n", current, what);
        failures += 1;
    }
    return condition;
}

template <class T>
bool check_ok(const madopilot::Result<T>& result, const char* what)
{
    if (!result) {
        std::printf("FAIL: %s: %s returned %d\n", current, what,
                    static_cast<int>(result.status()));
        if (!result.error().message().empty()) {
            std::printf("      %s\n", result.error().message().c_str());
        }
        failures += 1;
        return false;
    }
    return true;
}

/// The deterministic flow, built once and shared by the checks below.
///
/// Everything here is move-only, so the fixture holds the owners and hands out
/// references and clones. Nothing in this struct releases anything explicitly.
struct Fixture {
    madopilot::Api api;
    std::vector<std::uint8_t> scene;
    madopilot::Cancellation cancellation;
    madopilot::Operation operation;
    madopilot::Engine engine;
    madopilot::TargetList targets;
    madopilot::Session session;
    madopilot::Frame frame;
    madopilot::Package package;
    madopilot::Template present;
    madopilot::Template absent;

    /// A second session on the same target, for a check that closes one.
    ///
    /// The shared session stays open: a closed session starts no further work,
    /// so closing it would decide the outcome of every later check.
    madopilot::Session open_another()
    {
        madopilot::OpenRequest request;
        request.require_format(MADOPILOT_PIXEL_FORMAT_RGBA8);
        auto opened = engine.open_session(targets, 0, request, operation);
        if (!check_ok(opened, "open a second session")) {
            return madopilot::Session();
        }
        return opened.take();
    }

    bool build(const char* package_path)
    {
        auto loaded = madopilot::Api::load();
        if (!check_ok(loaded, "Api::load")) {
            return false;
        }
        api = loaded.take();

        auto now = api.clock_now();
        if (!check_ok(now, "clock_now")) {
            return false;
        }
        auto token = api.create_cancellation();
        if (!check_ok(token, "create_cancellation")) {
            return false;
        }
        cancellation = token.take();
        operation.deadline(now.value() + 30ull * 1000ull * 1000ull * 1000ull)
            .cancellation(cancellation);

        scene.resize(SCENE_BYTES);
        scene_fill_rgba(scene.data());

        madopilot::ReplayFrame supplied;
        supplied.extent(SCENE_WIDTH, SCENE_HEIGHT)
            .format(MADOPILOT_PIXEL_FORMAT_RGBA8)
            .continuity(MADOPILOT_CONTINUITY_CONTINUOUS)
            .pixels(scene.data(), scene.size());

        auto source = madopilot::Source::replay_memory("panel");
        source.frame(supplied);

        auto built = api.create_engine(source, operation);
        if (!check_ok(built, "create_engine")) {
            return false;
        }
        engine = built.take();

        auto discovered = engine.discover(operation);
        if (!check_ok(discovered, "discover")) {
            return false;
        }
        targets = discovered.take();

        session = open_another();
        if (session.empty()) {
            return false;
        }

        auto taken = session.acquire_frame(operation);
        if (!check_ok(taken, "session frame")) {
            return false;
        }
        frame = taken.take();

        auto loaded_package = engine.load_package(
            madopilot::PackageSource::directory(package_path), operation);
        if (!check_ok(loaded_package, "load_package")) {
            return false;
        }
        package = loaded_package.take();

        auto patch = engine.prepare_from_package(package, "panel.patch", operation);
        if (!check_ok(patch, "prepare_from_package(panel.patch)")) {
            return false;
        }
        present = patch.take();

        auto nothing = engine.prepare_from_package(package, "panel.absent", operation);
        if (!check_ok(nothing, "prepare_from_package(panel.absent)")) {
            return false;
        }
        absent = nothing.take();

        return true;
    }
};

/// Moving an owner transfers the reference and leaves the source empty.
void moving_an_owner_transfers_it(Fixture& fixture)
{
    madopilot::Frame source = fixture.frame.clone();
    check(!source.empty(), "a clone starts non-empty");

    const madopilot::Frame destination = std::move(source);
    // NOLINTNEXTLINE(bugprone-use-after-move) — inspecting the moved-from state
    // is exactly what this checks.
    check(source.empty(), "the moved-from owner is empty");
    check(!destination.empty(), "the destination owns the reference");
    check_ok(destination.stamp(), "the destination still answers");

    // An emptied owner keeps the table it came from, so an operation on it is
    // refused by the C boundary rather than by the wrapper.
    const auto refused = source.stamp();
    check(!refused && refused.status() == MADOPILOT_STATUS_INVALID_ARGUMENT,
          "an emptied owner is refused by the C boundary");

    // Both destructors run at the end of this scope. Only one release happens,
    // because only one of them holds a reference.
}

/// Move assignment releases what the destination held first.
void move_assignment_releases_the_destination(Fixture& fixture)
{
    madopilot::Frame first = fixture.frame.clone();
    madopilot::Frame second = fixture.frame.clone();

    second = std::move(first);
    // NOLINTNEXTLINE(bugprone-use-after-move)
    check(first.empty(), "the moved-from owner is empty after assignment");
    check_ok(second.stamp(), "the destination answers after assignment");
    check_ok(fixture.frame.stamp(), "the original is untouched");
}

/// Assigning an owner to itself leaves it owning what it owned.
///
/// `Owner::operator=` guards `this != &other` and resets the destination first.
/// Without the guard, self-assignment releases the handle and then moves the
/// now-dangling value back into place — a use-after-free that no test reached:
/// deleting the guard left the whole suite green.
void self_move_assignment_keeps_the_owner_intact(Fixture& fixture)
{
    madopilot::Frame owner = fixture.frame.clone();

    // The cast is what makes this a self-move rather than a self-copy, and the
    // pragmas are here because warning about exactly this is the compiler doing
    // its job — the point of the test is that the guarded path is correct when a
    // caller does it anyway, through an alias the compiler cannot see through.
#if defined(__clang__)
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wself-move"
#elif defined(__GNUC__)
#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wself-move"
#endif
    owner = std::move(owner);
#if defined(__clang__)
#pragma clang diagnostic pop
#elif defined(__GNUC__)
#pragma GCC diagnostic pop
#endif

    check(!owner.empty(), "self-assignment does not release the owner");
    check_ok(owner.stamp(), "and it still answers afterwards");
    check_ok(fixture.frame.stamp(), "the original is untouched");
}

/// A clone is an independent owner that outlives the one it came from.
void a_clone_outlives_its_origin(Fixture& fixture)
{
    madopilot::Mapping survivor;
    {
        madopilot::MapRequest request;
        request.format(MADOPILOT_PIXEL_FORMAT_RGBA8);
        auto mapped = fixture.frame.map(request, fixture.operation);
        if (!check_ok(mapped, "frame map")) {
            return;
        }
        const madopilot::Mapping original = mapped.take();
        survivor = original.clone();
        check(!survivor.empty(), "the clone owns a reference of its own");
        // `original` is released here.
    }

    const auto image = survivor.describe();
    if (check_ok(image, "the clone still describes after its origin is gone")) {
        check(image.value().bytes.size() == SCENE_BYTES,
              "the clone's bytes are the whole frame");
    }
}

/// Cloning an empty owner is an empty owner, not a null dereference.
void cloning_an_empty_owner_is_empty(Fixture& fixture)
{
    const madopilot::Frame empty;
    const madopilot::Frame clone = empty.clone();
    check(clone.empty(), "cloning a default-constructed owner yields an empty owner");

    madopilot::Frame emptied = fixture.frame.clone();
    emptied.reset();
    const madopilot::Frame from_emptied = emptied.clone();
    check(from_emptied.empty(), "cloning an emptied owner yields an empty owner");
}

/// A separately owned child stays valid after every parent is destroyed.
void a_child_outlives_its_parents(Fixture& fixture)
{
    madopilot::Mapping mapping;
    madopilot::MatchResult result;

    {
        // The parents: a session of this check's own, the frame it published,
        // and a clone of the prepared template. All three die at the closing
        // brace, along with the session's close.
        madopilot::Session session = fixture.open_another();
        if (session.empty()) {
            return;
        }

        auto taken = session.acquire_frame(fixture.operation);
        if (!check_ok(taken, "session frame")) {
            return;
        }
        const madopilot::Frame frame = taken.take();

        madopilot::MapRequest request;
        request.format(MADOPILOT_PIXEL_FORMAT_RGBA8);
        auto mapped = frame.map(request, fixture.operation);
        if (!check_ok(mapped, "frame map")) {
            return;
        }
        mapping = mapped.take();

        const madopilot::Template prepared = fixture.present.clone();
        madopilot::FindRequest find;
        find.frame(frame).search_for(prepared);
        auto found = session.find(find, fixture.operation);
        if (!check_ok(found, "find")) {
            return;
        }
        result = found.take();

        check_ok(session.close(fixture.operation), "close the parent session");
    }

    const auto image = mapping.describe();
    if (check_ok(image, "the mapping describes after its session closed")) {
        check(image.value().bytes.size() == SCENE_BYTES,
              "the mapped bytes are still the whole frame");
    }

    const auto stamp = result.stamp();
    if (check_ok(stamp, "the result is still correlated")) {
        check(stamp.value().sequence == 0, "with the exact frame it searched");
    }

    const auto info = result.describe();
    if (check_ok(info, "the result still describes")) {
        check(info.value().match_count == 2, "and still reports its matches");
        check(!info.value().backend_id.empty(),
              "and its backend text is still readable from the result");
    }
}

/// A borrowed byte view is stable while its owner is retained.
void a_borrowed_view_tracks_its_owner(Fixture& fixture)
{
    madopilot::MapRequest request;
    request.format(MADOPILOT_PIXEL_FORMAT_RGBA8);
    auto mapped = fixture.frame.map(request, fixture.operation);
    if (!check_ok(mapped, "frame map")) {
        return;
    }
    const madopilot::Mapping mapping = mapped.take();

    const auto first = mapping.describe();
    const auto second = mapping.describe();
    if (!check_ok(first, "first describe") || !check_ok(second, "second describe")) {
        return;
    }

    check(first.value().bytes.data() == second.value().bytes.data(),
          "the same mapping hands back the same pointer");
    check(first.value().bytes.size() == second.value().bytes.size(),
          "and the same length");
    check(first.value().stride == second.value().stride, "and the same stride");
    check(first.value().format == second.value().format, "and the same pixel format");

    // The copy is the caller's, on its own lifetime.
    const std::vector<std::uint8_t> owned = first.value().bytes.to_vector();
    if (check(owned.size() == SCENE_BYTES, "to_vector copies the whole view")) {
        check(owned.front() == first.value().bytes.data()[0], "and copies the bytes");
    }
}

/// Text that must outlive a C handle is copied, not borrowed.
void error_text_outlives_the_c_handle(Fixture& fixture)
{
    madopilot::Error kept;
    {
        const auto unreadable = fixture.engine.load_package(
            madopilot::PackageSource::directory("no-such-package-directory"),
            fixture.operation);
        check(!unreadable, "an unreadable source fails to load");
        kept = unreadable.error();
        // The C error handle was described, copied, and released inside the
        // wrapper before this Result was returned.
    }

    check(kept.status() != MADOPILOT_STATUS_OK, "the copied error keeps its status");
    check(kept.category() == MADOPILOT_ERROR_CATEGORY_ASSET, "and its category");
    check(!kept.message().empty(), "and owns its message");
    if (check(kept.asset_detail().has_value(), "and carries the asset rule and stage")) {
        check(kept.asset_detail()->fault == MADOPILOT_ASSET_FAULT_SOURCE_UNREADABLE,
              "which names the rule that was broken");
        check(kept.asset_detail()->stage == MADOPILOT_ASSET_STAGE_SOURCE,
              "and how far loading had got");
    }
}

/// A caller that reads only the status still gets the C error handle released,
/// and no residue is left for the next call to find.
void a_failure_leaves_no_residue(Fixture& fixture)
{
    for (int attempt = 0; attempt < 32; ++attempt) {
        const auto refused = fixture.engine.load_package(
            madopilot::PackageSource::directory("no-such-package-directory"),
            fixture.operation);
        check(refused.status() == MADOPILOT_STATUS_ASSET_INVALID,
              "the same refusal every time");
    }

    const auto now = fixture.api.clock_now();
    if (check_ok(now, "a later call succeeds")) {
        check(now.error().status() == MADOPILOT_STATUS_OK,
              "and carries no failure from the ones before it");
    }
}

/* --- The error handle outlives nothing, including a throw ------------------ */

int fake_releases = 0;

madopilot_status_t counting_error_release(madopilot_error_t*)
{
    fake_releases += 1;
    return MADOPILOT_STATUS_OK;
}

/// Describes an error whose message the wrapper will try to copy.
///
/// The text is borrowed from static storage, so nothing here depends on the
/// handle being real: the handle this fake table is called with is a valid
/// address that neither entry dereferences.
madopilot_status_t describing_a_message(const madopilot_error_t*,
                                        madopilot_error_detail_t* out_detail)
{
    static const char text[] = "a message the caller is about to copy";

    out_detail->flags = 0;
    out_detail->status = MADOPILOT_STATUS_INTERNAL;
    out_detail->category = MADOPILOT_ERROR_CATEGORY_UNSPECIFIED;
    out_detail->message = madopilot_str_t{text, sizeof(text) - 1};

    return MADOPILOT_STATUS_OK;
}

/// `detail::take_error` releases the C handle even when copying its text throws.
///
/// The wrapper's one documented exception is `std::bad_alloc` from the owned
/// error text, and that is the path on which the release used to be skipped:
/// it was the last statement of the function rather than a scope guard. Nothing
/// the real library can be asked to do makes an allocation fail, so this drives
/// `detail::take_error` directly, against a table of two fakes and a starved allocator.
void a_throwing_copy_still_releases_the_error(Fixture&)
{
    madopilot_api_t table{};
    table.error_describe = describing_a_message;
    table.error_release = counting_error_release;

    // Never dereferenced: the two entries above ignore it and the wrapper only
    // passes it along. It has to be non-null, because null means "no handle".
    madopilot_error_t* const handle = reinterpret_cast<madopilot_error_t*>(&table);

    fake_releases = 0;
    bool threw = false;
    starve_allocations = true;
    try {
        static_cast<void>(madopilot::detail::take_error(&table, MADOPILOT_STATUS_INTERNAL, handle));
    } catch (const std::bad_alloc&) {
        starve_allocations = false;
        threw = true;
    }
    starve_allocations = false;

    check(threw, "copying the message throws when the allocator refuses");
    check(fake_releases == 1, "and the error handle is released exactly once anyway");
}

/// A search that qualifies nothing is a success with an empty optional.
void zero_matches_is_a_success(Fixture& fixture)
{
    madopilot::FindRequest find;
    find.frame(fixture.frame).search_for(fixture.absent);

    const auto found = fixture.session.find(find, fixture.operation);
    if (!check_ok(found, "find(panel.absent)")) {
        return;
    }

    const auto best = found.value().first_match();
    if (check_ok(best, "first_match")) {
        check(!best.value().has_value(), "the optional match is empty");
    }

    const auto info = found.value().describe();
    if (check_ok(info, "describe")) {
        check(info.value().match_count == 0, "and the count is zero");
    }

    const auto matches = found.value().matches();
    if (check_ok(matches, "matches")) {
        check(matches.value().empty(), "and the match list is empty");
    }

    // Correlated with the searched frame exactly as a non-empty result is.
    const auto stamp = found.value().stamp();
    if (check_ok(stamp, "stamp")) {
        const auto frame_stamp = fixture.frame.stamp();
        if (check_ok(frame_stamp, "frame stamp")) {
            check(stamp.value().stream == frame_stamp.value().stream &&
                      stamp.value().sequence == frame_stamp.value().sequence,
                  "with the exact frame that was searched");
        }
    }
}

/// A caller-supplied region must be in capture pixels. Any other space is the
/// C ABI's own refusal, carried through unchanged and not converted.
///
/// The status is asserted rather than merely checked for failure: the C ABI has
/// no general coordinate-conversion entry, so a region it does not read is
/// invalid argument from the boundary, not `MADOPILOT_STATUS_UNSUPPORTED`. That
/// distinction is what `docs/c-abi.md` documents, and a check that only asked
/// "did it fail" would pass whichever status the boundary chose.
void an_unsupported_coordinate_space_is_refused(Fixture& fixture)
{
    madopilot::Rect region{MADOPILOT_SPACE_TARGET_LOGICAL, 0, 0, 4, 4};

    madopilot::MapRequest request;
    request.format(MADOPILOT_PIXEL_FORMAT_RGBA8).region(region);

    const auto refused = fixture.frame.map(request, fixture.operation);
    check(!refused, "a region in an unsupported space does not map");
    check(refused.status() == MADOPILOT_STATUS_INVALID_ARGUMENT,
          "and is invalid argument, because the prefix converts nothing");
    check(refused.error().category() == MADOPILOT_ERROR_CATEGORY_ABI,
          "reported by the boundary rather than by capture");

    // The same call through the raw table, to prove the wrapper changed nothing.
    const auto request_c = request.to_c();
    const auto operation_c = fixture.operation.to_c();
    madopilot_mapping_t* mapping = nullptr;
    madopilot_error_t* error = nullptr;
    const madopilot::Status direct = fixture.api.table()->frame_map(
        fixture.frame.get(), &request_c, &operation_c, &mapping, &error);
    fixture.api.table()->error_release(error);
    fixture.api.table()->mapping_release(mapping);

    check(refused.status() == direct, "and the wrapper reports what the C entry did");

    // A search region is held to the same rule, so the two entries cannot drift.
    madopilot::FindRequest search;
    search.frame(fixture.frame).search_for(fixture.present).region(region);
    const auto unsearched = fixture.session.find(search, fixture.operation);
    check(!unsearched, "a search region in an unsupported space does not search");
    check(unsearched.status() == MADOPILOT_STATUS_INVALID_ARGUMENT,
          "with the same status the mapping entry gave");
}

/// ABI 1.2 projects capture-only wiring explicitly: supported capture, no
/// permission probe, and no input delivery. Unsupported work is a failed
/// `Result`; successful capability queries remain ordinary values.
void abi_1_2_replay_capabilities_are_explicit(Fixture& fixture)
{
    const auto engine_capabilities = fixture.engine.capabilities();
    if (check_ok(engine_capabilities, "engine capabilities")) {
        check(!engine_capabilities.value().delivers_input(),
              "the replay engine reports no input adapter");
        check(!engine_capabilities.value().reads_permissions(),
              "the replay engine reports no permission probe");
    }

    const auto target = fixture.targets.at(0);
    const auto input_capability = fixture.targets.input_capability(
        0, MADOPILOT_INPUT_OPERATION_POINTER,
        MADOPILOT_INPUT_DELIVERY_PROCESS_DIRECTED);
    if (check_ok(target, "target") &&
        check_ok(input_capability, "input capability")) {
        check(input_capability.value().target == target.value().target,
              "capability identity matches discovery");
        check(input_capability.value().operation ==
                  MADOPILOT_INPUT_OPERATION_POINTER &&
                  input_capability.value().delivery ==
                      MADOPILOT_INPUT_DELIVERY_PROCESS_DIRECTED,
              "capability preserves the queried pair");
        check(input_capability.value().support ==
                  MADOPILOT_CAPABILITY_UNSUPPORTED,
              "replay input is explicitly unsupported");
        check(!input_capability.value().permission.has_value() &&
                  !input_capability.value().evidence.has_value(),
              "an unsupported route claims no permission or submission evidence");
    }

    const auto engine_descriptor =
        fixture.engine.input_descriptor(fixture.targets, 0, fixture.operation);
    const auto session_descriptor = fixture.session.input_descriptor();
    if (check_ok(engine_descriptor, "engine input descriptor") &&
        check_ok(session_descriptor, "session input descriptor")) {
        check(engine_descriptor.value().target == session_descriptor.value().target,
              "pre-open and accepted descriptors name the same target");
        check(engine_descriptor.value().known_pairs == MADOPILOT_INPUT_PAIRS_ALL &&
                  session_descriptor.value().known_pairs ==
                      MADOPILOT_INPUT_PAIRS_ALL &&
                  engine_descriptor.value().supported_pairs == 0 &&
                  session_descriptor.value().supported_pairs == 0 &&
                  engine_descriptor.value().unknown_pairs == 0 &&
                  session_descriptor.value().unknown_pairs == 0,
              "both descriptors distinguish known unsupported input");
    }

    const auto permission = fixture.engine.permission(
        MADOPILOT_PERMISSION_KIND_SCREEN_CAPTURE, fixture.operation);
    check(!permission && permission.status() == MADOPILOT_STATUS_UNSUPPORTED,
          "an absent permission probe is unsupported");
    check(permission.error().category() == MADOPILOT_ERROR_CATEGORY_PERMISSION,
          "the unsupported probe remains a permission failure");

    madopilot::InputRequest request;
    request.event(madopilot::InputEvent::delay(1))
        .delivery(MADOPILOT_INPUT_DELIVERY_PROCESS_DIRECTED);
    const auto refused = fixture.session.send_input(request, fixture.operation);
    check(!refused && refused.status() == MADOPILOT_STATUS_UNSUPPORTED,
          "a capture-only session refuses unavailable input before admission");
    check(refused.error().category() == MADOPILOT_ERROR_CATEGORY_INPUT,
          "the refusal carries an owned input error");
}

/// The existing ABI 1.2 C records project the public process-directed contract
/// without strengthening invocation into application consumption.
void abi_1_2_process_directed_projection_is_truthful(Fixture&)
{
    auto capability = madopilot::detail::sized<madopilot_input_capability_t>();
    capability.flags = MADOPILOT_INPUT_CAPABILITY_HAS_EVIDENCE;
    capability.target = UINT64_C(41);
    capability.operation = MADOPILOT_INPUT_OPERATION_KEYBOARD;
    capability.delivery = MADOPILOT_INPUT_DELIVERY_PROCESS_DIRECTED;
    capability.support = MADOPILOT_CAPABILITY_UNKNOWN;
    capability.address_scope = MADOPILOT_INPUT_ADDRESS_OWNING_PROCESS;
    capability.evidence = MADOPILOT_SUBMISSION_EVIDENCE_INVOCATION_ONLY;
    const auto projected_capability =
        madopilot::detail::project_input_capability(capability);
    check(projected_capability.target == UINT64_C(41) &&
              projected_capability.operation ==
                  MADOPILOT_INPUT_OPERATION_KEYBOARD &&
              projected_capability.delivery ==
                  MADOPILOT_INPUT_DELIVERY_PROCESS_DIRECTED &&
              projected_capability.support == MADOPILOT_CAPABILITY_UNKNOWN &&
              projected_capability.address_scope ==
                  MADOPILOT_INPUT_ADDRESS_OWNING_PROCESS &&
              projected_capability.evidence.has_value() &&
              *projected_capability.evidence ==
                  MADOPILOT_SUBMISSION_EVIDENCE_INVOCATION_ONLY,
          "capability projection keeps process scope, unknown compatibility, "
          "and invocation-only evidence");

    madopilot::InputOpenRequest open;
    open.requirement(MADOPILOT_INPUT_REQUIRED)
        .require_pairs(MADOPILOT_INPUT_PAIR_KEYBOARD_PROCESS_DIRECTED);
    const auto open_c = open.to_c();
    check(open_c.required_pairs ==
                  MADOPILOT_INPUT_PAIR_KEYBOARD_PROCESS_DIRECTED &&
              open_c.preferred_pairs == 0,
          "session open opts into only the process-directed pair");

    madopilot::InputRequest request;
    request.event(madopilot::InputEvent::key_press(MADOPILOT_KEY_ENTER))
        .delivery(MADOPILOT_INPUT_DELIVERY_PROCESS_DIRECTED)
        .focus_policy(MADOPILOT_FOCUS_PRESERVE);
    const auto request_c = request.to_c();
    check(request_c.value().delivery_count == 1 &&
              request_c.value().deliveries != nullptr &&
              request_c.value().deliveries[0] ==
                  MADOPILOT_INPUT_DELIVERY_PROCESS_DIRECTED &&
              request_c.value().focus_policy == MADOPILOT_FOCUS_PRESERVE,
          "the request contains no implicit system fallback");

    auto receipt = madopilot::detail::sized<madopilot_input_receipt_info_t>();
    receipt.flags = MADOPILOT_INPUT_RECEIPT_HAS_SELECTED_ROUTE |
                    MADOPILOT_INPUT_RECEIPT_HAS_LAST_SUBMITTED |
                    MADOPILOT_INPUT_RECEIPT_HAS_EVIDENCE;
    receipt.target = UINT64_C(41);
    receipt.outcome = MADOPILOT_SEQUENCE_COMPLETE;
    receipt.selected_route = MADOPILOT_INPUT_DELIVERY_PROCESS_DIRECTED;
    receipt.address_scope = MADOPILOT_INPUT_ADDRESS_OWNING_PROCESS;
    receipt.attempt_count = 1;
    receipt.submitted = 1;
    receipt.last_submitted = 0;
    receipt.evidence = MADOPILOT_SUBMISSION_EVIDENCE_INVOCATION_ONLY;
    receipt.cleanup = MADOPILOT_CLEANUP_NOT_NEEDED;
    const auto projected_receipt =
        madopilot::detail::project_input_receipt_info(receipt);
    check(projected_receipt.selected_route.has_value() &&
              *projected_receipt.selected_route ==
                  MADOPILOT_INPUT_DELIVERY_PROCESS_DIRECTED &&
              projected_receipt.address_scope ==
                  MADOPILOT_INPUT_ADDRESS_OWNING_PROCESS &&
              projected_receipt.evidence.has_value() &&
              *projected_receipt.evidence ==
                  MADOPILOT_SUBMISSION_EVIDENCE_INVOCATION_ONLY &&
              projected_receipt.attempt_count == 1 &&
              projected_receipt.submitted == 1 &&
              !projected_receipt.used_fallback,
          "receipt projection reports invocation progress without a fallback "
          "or consumption claim");

    auto attempt = madopilot::detail::sized<madopilot_input_attempt_t>();
    attempt.flags = MADOPILOT_INPUT_ATTEMPT_HAS_LAST_SUBMITTED |
                    MADOPILOT_INPUT_ATTEMPT_HAS_EVIDENCE;
    attempt.route = MADOPILOT_INPUT_DELIVERY_PROCESS_DIRECTED;
    attempt.address_scope = MADOPILOT_INPUT_ADDRESS_OWNING_PROCESS;
    attempt.outcome = MADOPILOT_SEQUENCE_COMPLETE;
    attempt.submitted = 1;
    attempt.last_submitted = 0;
    attempt.evidence = MADOPILOT_SUBMISSION_EVIDENCE_INVOCATION_ONLY;
    const auto projected_attempt =
        madopilot::detail::project_input_attempt(attempt);
    check(projected_attempt.route ==
                  MADOPILOT_INPUT_DELIVERY_PROCESS_DIRECTED &&
              projected_attempt.address_scope ==
                  MADOPILOT_INPUT_ADDRESS_OWNING_PROCESS &&
              projected_attempt.evidence.has_value() &&
              *projected_attempt.evidence ==
                  MADOPILOT_SUBMISSION_EVIDENCE_INVOCATION_ONLY,
          "attempt projection retains the same process-addressed threshold");
}

/// Variable-size request data belongs to the C++ values that expose it. Copies
/// rebuild their C views from their own strings and arrays.
void abi_1_2_requests_own_their_storage(Fixture& fixture)
{
    std::string source_text = "copied text";
    const madopilot::InputEvent text = madopilot::InputEvent::text(source_text);
    source_text.assign("changed");

    madopilot::InputRequest original;
    original.event(text)
        .delivery(MADOPILOT_INPUT_DELIVERY_PROCESS_DIRECTED)
        .delivery(MADOPILOT_INPUT_DELIVERY_SYSTEM)
        .focus_policy(MADOPILOT_FOCUS_REQUIRE_FOCUSED)
        .geometry_policy(MADOPILOT_GEOMETRY_REQUIRE_UNCHANGED)
        .cleanup_budget(4, 5);
    madopilot::InputRequest copied = original;

    const auto request_view = copied.to_c();
    const auto second_view = copied.to_c();
    const auto& request = request_view.value();
    const auto& second_request = second_view.value();
    check(request.events != second_request.events,
          "each C projection owns an independent event-record array");
    check(request.event_count == 1 && request.events != nullptr,
          "the copied request owns one event");
    check(request.event_stride == sizeof(madopilot_input_event_t),
          "the request advertises the event stride it compiled with");
    if (request.event_count == 1 && request.events != nullptr) {
        const auto& event = request.events[0];
        check(event.kind == MADOPILOT_INPUT_EVENT_TEXT,
              "the copied event keeps its active variant");
        check(std::string_view(event.text.data, event.text.len) == "copied text",
              "the event copied text rather than borrowing its source");
    }
    check(request.delivery_count == 2 && request.deliveries != nullptr,
          "the copied request owns its delivery order");
    if (request.delivery_count == 2 && request.deliveries != nullptr) {
        check(request.deliveries[0] ==
                      MADOPILOT_INPUT_DELIVERY_PROCESS_DIRECTED &&
                  request.deliveries[1] == MADOPILOT_INPUT_DELIVERY_SYSTEM,
              "explicit delivery precedence survives the copy");
    }
    check(request.focus_policy == MADOPILOT_FOCUS_REQUIRE_FOCUSED &&
              request.geometry_policy == MADOPILOT_GEOMETRY_REQUIRE_UNCHANGED,
          "typed policies survive the copy");
    check((request.flags & MADOPILOT_INPUT_REQUEST_HAS_CLEANUP_BUDGET) != 0u &&
              request.cleanup_max_events == 4 && request.cleanup_timeout_nanos == 5,
          "the explicit cleanup budget survives the copy");

    auto move_source = copied.to_c();
    const auto* move_source_events = move_source.value().events;
    auto move_constructed = std::move(move_source);
    check(move_constructed.value().event_count == 1 &&
              move_constructed.value().events == move_source_events,
          "move construction rebinds the C record to the transferred array");
    check((move_source.value().event_count == 0 &&
           move_source.value().events == nullptr) ||
              (move_source.value().event_count != 0 &&
               move_source.value().events != nullptr),
          "the moved-from projection remains internally consistent");

    auto assignment_source = copied.to_c();
    const auto* assignment_source_events = assignment_source.value().events;
    auto move_assigned = copied.to_c();
    const auto* replaced_events = move_assigned.value().events;
    move_assigned = std::move(assignment_source);
    check(move_assigned.value().event_count == 1 &&
              move_assigned.value().events == assignment_source_events &&
              move_assigned.value().events != replaced_events,
          "move assignment releases its old array and rebinds the transferred one");
    check((assignment_source.value().event_count == 0 &&
           assignment_source.value().events == nullptr) ||
              (assignment_source.value().event_count != 0 &&
               assignment_source.value().events != nullptr),
          "the move-assigned source remains internally consistent");

    auto self_moved = copied.to_c();
    const auto* self_events = self_moved.value().events;
    const auto* self_deliveries = self_moved.value().deliveries;
#if defined(__clang__)
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wself-move"
#elif defined(__GNUC__)
#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wself-move"
#endif
    self_moved = std::move(self_moved);
#if defined(__clang__)
#pragma clang diagnostic pop
#elif defined(__GNUC__)
#pragma GCC diagnostic pop
#endif
    check(self_moved.value().event_count == 1 &&
              self_moved.value().events == self_events &&
              self_moved.value().events[0].text.data != nullptr &&
              std::string_view(self_moved.value().events[0].text.data,
                               self_moved.value().events[0].text.len) ==
                  "copied text" &&
              self_moved.value().delivery_count == 2 &&
              self_moved.value().deliveries == self_deliveries,
          "self-move assignment preserves the complete call-local C projection");

    const void* concurrent_events[2] = {nullptr, nullptr};
    const void* concurrent_deliveries[2] = {nullptr, nullptr};
    const char* concurrent_text_data[2] = {nullptr, nullptr};
    bool concurrent_text[2] = {false, false};
    bool concurrent_distinct[2] = {false, false};
    std::atomic<int> ready{0};
    std::atomic<int> compared{0};
    const auto project_concurrently =
        [&concurrent_events, &concurrent_deliveries, &concurrent_text_data,
         &concurrent_text, &concurrent_distinct, &ready,
         &compared](madopilot::InputRequest request, std::size_t index) {
            const auto view = request.to_c();
            concurrent_events[index] = view.value().events;
            concurrent_deliveries[index] = view.value().deliveries;
            concurrent_text_data[index] =
                view.value().event_count == 1 && view.value().events != nullptr
                    ? view.value().events[0].text.data
                    : nullptr;
            concurrent_text[index] =
                view.value().event_count == 1 &&
                view.value().events != nullptr &&
                std::string_view(view.value().events[0].text.data,
                                 view.value().events[0].text.len) ==
                    "copied text";
            ready.fetch_add(1, std::memory_order_acq_rel);
            while (ready.load(std::memory_order_acquire) != 2) {
                std::this_thread::yield();
            }
            concurrent_distinct[index] =
                concurrent_events[0] != nullptr &&
                concurrent_events[1] != nullptr &&
                concurrent_events[0] != concurrent_events[1] &&
                concurrent_deliveries[0] != nullptr &&
                concurrent_deliveries[1] != nullptr &&
                concurrent_deliveries[0] != concurrent_deliveries[1] &&
                concurrent_text_data[0] != nullptr &&
                concurrent_text_data[1] != nullptr &&
                concurrent_text_data[0] != concurrent_text_data[1];
            compared.fetch_add(1, std::memory_order_acq_rel);
            while (compared.load(std::memory_order_acquire) != 2) {
                std::this_thread::yield();
            }
        };
    std::thread first_projection(project_concurrently, copied, std::size_t{0});
    std::thread second_projection(project_concurrently, copied, std::size_t{1});
    first_projection.join();
    second_projection.join();
    check(concurrent_distinct[0] && concurrent_distinct[1] &&
              concurrent_text[0] && concurrent_text[1],
          "concurrent immutable request copies own distinct unchanged projections");

    const auto after_concurrency = copied.to_c();
    check(after_concurrency.value().event_count == 1 &&
              std::string_view(after_concurrency.value().events[0].text.data,
                               after_concurrency.value().events[0].text.len) ==
                  "copied text",
          "concurrent projection leaves the reusable request unchanged");

    madopilot::InputOpenRequest policy;
    policy.requirement(MADOPILOT_INPUT_REQUIRED)
        .require_pairs(MADOPILOT_INPUT_PAIR_POINTER_PROCESS_DIRECTED)
        .prefer_pairs(MADOPILOT_INPUT_PAIR_KEYBOARD_SYSTEM);
    madopilot::OpenRequest open;
    open.input(policy);
    policy.requirement(MADOPILOT_INPUT_OPTIONAL).require_pairs(0);

    madopilot::OpenRequest open_copy = open;
    const auto refused =
        fixture.engine.open_session(fixture.targets, 0, open_copy, fixture.operation);
    check(!refused && refused.status() == MADOPILOT_STATUS_UNSUPPORTED,
          "the copied open request keeps its required input policy");
    if (!refused) {
        check(refused.error().category() == MADOPILOT_ERROR_CATEGORY_INPUT,
              "a required-input open refusal keeps the input error category");
    }
}

/// Every owner remembers the negotiated table prefix. A full ABI 1.2 library
/// loaded through the frozen 1.0 extent must refuse every 1.2 wrapper method
/// without reading the appended slots.
void an_old_table_extent_hides_every_abi_1_2_entry(Fixture& fixture)
{
    auto loaded = madopilot::Api::load(
        MADOPILOT_ABI_MAJOR, 0, MADOPILOT_API_SIZE_ABI_1_0);
    if (!check_ok(loaded, "load the frozen 1.0 extent")) {
        return;
    }
    madopilot::Api api = loaded.take();
    check(api.extent() == MADOPILOT_API_SIZE_ABI_1_0,
          "the wrapper records the caller-known extent");

    madopilot::ReplayFrame supplied;
    supplied.extent(SCENE_WIDTH, SCENE_HEIGHT)
        .format(MADOPILOT_PIXEL_FORMAT_RGBA8)
        .continuity(MADOPILOT_CONTINUITY_CONTINUOUS)
        .pixels(fixture.scene.data(), fixture.scene.size());
    auto source = madopilot::Source::replay_memory("old-prefix");
    source.frame(supplied);

    auto built = api.create_engine(source, fixture.operation);
    if (!check_ok(built, "create an engine through the old prefix")) {
        return;
    }
    madopilot::Engine engine = built.take();

    const auto capabilities = engine.capabilities();
    check(!capabilities && capabilities.status() == MADOPILOT_STATUS_UNSUPPORTED,
          "an old-prefix engine cannot read engine_capabilities");
    madopilot::Engine cloned = engine.clone();
    check(cloned.extent() == MADOPILOT_API_SIZE_ABI_1_0,
          "a clone preserves the negotiated extent");
    const auto permission = cloned.permission(
        MADOPILOT_PERMISSION_KIND_SCREEN_CAPTURE, fixture.operation);
    check(!permission && permission.status() == MADOPILOT_STATUS_UNSUPPORTED,
          "an old-prefix clone cannot read engine_permission");

    auto discovered = engine.discover(fixture.operation);
    if (!check_ok(discovered, "discover through the old prefix")) {
        return;
    }
    madopilot::TargetList targets = discovered.take();
    const auto input_capability = targets.input_capability(
        0, MADOPILOT_INPUT_OPERATION_POINTER, MADOPILOT_INPUT_DELIVERY_SYSTEM);
    check(!input_capability &&
              input_capability.status() == MADOPILOT_STATUS_UNSUPPORTED,
          "an old-prefix child cannot read target_list_input_capability");
    const auto input_descriptor =
        engine.input_descriptor(targets, 0, fixture.operation);
    check(!input_descriptor &&
              input_descriptor.status() == MADOPILOT_STATUS_UNSUPPORTED,
          "an old-prefix engine cannot read engine_input_descriptor");

    madopilot::InputOpenRequest required_input;
    required_input.requirement(MADOPILOT_INPUT_REQUIRED)
        .require_pairs(MADOPILOT_INPUT_PAIR_POINTER_SYSTEM);
    madopilot::OpenRequest input_open;
    input_open.input(required_input);
    const auto input_session =
        engine.open_session(targets, 0, input_open, fixture.operation);
    check(!input_session &&
              input_session.status() == MADOPILOT_STATUS_UNSUPPORTED,
          "an old-prefix engine cannot call session_open_with_input");

    madopilot::OpenRequest open;
    open.require_format(MADOPILOT_PIXEL_FORMAT_RGBA8);
    auto opened = engine.open_session(targets, 0, open, fixture.operation);
    if (!check_ok(opened, "open through the old prefix")) {
        return;
    }
    madopilot::Session session = opened.take();
    const auto accepted = session.input_descriptor();
    check(!accepted && accepted.status() == MADOPILOT_STATUS_UNSUPPORTED,
          "an old-prefix session cannot read session_input_descriptor");

    madopilot::InputRequest request;
    request.event(madopilot::InputEvent::delay(1))
        .delivery(MADOPILOT_INPUT_DELIVERY_SYSTEM);
    const auto receipt = session.send_input(request, fixture.operation);
    check(!receipt && receipt.status() == MADOPILOT_STATUS_UNSUPPORTED,
          "an old-prefix session cannot call session_send_input");
}

/// An ABI 1.2 prefix must include every lifecycle/accessor entry needed by an
/// owner before the wrapper invokes the C entry that could return that owner.
void partial_abi_1_2_owner_prefixes_are_refused(Fixture& fixture)
{
    auto receipt_loaded = madopilot::Api::load(
        MADOPILOT_ABI_MAJOR, MADOPILOT_ABI_MINOR,
        MADOPILOT_API_SIZE_SESSION_SEND_INPUT);
    if (!check_ok(receipt_loaded, "load the partial receipt prefix")) {
        return;
    }
    madopilot::Api receipt_api = receipt_loaded.take();

    madopilot::ReplayFrame supplied;
    supplied.extent(SCENE_WIDTH, SCENE_HEIGHT)
        .format(MADOPILOT_PIXEL_FORMAT_RGBA8)
        .continuity(MADOPILOT_CONTINUITY_CONTINUOUS)
        .pixels(fixture.scene.data(), fixture.scene.size());
    auto receipt_source = madopilot::Source::replay_memory("partial-receipt");
    receipt_source.frame(supplied);
    auto receipt_engine_result =
        receipt_api.create_engine(receipt_source, fixture.operation);
    if (!check_ok(receipt_engine_result, "create a partial-prefix engine")) {
        return;
    }
    madopilot::Engine receipt_engine = receipt_engine_result.take();
    auto targets_result = receipt_engine.discover(fixture.operation);
    if (!check_ok(targets_result, "discover through the partial receipt prefix")) {
        return;
    }
    madopilot::TargetList targets = targets_result.take();
    madopilot::OpenRequest open;
    open.require_format(MADOPILOT_PIXEL_FORMAT_RGBA8);
    auto session_result =
        receipt_engine.open_session(targets, 0, open, fixture.operation);
    if (!check_ok(session_result, "open through the partial receipt prefix")) {
        return;
    }
    madopilot::Session session = session_result.take();
    madopilot::InputRequest request;
    request.event(madopilot::InputEvent::delay(1))
        .delivery(MADOPILOT_INPUT_DELIVERY_SYSTEM);
    const auto receipt = session.send_input(request, fixture.operation);
    check(!receipt && receipt.status() == MADOPILOT_STATUS_UNSUPPORTED,
          "a partial receipt prefix is refused before session_send_input");
    if (!receipt) {
        check(receipt.error().category() ==
                  MADOPILOT_ERROR_CATEGORY_UNSPECIFIED,
              "the partial receipt refusal comes from the wrapper");
    }

    auto diagnostic_loaded = madopilot::Api::load(
        MADOPILOT_ABI_MAJOR, MADOPILOT_ABI_MINOR,
        MADOPILOT_API_SIZE_ENGINE_TAKE_DIAGNOSTIC_READER);
    if (!check_ok(diagnostic_loaded, "load the partial diagnostic prefix")) {
        return;
    }
    madopilot::Api diagnostic_api = diagnostic_loaded.take();
    auto diagnostic_source = madopilot::Source::replay_memory("partial-diagnostic");
    diagnostic_source.frame(supplied);
    madopilot::EngineOptions options;
    options.diagnostics(MADOPILOT_DIAGNOSTIC_LEVEL_NORMAL, 8);
    auto diagnostic_engine_result =
        diagnostic_api.create_engine(diagnostic_source, options, fixture.operation);
    if (!check_ok(diagnostic_engine_result, "create a partial diagnostic engine")) {
        return;
    }
    madopilot::Engine diagnostic_engine = diagnostic_engine_result.take();
    const auto reader = diagnostic_engine.take_diagnostic_reader();
    check(!reader && reader.status() == MADOPILOT_STATUS_UNSUPPORTED,
          "a partial diagnostic prefix is refused before returning a reader");
    if (!reader) {
        check(reader.error().category() ==
                  MADOPILOT_ERROR_CATEGORY_UNSPECIFIED,
              "the partial diagnostic refusal comes from the wrapper");
    }
}

/// Diagnostics default to off and therefore expose no reader.
void diagnostics_off_has_no_reader(Fixture& fixture)
{
    auto reader = fixture.engine.take_diagnostic_reader();
    if (check_ok(reader, "take the diagnostics-off reader")) {
        check(!reader.value().has_value(),
              "the default engine allocated no diagnostic consumer");
    }
}

/// A pull reader and its immutable batches remain valid after their engine.
void diagnostic_reader_outlives_its_engine(Fixture& fixture)
{
    madopilot::ReplayFrame supplied;
    supplied.extent(SCENE_WIDTH, SCENE_HEIGHT)
        .format(MADOPILOT_PIXEL_FORMAT_RGBA8)
        .continuity(MADOPILOT_CONTINUITY_CONTINUOUS)
        .pixels(fixture.scene.data(), fixture.scene.size());
    auto source = madopilot::Source::replay_memory("diagnostics");
    source.frame(supplied);

    madopilot::EngineOptions options;
    options.diagnostics(MADOPILOT_DIAGNOSTIC_LEVEL_DEBUG, 16);
    madopilot::Operation operation = fixture.operation;
    constexpr std::uint64_t activity = UINT64_C(0x5a17);
    operation.activity_tag(activity);
    auto built = fixture.api.create_engine(source, options, operation);
    if (!check_ok(built, "create a diagnostic engine")) {
        return;
    }
    madopilot::Engine engine = built.take();

    auto taken = engine.take_diagnostic_reader();
    if (!check_ok(taken, "take the diagnostic reader") ||
        !check(taken.value().has_value(), "an enabled engine exposes one reader")) {
        return;
    }
    auto reader_value = taken.take();
    madopilot::DiagnosticReader reader = std::move(*reader_value);

    const auto repeated = engine.take_diagnostic_reader();
    if (check_ok(repeated, "take the reader a second time")) {
        check(!repeated.value().has_value(),
              "the engine exposes exactly one diagnostic reader");
    }

    auto discovered = engine.discover(operation);
    if (!check_ok(discovered, "produce a correlated diagnostic")) {
        return;
    }
    madopilot::TargetList targets = discovered.take();
    targets.reset();
    engine.reset();

    auto drained = reader.drain();
    if (!check_ok(drained, "drain after engine release")) {
        return;
    }
    madopilot::DiagnosticDrain drain = drained.take();
    if (!check(drain.state == MADOPILOT_DIAGNOSTIC_DRAIN_BATCH &&
                   drain.batch.has_value(),
               "sealed retained records drain as an owned batch")) {
        return;
    }
    madopilot::DiagnosticBatch batch = std::move(*drain.batch);
    const auto info = batch.describe();
    if (!check_ok(info, "describe the diagnostic batch") ||
        !check(info.value().record_count != 0,
               "debug discovery retained at least one record")) {
        return;
    }
    const auto record = batch.record_at(0);
    if (check_ok(record, "read the first diagnostic record")) {
        check(record.value().operation_id != 0 &&
                  record.value().has(MADOPILOT_DIAGNOSTIC_RECORD_HAS_ACTIVITY) &&
                  record.value().activity_tag == activity,
              "the record carries checked operation identity and caller activity");
    }

    batch.reset();
    const auto ended = reader.drain();
    if (check_ok(ended, "drain the sealed empty reader")) {
        check(ended.value().state == MADOPILOT_DIAGNOSTIC_DRAIN_END_OF_STREAM &&
                  !ended.value().batch.has_value(),
              "draining is self-silent and reaches sealed-empty");
    }
}

/// Close is explicit, idempotent, and reports its outcome to the caller.
void close_reports_its_outcome(Fixture& fixture)
{
    madopilot::Session session = fixture.open_another();
    if (session.empty()) {
        return;
    }

    check_ok(session.close(fixture.operation), "the first close");
    check_ok(session.close(fixture.operation), "the second close is idempotent");

    const auto closed = session.is_closed();
    if (check_ok(closed, "is_closed")) {
        check(closed.value(), "the session reports itself closed");
    }

    const auto after = session.acquire_frame(fixture.operation);
    check(!after && after.status() == MADOPILOT_STATUS_CLOSED,
          "a closed session starts no further work");

    // A close that cannot reach the library reports that, rather than throwing
    // or being swallowed by a destructor. The C boundary produces the status.
    const madopilot::Session moved = std::move(session);
    // NOLINTNEXTLINE(bugprone-use-after-move)
    const auto orphaned = session.close(fixture.operation);
    check(!orphaned && orphaned.status() == MADOPILOT_STATUS_INVALID_ARGUMENT,
          "closing an emptied owner is refused rather than throwing");

    // The still-owning clone remains usable, and its destructor releases it.
    check_ok(moved.is_closed(), "the surviving owner still answers");
}

/// Several threads read one immutable result through explicit clones.
void concurrent_readers_use_explicit_clones(Fixture& fixture)
{
    madopilot::FindRequest find;
    find.frame(fixture.frame).search_for(fixture.present);

    auto found = fixture.session.find(find, fixture.operation);
    if (!check_ok(found, "find")) {
        return;
    }
    const madopilot::MatchResult result = found.take();

    constexpr int readers = 8;
    std::vector<std::thread> threads;
    std::vector<int> counts(readers, 0);
    threads.reserve(readers);

    for (int index = 0; index < readers; ++index) {
        // Each thread gets its own owner, so no thread depends on another's
        // lifetime. Releasing the last reference concurrently with an
        // unprotected call is what this avoids.
        madopilot::MatchResult mine = result.clone();
        threads.emplace_back([mine = std::move(mine), &counts, index]() mutable {
            for (int round = 0; round < 64; ++round) {
                const auto info = mine.describe();
                if (info && info.value().match_count == 2) {
                    counts[static_cast<std::size_t>(index)] += 1;
                }
            }
        });
    }
    for (std::thread& thread : threads) {
        thread.join();
    }

    for (int index = 0; index < readers; ++index) {
        check(counts[static_cast<std::size_t>(index)] == 64,
              "every reader saw the same immutable result");
    }

    // The original still owns its own reference after every clone is gone.
    check_ok(result.describe(), "the original outlives every clone");
}

void ocr_request_projections_rebind_after_every_copy_and_move(Fixture&)
{
    const std::string model(96, 'm');
    const std::string backend(96, 'b');
    const std::string version(96, 'v');
    madopilot::OcrRequest request;
    request.model(model).backend(backend, version);

    auto original = request.to_c();
    auto copied = original;
    madopilot::OcrRequest::CView copy_assigned(request);
    copy_assigned = original;
    check(copied.value().model_id.data != original.value().model_id.data,
          "copy construction rebinds OCR model storage");
    check(copy_assigned.value().model_id.data != original.value().model_id.data,
          "copy assignment rebinds OCR model storage");
    auto moved = std::move(copy_assigned);
    madopilot::OcrRequest::CView move_assigned(request);
    move_assigned = std::move(copied);
    check(copy_assigned.value().model_id.data != moved.value().model_id.data,
          "move construction rebinds the moved-from OCR projection");
    check(copied.value().model_id.data != move_assigned.value().model_id.data,
          "move assignment rebinds the moved-from OCR projection");

    const auto equals = [](madopilot_str_t view, const std::string& expected) {
        return std::string_view(view.data, view.len) == expected;
    };
    check(equals(original.value().model_id, model) &&
              equals(moved.value().model_id, model) &&
              equals(move_assigned.value().model_id, model),
          "copy and move preserve every OCR string value");
    check(original.value().model_id.data != moved.value().model_id.data &&
              original.value().model_id.data != move_assigned.value().model_id.data &&
              moved.value().model_id.data != move_assigned.value().model_id.data,
          "each OCR projection points into its own model storage");
    check(original.value().backend_id.data != moved.value().backend_id.data &&
              original.value().backend_version.data != moved.value().backend_version.data,
          "backend ID and version views are independently rebound");

    auto first = request.to_c();
    auto second = request.to_c();
    std::atomic<bool> both_valid{true};
    std::thread one([&] {
        both_valid.store(equals(first.value().model_id, model),
                         std::memory_order_release);
    });
    std::thread two([&] {
        if (!equals(second.value().model_id, model)) {
            both_valid.store(false, std::memory_order_release);
        }
    });
    one.join();
    two.join();
    check(both_valid.load(std::memory_order_acquire) &&
              first.value().model_id.data != second.value().model_id.data,
          "concurrent immutable projections have distinct stable C records");
}

void profile_and_zone_projections_rebind_after_every_copy_and_move(
    Fixture& fixture)
{
    const std::string model_root(96, 'r');
    const std::string runtime_path(96, 't');
    madopilot::OcrProfileOptions profile(
        MADOPILOT_OCR_PROFILE_BOUNDED_DETECTOR, model_root, runtime_path);
    auto profile_original = profile.to_c();
    auto profile_copied = profile_original;
    madopilot::OcrProfileOptions::CView profile_copy_assigned(profile);
    profile_copy_assigned = profile_original;
    auto profile_moved = std::move(profile_copy_assigned);
    madopilot::OcrProfileOptions::CView profile_move_assigned(profile);
    profile_move_assigned = std::move(profile_copied);
    check(profile_copy_assigned.value().model_root.data !=
                  profile_moved.value().model_root.data &&
              profile_copy_assigned.value().runtime_path.data !=
                  profile_moved.value().runtime_path.data,
          "profile move construction rebinds the moved-from projection");
    check(profile_copied.value().model_root.data !=
                  profile_move_assigned.value().model_root.data &&
              profile_copied.value().runtime_path.data !=
                  profile_move_assigned.value().runtime_path.data,
          "profile move assignment rebinds the moved-from projection");
    profile = madopilot::OcrProfileOptions(
        MADOPILOT_OCR_PROFILE_BOUNDED_DETECTOR, "mutated", "mutated");

    const auto equals = [](madopilot_str_t view, const std::string& expected) {
        return std::string_view(view.data, view.len) == expected;
    };
    check(equals(profile_original.value().model_root, model_root) &&
              equals(profile_moved.value().runtime_path, runtime_path) &&
              equals(profile_move_assigned.value().model_root, model_root),
          "profile projections own path values after source mutation");
    check(profile_original.value().model_root.data !=
                  profile_moved.value().model_root.data &&
              profile_original.value().runtime_path.data !=
                  profile_move_assigned.value().runtime_path.data,
          "profile copy and move rebind both path views");

    const std::string provider_root(96, 'c');
    madopilot::OcrProviderOptions provider(
        MADOPILOT_OCR_PROVIDER_POLICY_REQUIRE_CUDA, provider_root);
    auto provider_original = provider.to_c();
    auto provider_copied = provider_original;
    madopilot::OcrProviderOptions::CView provider_copy_assigned(provider);
    provider_copy_assigned = provider_original;
    auto provider_moved = std::move(provider_copy_assigned);
    madopilot::OcrProviderOptions::CView provider_move_assigned(provider);
    provider_move_assigned = std::move(provider_copied);
    check(provider_copy_assigned.value().provider_root.data !=
                  provider_moved.value().provider_root.data,
          "provider move construction rebinds the moved-from projection");
    check(provider_copied.value().provider_root.data !=
                  provider_move_assigned.value().provider_root.data,
          "provider move assignment rebinds the moved-from projection");
    provider = madopilot::OcrProviderOptions(
        MADOPILOT_OCR_PROVIDER_POLICY_CPU, "mutated");
    check(equals(provider_original.value().provider_root, provider_root) &&
              equals(provider_moved.value().provider_root, provider_root) &&
              equals(provider_move_assigned.value().provider_root,
                     provider_root),
          "provider projections own the root after source mutation");
    check(provider_original.value().provider_root.data !=
                  provider_moved.value().provider_root.data &&
              provider_original.value().provider_root.data !=
                  provider_move_assigned.value().provider_root.data,
          "provider copy and move rebind every root view");

    std::vector<madopilot::OcrProviderOptions::CView>
        relocated_provider_views;
    for (int index = 0; index < 16; ++index) {
        relocated_provider_views.push_back(provider_original);
    }
    bool relocated_valid = true;
    for (const auto& projection : relocated_provider_views) {
        relocated_valid =
            relocated_valid &&
            equals(projection.value().provider_root, provider_root) &&
            projection.value().provider_root.data !=
                provider_original.value().provider_root.data;
    }
    check(relocated_valid,
          "container relocation preserves independently rebound provider roots");

    const std::string model(96, 'm');
    const std::string backend(96, 'b');
    const std::string version(96, 'v');
    madopilot::ZoneScanOcrRequest request;
    request.frame(fixture.frame)
        .package(fixture.package)
        .model(model)
        .backend(backend, version);
    for (std::int32_t index = 0; index < 8; ++index) {
        request.zone(madopilot::Rect{
            MADOPILOT_SPACE_CAPTURE_PIXELS, index * 10, 0, index * 10 + 8, 8});
    }

    auto original = request.to_c();
    auto copied = original;
    madopilot::ZoneScanOcrRequest::CView copy_assigned(request);
    copy_assigned = original;
    auto moved = std::move(copy_assigned);
    madopilot::ZoneScanOcrRequest::CView move_assigned(request);
    move_assigned = std::move(copied);
    check(copy_assigned.value().zones != moved.value().zones &&
              copy_assigned.value().zone_stride ==
                  sizeof(madopilot_ocr_zone_t) &&
              (copy_assigned.value().zone_count == 0 ||
               copy_assigned.value().zones != nullptr) &&
              copy_assigned.value().model_id.data !=
                  moved.value().model_id.data,
          "zone move construction rebinds the moved-from projection");
    check(copied.value().zones != move_assigned.value().zones &&
              copied.value().zone_stride == sizeof(madopilot_ocr_zone_t) &&
              (copied.value().zone_count == 0 ||
               copied.value().zones != nullptr) &&
              copied.value().backend_id.data !=
                  move_assigned.value().backend_id.data,
          "zone move assignment rebinds the moved-from projection");
    request.clear_zones().model("mutated").backend("mutated", "mutated");

    check(original.value().zone_count == 8 && moved.value().zone_count == 8 &&
              move_assigned.value().zone_count == 8,
          "copy and move preserve all eight zone records");
    check(original.value().zone_stride == sizeof(madopilot_ocr_zone_t) &&
              moved.value().zone_stride == sizeof(madopilot_ocr_zone_t),
          "every projection repairs the exact zone stride");
    check(original.value().zones != moved.value().zones &&
              original.value().zones != move_assigned.value().zones &&
              moved.value().zones != move_assigned.value().zones,
          "each grouped projection owns distinct zone storage");
    check(equals(original.value().model_id, model) &&
              equals(moved.value().backend_id, backend) &&
              equals(move_assigned.value().backend_version, version),
          "grouped projections own identity strings after source mutation");
    check(original.value().model_id.data != moved.value().model_id.data &&
              original.value().backend_id.data !=
                  move_assigned.value().backend_id.data,
          "grouped copy and move rebind every identity view");
    check(original.value().zones[7].region.left == 70 &&
              original.value().zones[7].region.right == 78,
          "zone geometry remains caller ordered after projection moves");

    auto first = original;
    auto second = original;
    std::atomic<bool> both_valid{true};
    std::thread one([&] {
        both_valid.store(
            first.value().zones != nullptr && first.value().zone_count == 8 &&
                equals(first.value().model_id, model),
            std::memory_order_release);
    });
    std::thread two([&] {
        if (second.value().zones == nullptr ||
            second.value().zone_count != 8 ||
            !equals(second.value().backend_version, version)) {
            both_valid.store(false, std::memory_order_release);
        }
    });
    one.join();
    two.join();
    check(both_valid.load(std::memory_order_acquire) &&
              first.value().zones != second.value().zones &&
              first.value().model_id.data != second.value().model_id.data,
          "concurrent grouped projections own distinct stable C records");
}

void projection_copy_assignment_preserves_the_destination_on_allocation_failure(
    Fixture&)
{
    const std::string old_model_root(96, 'o');
    const std::string old_runtime_path(96, 'p');
    const std::string new_model_root(192, 'r');
    const std::string new_runtime_path(192, 't');
    madopilot::OcrProfileOptions old_profile(
        MADOPILOT_OCR_PROFILE_BOUNDED_DETECTOR, old_model_root,
        old_runtime_path);
    madopilot::OcrProfileOptions new_profile(
        MADOPILOT_OCR_PROFILE_BOUNDED_DETECTOR, new_model_root,
        new_runtime_path);
    auto profile_destination = old_profile.to_c();
    const auto profile_source = new_profile.to_c();

    bool profile_threw = false;
    allocations_before_failure.store(1, std::memory_order_relaxed);
    try {
        profile_destination = profile_source;
    } catch (const std::bad_alloc&) {
        profile_threw = true;
    }
    allocations_before_failure.store(-1, std::memory_order_relaxed);

    const auto equals = [](madopilot_str_t view, const std::string& expected) {
        return std::string_view(view.data, view.len) == expected;
    };
    check(profile_threw &&
              equals(profile_destination.value().model_root, old_model_root) &&
              equals(profile_destination.value().runtime_path, old_runtime_path),
          "profile copy assignment leaves its C projection unchanged on bad_alloc");

    const std::string old_model(96, 'm');
    const std::string old_backend(96, 'b');
    const std::string old_version(96, 'v');
    const std::string new_model(192, 'M');
    const std::string new_backend(192, 'B');
    const std::string new_version(192, 'V');
    madopilot::ZoneScanOcrRequest old_request;
    old_request.model(old_model)
        .backend(old_backend, old_version)
        .zone(madopilot::Rect{MADOPILOT_SPACE_CAPTURE_PIXELS, 1, 2, 3, 4});
    madopilot::ZoneScanOcrRequest new_request;
    new_request.model(new_model).backend(new_backend, new_version);
    for (std::int32_t index = 0; index < 8; ++index) {
        new_request.zone(madopilot::Rect{
            MADOPILOT_SPACE_CAPTURE_PIXELS, index, 0, index + 1, 1});
    }
    auto request_destination = old_request.to_c();
    const auto request_source = new_request.to_c();

    bool request_threw = false;
    allocations_before_failure.store(1, std::memory_order_relaxed);
    try {
        request_destination = request_source;
    } catch (const std::bad_alloc&) {
        request_threw = true;
    }
    allocations_before_failure.store(-1, std::memory_order_relaxed);

    check(request_threw &&
              equals(request_destination.value().model_id, old_model) &&
              equals(request_destination.value().backend_id, old_backend) &&
              equals(request_destination.value().backend_version, old_version) &&
              request_destination.value().zone_count == 1 &&
              request_destination.value().zones != nullptr &&
              request_destination.value().zones[0].region.left == 1 &&
              request_destination.value().zones[0].region.bottom == 4,
          "zone copy assignment leaves its C projection unchanged on bad_alloc");
}

madopilot::Result<madopilot::Session> prefix_session(
    madopilot::Api& api, Fixture& fixture, std::string_view name)
{
    madopilot::ReplayFrame supplied;
    supplied.extent(SCENE_WIDTH, SCENE_HEIGHT)
        .format(MADOPILOT_PIXEL_FORMAT_RGBA8)
        .continuity(MADOPILOT_CONTINUITY_CONTINUOUS)
        .pixels(fixture.scene.data(), fixture.scene.size());
    auto source = madopilot::Source::replay_memory(name);
    source.frame(supplied);
    auto built = api.create_engine(source, fixture.operation);
    if (!built) {
        return madopilot::Result<madopilot::Session>::failure(built.error());
    }
    madopilot::Engine engine = built.take();
    auto discovered = engine.discover(fixture.operation);
    if (!discovered) {
        return madopilot::Result<madopilot::Session>::failure(discovered.error());
    }
    madopilot::TargetList targets = discovered.take();
    madopilot::OpenRequest open;
    open.require_format(MADOPILOT_PIXEL_FORMAT_RGBA8);
    return engine.open_session(targets, 0, open, fixture.operation);
}

void old_and_partial_extents_hide_ocr_before_missing_entries(Fixture& fixture)
{
    auto old_loaded = madopilot::Api::load(
        MADOPILOT_ABI_MAJOR, 2,
        MADOPILOT_API_SIZE_DIAGNOSTIC_BATCH_RECORD_AT);
    if (!check_ok(old_loaded, "load the complete ABI 1.2 extent")) {
        return;
    }
    madopilot::Api old_api = old_loaded.take();
    auto old_session_result = prefix_session(old_api, fixture, "abi-1.2-ocr");
    if (!check_ok(old_session_result, "open through the ABI 1.2 extent")) {
        return;
    }
    madopilot::Session old_session = old_session_result.take();
    madopilot::OcrRequest request;
    const auto old_refusal = old_session.recognize(request, fixture.operation);
    check(!old_refusal && old_refusal.status() == MADOPILOT_STATUS_UNSUPPORTED,
          "ABI 1.2 refuses OCR before reading session_recognize");

    auto partial_loaded = madopilot::Api::load(
        MADOPILOT_ABI_MAJOR, MADOPILOT_ABI_MINOR,
        MADOPILOT_API_SIZE_SESSION_RECOGNIZE);
    if (!check_ok(partial_loaded, "load a partial ABI 1.3 OCR extent")) {
        return;
    }
    madopilot::Api partial_api = partial_loaded.take();
    auto partial_session_result =
        prefix_session(partial_api, fixture, "partial-abi-1.3-ocr");
    if (!check_ok(partial_session_result, "open through partial ABI 1.3")) {
        return;
    }
    madopilot::Session partial_session = partial_session_result.take();
    const auto partial_refusal =
        partial_session.recognize(request, fixture.operation);
    check(!partial_refusal &&
              partial_refusal.status() == MADOPILOT_STATUS_UNSUPPORTED,
          "partial ABI 1.3 refuses OCR before reading a missing owner entry");

    madopilot::ReplayFrame supplied;
    supplied.extent(SCENE_WIDTH, SCENE_HEIGHT)
        .format(MADOPILOT_PIXEL_FORMAT_RGBA8)
        .continuity(MADOPILOT_CONTINUITY_CONTINUOUS)
        .pixels(fixture.scene.data(), fixture.scene.size());
    auto source = madopilot::Source::replay_memory("partial-abi-1.4-profile");
    source.frame(supplied);
    madopilot::EngineOptions options;
    madopilot::OcrProfileOptions profile(
        MADOPILOT_OCR_PROFILE_BOUNDED_DETECTOR, "", "");

    auto no_profile_entry_loaded = madopilot::Api::load(
        MADOPILOT_ABI_MAJOR, MADOPILOT_ABI_MINOR,
        MADOPILOT_API_SIZE_ENGINE_CREATE_WITH_DEFAULT_OCR);
    if (!check_ok(no_profile_entry_loaded,
                  "load the complete ABI 1.3 extent")) {
        return;
    }
    madopilot::Api no_profile_entry_api = no_profile_entry_loaded.take();
    const auto no_profile_entry =
        no_profile_entry_api.create_engine_with_ocr_profile(
            source, options, profile, fixture.operation);
    check(!no_profile_entry &&
              no_profile_entry.status() == MADOPILOT_STATUS_UNSUPPORTED,
          "complete ABI 1.3 refuses profile construction before its entry");

    auto profile_entry_loaded = madopilot::Api::load(
        MADOPILOT_ABI_MAJOR, MADOPILOT_ABI_MINOR,
        MADOPILOT_API_SIZE_ENGINE_CREATE_WITH_OCR_PROFILE);
    if (!check_ok(profile_entry_loaded,
                  "load the complete profile-construction entry")) {
        return;
    }
    madopilot::Api profile_entry_api = profile_entry_loaded.take();
    const auto current_profile_call =
        profile_entry_api.create_engine_with_ocr_profile(
            source, options, profile, fixture.operation);
    check(!current_profile_call &&
              current_profile_call.status() ==
                  MADOPILOT_STATUS_INVALID_ARGUMENT,
          "complete profile entry reaches C validation rather than fallback");

    auto partial_zone_loaded = madopilot::Api::load(
        MADOPILOT_ABI_MAJOR, MADOPILOT_ABI_MINOR,
        MADOPILOT_API_SIZE_SESSION_SCAN_OCR_ZONES);
    if (!check_ok(partial_zone_loaded,
                  "load a grouped scan without its owner suffix")) {
        return;
    }
    madopilot::Api partial_zone_api = partial_zone_loaded.take();
    auto partial_zone_session_result =
        prefix_session(partial_zone_api, fixture, "partial-abi-1.4-zone");
    if (!check_ok(partial_zone_session_result,
                  "open through partial ABI 1.4")) {
        return;
    }
    madopilot::Session partial_zone_session =
        partial_zone_session_result.take();
    madopilot::ZoneScanOcrRequest zone_request;
    const auto partial_zone_refusal =
        partial_zone_session.scan_ocr_zones(zone_request, fixture.operation);
    check(!partial_zone_refusal &&
              partial_zone_refusal.status() == MADOPILOT_STATUS_UNSUPPORTED,
          "partial ABI 1.4 refuses grouped OCR before a missing owner entry");

    auto final_partial_loaded = madopilot::Api::load(
        MADOPILOT_ABI_MAJOR, MADOPILOT_ABI_MINOR,
        MADOPILOT_API_SIZE_OCR_ZONE_SCAN_RESULT_REGION_AT);
    if (!check_ok(final_partial_loaded,
                  "load ABI 1.4 through the final incomplete owner extent")) {
        return;
    }
    madopilot::Api final_partial_api = final_partial_loaded.take();
    auto final_partial_session_result =
        prefix_session(final_partial_api, fixture, "final-partial-abi-1.4-zone");
    if (!check_ok(final_partial_session_result,
                  "open through the final incomplete ABI 1.4 extent")) {
        return;
    }
    madopilot::Session final_partial_session =
        final_partial_session_result.take();
    const auto final_partial_refusal =
        final_partial_session.scan_ocr_zones(zone_request, fixture.operation);
    check(!final_partial_refusal &&
              final_partial_refusal.status() == MADOPILOT_STATUS_UNSUPPORTED,
          "the 704-byte ABI 1.4 extent refuses grouped OCR before text_at");

    auto no_descriptor_loaded = madopilot::Api::load(
        MADOPILOT_ABI_MAJOR, MADOPILOT_ABI_MINOR,
        MADOPILOT_API_SIZE_OCR_ZONE_SCAN_RESULT_TEXT_AT);
    if (!check_ok(no_descriptor_loaded,
                  "load the grouped ABI 1.4 extent without the descriptor")) {
        return;
    }
    madopilot::Api no_descriptor_api = no_descriptor_loaded.take();
    auto no_descriptor_built =
        no_descriptor_api.create_engine(source, fixture.operation);
    if (!check_ok(no_descriptor_built,
                  "build through the pre-descriptor ABI 1.4 extent")) {
        return;
    }
    madopilot::Engine no_descriptor_engine = no_descriptor_built.take();
    const auto no_descriptor = no_descriptor_engine.ocr_descriptor();
    check(!no_descriptor &&
              no_descriptor.status() == MADOPILOT_STATUS_UNSUPPORTED,
          "the 712-byte ABI 1.4 extent refuses the appended descriptor entry");

    madopilot::OcrProviderOptions provider(
        MADOPILOT_OCR_PROVIDER_POLICY_CPU);
    auto no_provider_entry_loaded = madopilot::Api::load(
        MADOPILOT_ABI_MAJOR, MADOPILOT_ABI_MINOR,
        MADOPILOT_API_SIZE_ENGINE_OCR_DESCRIPTOR);
    if (!check_ok(no_provider_entry_loaded,
                  "load the complete ABI 1.4 extent")) {
        return;
    }
    madopilot::Api no_provider_entry_api =
        no_provider_entry_loaded.take();
    const auto no_provider_entry =
        no_provider_entry_api.create_engine_with_ocr_provider(
            source, options, profile, provider, fixture.operation);
    check(!no_provider_entry &&
              no_provider_entry.status() == MADOPILOT_STATUS_UNSUPPORTED,
          "complete ABI 1.4 refuses provider construction before its entry");

    auto provider_entry_loaded = madopilot::Api::load(
        MADOPILOT_ABI_MAJOR, MADOPILOT_ABI_MINOR,
        MADOPILOT_API_SIZE_ENGINE_CREATE_WITH_OCR_PROVIDER);
    if (!check_ok(provider_entry_loaded,
                  "load the complete provider-construction entry")) {
        return;
    }
    madopilot::Api provider_entry_api = provider_entry_loaded.take();
    const auto provider_entry_call =
        provider_entry_api.create_engine_with_ocr_provider(
            source, options, profile, provider, fixture.operation);
    check(!provider_entry_call &&
              provider_entry_call.status() ==
                  MADOPILOT_STATUS_INVALID_ARGUMENT,
          "complete provider entry reaches C validation");

    auto pre_provider_descriptor_built =
        provider_entry_api.create_engine(source, fixture.operation);
    if (!check_ok(pre_provider_descriptor_built,
                  "build through the pre-provider-descriptor extent")) {
        return;
    }
    madopilot::Engine pre_provider_descriptor_engine =
        pre_provider_descriptor_built.take();
    const auto pre_provider_descriptor =
        pre_provider_descriptor_engine.ocr_provider_descriptor();
    check(!pre_provider_descriptor &&
              pre_provider_descriptor.status() ==
                  MADOPILOT_STATUS_UNSUPPORTED,
          "the 728-byte ABI 1.5 extent refuses the missing provider descriptor");

    const auto current_zone_call =
        fixture.session.scan_ocr_zones(zone_request, fixture.operation);
    check(!current_zone_call &&
              current_zone_call.status() == MADOPILOT_STATUS_INVALID_ARGUMENT,
          "complete ABI 1.4 reaches grouped request validation");
}
void run(const char* name, void (*test)(Fixture&), Fixture& fixture)
{
    current = name;
    const int before = failures;
    test(fixture);
    std::printf("%s %s\n", failures == before ? "ok  " : "FAIL", name);
}

} // namespace

int main(int argc, char** argv)

{
    const char* package_path = nullptr;

    for (int index = 1; index < argc; ++index) {
        const std::string_view argument(argv[index]);
        if (argument == "--package" && index + 1 < argc) {
            package_path = argv[++index];
        } else {
            std::fprintf(stderr, "usage: %s --package <dir>\n", argv[0]);
            return 2;
        }
    }
    if (package_path == nullptr) {
        std::fprintf(stderr, "usage: %s --package <dir>\n", argv[0]);
        return 2;
    }

    Fixture fixture;
    current = "fixture";
    if (!fixture.build(package_path)) {
        std::printf("FAIL: the deterministic fixture could not be built\n");
        return 1;
    }

    run("moving an owner transfers it", moving_an_owner_transfers_it, fixture);
    run("move assignment releases the destination",
        move_assignment_releases_the_destination, fixture);
    run("self move assignment keeps the owner intact",
        self_move_assignment_keeps_the_owner_intact, fixture);
    run("a clone outlives its origin", a_clone_outlives_its_origin, fixture);
    run("cloning an empty owner is empty", cloning_an_empty_owner_is_empty, fixture);
    run("a child outlives its parents", a_child_outlives_its_parents, fixture);
    run("a borrowed view tracks its owner", a_borrowed_view_tracks_its_owner, fixture);
    run("error text outlives the C handle", error_text_outlives_the_c_handle, fixture);
    run("a failure leaves no residue", a_failure_leaves_no_residue, fixture);
    run("a throwing copy still releases the error",
        a_throwing_copy_still_releases_the_error, fixture);
    run("zero matches is a success", zero_matches_is_a_success, fixture);
    run("an unsupported coordinate space is refused",
        an_unsupported_coordinate_space_is_refused, fixture);
    run("ABI 1.2 replay capabilities are explicit",
        abi_1_2_replay_capabilities_are_explicit, fixture);
    run("ABI 1.2 process-directed projection is truthful",
        abi_1_2_process_directed_projection_is_truthful, fixture);
    run("ABI 1.2 requests own their storage",
        abi_1_2_requests_own_their_storage, fixture);
    run("an old table extent hides every ABI 1.2 entry",
        an_old_table_extent_hides_every_abi_1_2_entry, fixture);
    run("partial ABI 1.2 owner prefixes are refused",
        partial_abi_1_2_owner_prefixes_are_refused, fixture);
    run("diagnostics off has no reader", diagnostics_off_has_no_reader, fixture);
    run("diagnostic reader outlives its engine",
        diagnostic_reader_outlives_its_engine, fixture);
    run("close reports its outcome", close_reports_its_outcome, fixture);
    run("concurrent readers use explicit clones", concurrent_readers_use_explicit_clones,
        fixture);

    run("OCR request projections rebind after copy and move",
        ocr_request_projections_rebind_after_every_copy_and_move, fixture);
    run("profile and zone projections rebind after copy and move",
        profile_and_zone_projections_rebind_after_every_copy_and_move,
        fixture);
    run("projection copy assignment preserves its destination on bad_alloc",
        projection_copy_assignment_preserves_the_destination_on_allocation_failure,
        fixture);
    run("old and partial extents hide OCR before missing entries",
        old_and_partial_extents_hide_ocr_before_missing_entries, fixture);
    if (failures != 0) {
        std::printf("%d C++ ownership check(s) failed\n", failures);
        return 1;
    }
    std::printf("madopilot-cpp-ownership complete\n");

    return 0;
}
