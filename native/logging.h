#pragma once

#include <chrono>

extern "C" void antitone_native_log(int level, const char *component, const char *operation,
                                    int status, double elapsed_ms);

namespace antitone {
// Default failure also covers early returns and exception unwinding. Diagnostics
// never contain the input path; the existing error buffers remain authoritative.
class NativeLog {
    const char *component_;
    const char *operation_;
    std::chrono::steady_clock::time_point start_ = std::chrono::steady_clock::now();
    int status_ = -1;
    int level_ = 2;

public:
    NativeLog(const char *component, const char *operation)
        : component_(component), operation_(operation) {}

    void complete(int status = 0, int level = 4) noexcept {
        status_ = status;
        level_ = level;
    }

    ~NativeLog() noexcept {
        const double ms =
            std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - start_)
                .count();
        antitone_native_log(level_, component_, operation_, status_, ms);
    }
};
} // namespace antitone
