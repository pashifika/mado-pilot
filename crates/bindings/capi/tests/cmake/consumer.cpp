/*
 * A C++ consumer of `MadoPilot::Cpp`, and nothing else.
 *
 * `target_link_libraries(consumer-cpp PRIVATE MadoPilot::Cpp)` is the only
 * MadoPilot line in this program's build. The C++ include directory, the C
 * header the wrapper includes, C++17, and the released C library all have to
 * arrive through that one target or this does not compile, link, or run.
 *
 * The flow below is deliberately short. What the wrapper does with a session is
 * checked by `tests/cpp/madopilot-cpp-ownership.cpp`; what is checked here is
 * that a consumer project can reach it at all, and that no Rust-internal
 * library had to be named to do so.
 */

#include <cstdio>

#include "madopilot/madopilot.hpp"

int main()
{
    auto loaded = madopilot::Api::load();
    if (!loaded) {
        std::fprintf(stderr, "madopilot::Api::load failed with %d\n",
                     static_cast<int>(loaded.status()));
        return 1;
    }
    const madopilot::Api api = loaded.take();

    const auto build = api.describe_build();
    if (!build) {
        std::fprintf(stderr, "describe_build failed with %d\n",
                     static_cast<int>(build.status()));
        return 1;
    }

    std::printf("MadoPilot::Cpp consumer: abi %u.%u, table %u bytes, library %s\n",
                build.value().abi_major, build.value().abi_minor,
                build.value().table_size,
                build.value().library_version.to_string().c_str());

    // One RAII owner, taken and released without a single explicit release call.
    auto cancellation = api.create_cancellation();
    if (!cancellation) {
        std::fprintf(stderr, "create_cancellation failed with %d\n",
                     static_cast<int>(cancellation.status()));
        return 1;
    }

    const madopilot::Cancellation token = cancellation.take();
    if (!token.cancel()) {
        std::fprintf(stderr, "cancel failed\n");
        return 1;
    }

    const auto cancelled = token.is_cancelled();
    if (!cancelled || !cancelled.value()) {
        std::fprintf(stderr, "the token did not report itself cancelled\n");
        return 1;
    }

    std::printf("MadoPilot::Cpp consumer: ownership round trip complete\n");

    return 0;
}
