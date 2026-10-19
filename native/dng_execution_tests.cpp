#include "dng_execution.h"

#include <chrono>
#include <iostream>

namespace {
void require(bool condition) {
    if (!condition) {
        throw std::runtime_error("DNG execution invariant failed");
    }
}

void rows_cover_partial_work_once() {
    constexpr uint32_t width = 517, height = 513;
    std::vector<uint32_t> values(width * height, 0);
    std::vector<std::atomic<uint32_t>> visits(height);
    for (auto &visit : visits) {
        visit.store(0);
    }
    antitone_dng::Execution execution(8);
    const auto workers = execution.rows(width, height, [&](uint32_t first, uint32_t end) {
        for (uint32_t y = first; y < end; ++y) {
            require(visits[y].fetch_add(1) == 0);
            for (uint32_t x = 0; x < width; ++x) {
                values[y * width + x] = x + 17 * y + 1;
            }
        }
    });
    require(workers > 1 && workers <= execution.threads());
    for (uint32_t y = 0; y < height; ++y) {
        require(visits[y].load() == 1);
        for (uint32_t x = 0; x < width; ++x) {
            require(values[y * width + x] == x + 17 * y + 1);
        }
    }
    uint32_t tiny = 0;
    require(execution.rows(7, 3, [&](uint32_t first, uint32_t end) { tiny += end - first; }) == 1);
    require(tiny == 3);
}

void failure_rethrows_original_only_after_every_worker_retires() {
    struct OriginalFailure {};

    struct Lease {
        std::atomic<uint32_t> &active;

        explicit Lease(std::atomic<uint32_t> &value) : active(value) {
            ++active;
        }

        ~Lease() {
            --active;
        }
    };

    antitone_dng::Execution execution(4);
    std::atomic<uint32_t> started{0}, active{0}, finished{0};
    const auto deadline = std::chrono::steady_clock::now() + std::chrono::seconds(5);
    bool caught = false;
    try {
        execution.run(4, [&](uint32_t index) {
            Lease lease(active);
            ++started;
            while (started.load() < 4) {
                if (std::chrono::steady_clock::now() > deadline) {
                    throw std::runtime_error("DNG workers did not overlap");
                }
                std::this_thread::yield();
            }
            if (index == 1) {
                ++finished;
                throw OriginalFailure{};
            }
            while (!execution.aborted()) {
                if (std::chrono::steady_clock::now() > deadline) {
                    throw std::runtime_error("DNG failure did not stop peers");
                }
                std::this_thread::yield();
            }
            ++finished;
        });
    } catch (const OriginalFailure &) {
        caught = true;
    }
    require(caught && active == 0 && finished == 4);
}

void scratch_limits_worker_amplification_not_the_serial_baseline() {
    constexpr uint64_t mib = 1024 * 1024;
    for (uint32_t requested = 1; requested <= 8; ++requested) {
        for (uint64_t bytes : {mib, 9 * mib, 32 * mib, 64 * mib, 100 * mib}) {
            const auto count = antitone_dng::Execution::scratch_workers(requested, bytes);
            require(count >= 1 && count <= requested);
            require(count == 1 || bytes * count <= 64 * mib);
        }
    }
}
} // namespace

int main() {
    rows_cover_partial_work_once();
    failure_rethrows_original_only_after_every_worker_retires();
    scratch_limits_worker_amplification_not_the_serial_baseline();
    std::cout << "DNG worker ownership, failure joins and scratch bounds passed\n";
}
