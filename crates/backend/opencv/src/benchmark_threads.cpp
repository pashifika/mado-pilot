// Benchmark-only scalar boundary; no native types or exceptions cross into Rust.
#include <cstddef>
#include <cstdint>

#ifdef _WIN32
#define MP_CV_IMPORT __declspec(dllimport)
#else
#define MP_CV_IMPORT
#endif

// OpenCV's apps/version/opencv_version.cpp uses this exported diagnostic API.
namespace cv {
MP_CV_IMPORT const char* currentParallelFramework();
MP_CV_IMPORT int getNumThreads();
}

extern "C" std::int32_t madopilot_benchmark_parallel_threads(
    std::uint8_t* framework, std::size_t capacity, std::int32_t* threads) noexcept {
    static_assert(sizeof(int) == sizeof(std::int32_t), "OpenCV thread count width");
    if (!framework || capacity == 0 || !threads) {
        return 1;
    }
    framework[0] = 0;
    *threads = 0;
    try {
        const char* name = cv::currentParallelFramework();
        if (!name || !name[0]) {
            return 1;
        }
        std::size_t length = 0;
        while (length < capacity && name[length]) {
            ++length;
        }
        if (length == capacity) {
            return 2;
        }
        const int count = cv::getNumThreads();
        for (std::size_t index = 0; index < length; ++index) {
            framework[index] = static_cast<std::uint8_t>(name[index]);
        }
        framework[length] = 0;
        *threads = count;
        return 0;
    } catch (...) {
        return 3;
    }
}
