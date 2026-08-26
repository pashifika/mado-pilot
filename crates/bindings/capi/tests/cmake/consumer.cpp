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


    if (api.extent() < MADOPILOT_API_SIZE_ENGINE_OCR_PROVIDER_DESCRIPTOR ||
        build.value().bounded_ocr_model.empty() ||
        build.value().bounded_ocr_profile.empty()) {
        std::fprintf(stderr, "ABI 1.5 OCR provider surface is incomplete\n");
        return 1;
    }
    madopilot::OcrProfileOptions profile(
        MADOPILOT_OCR_PROFILE_BOUNDED_DETECTOR, "/controlled/model",
        "/controlled/runtime");
    auto profile_projection = profile.to_c();
    madopilot::ZoneScanOcrRequest zones;
    zones.model(build.value().bounded_ocr_model.view())
        .backend(build.value().default_ocr_backend.view(),
                 build.value().default_ocr_backend_version.view())
        .zone({MADOPILOT_SPACE_CAPTURE_PIXELS, 0, 0, 8, 8})
        .zone({MADOPILOT_SPACE_CAPTURE_PIXELS, 16, 0, 24, 8})
        .zone({MADOPILOT_SPACE_CAPTURE_PIXELS, 0, 16, 8, 24});
    auto first_projection = zones.to_c();
    auto second_projection = first_projection;
    if (profile_projection.value().kind !=
            MADOPILOT_OCR_PROFILE_BOUNDED_DETECTOR ||
        first_projection.value().zone_count != 3 ||
        first_projection.value().zones == second_projection.value().zones) {
        std::fprintf(stderr, "ABI 1.4 projection ownership is incomplete\n");
        return 1;
    }
    madopilot::OcrProviderOptions provider(
        MADOPILOT_OCR_PROVIDER_POLICY_REQUIRE_CUDA,
        "/controlled/cuda");
    auto provider_projection = provider.to_c();
    auto provider_copy = provider_projection;
    if (provider_projection.value().policy !=
            MADOPILOT_OCR_PROVIDER_POLICY_REQUIRE_CUDA ||
        provider_projection.value().provider_root.data ==
            provider_copy.value().provider_root.data) {
        std::fprintf(stderr, "ABI 1.5 provider projection ownership is incomplete\n");
        return 1;
    }
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
