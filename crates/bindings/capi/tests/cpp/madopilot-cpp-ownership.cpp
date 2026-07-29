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

#include <cstdio>
#include <cstdlib>
#include <new>
#include <string>
#include <string_view>
#include <thread>
#include <type_traits>
#include <vector>

#include "deterministic-scene.h"
#include "madopilot/madopilot.hpp"

/* ---------------------------------------------------------------------------
 * A starvable allocator
 *
 * One check needs an allocation to fail, because the wrapper's one documented
 * exception comes from copying owned error text and the release on that path
 * cannot be observed any other way. The replacement forwards to malloc unless a
 * check has armed it, so every other allocation in the program behaves as it
 * did. It is armed around a single call and disarmed before anything reports.
 * ------------------------------------------------------------------------ */

namespace {
bool starve_allocations = false;
}

void* operator new(std::size_t size)
{
    if (starve_allocations) {
        throw std::bad_alloc();
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
struct indexes : std::false_type {};

template <class T>
struct indexes<T, std::void_t<decltype(std::declval<T>().at(0))>> : std::true_type {};

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
static_assert(indexes<madopilot::TargetList&>::value, "a named target list is indexable");
static_assert(!indexes<madopilot::TargetList>::value, "a temporary target list is not");

/* Requests are values a caller composes and reuses. */
static_assert(std::is_copy_constructible_v<madopilot::Operation>, "Operation is a value");
static_assert(std::is_copy_constructible_v<madopilot::FindRequest>,
              "FindRequest is a value");
static_assert(std::is_copy_constructible_v<madopilot::Source>, "Source is a value");

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

/// `take_error_` releases the C handle even when copying its text throws.
///
/// The wrapper's one documented exception is `std::bad_alloc` from the owned
/// error text, and that is the path on which the release used to be skipped:
/// it was the last statement of the function rather than a scope guard. Nothing
/// the real library can be asked to do makes an allocation fail, so this drives
/// `take_error_` directly, against a table of two fakes and a starved allocator.
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
        static_cast<void>(madopilot::take_error_(&table, MADOPILOT_STATUS_INTERNAL, handle));
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
/// The status is asserted rather than merely checked for failure: the Phase 1
/// prefix has no coordinate-conversion entry, so a region it does not read is
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
    run("close reports its outcome", close_reports_its_outcome, fixture);
    run("concurrent readers use explicit clones", concurrent_readers_use_explicit_clones,
        fixture);

    if (failures != 0) {
        std::printf("%d C++ ownership check(s) failed\n", failures);
        return 1;
    }
    std::printf("madopilot-cpp-ownership complete\n");

    return 0;
}
