#pragma once

#include <algorithm>
#include <atomic>
#include <cstdint>
#include <exception>
#include <mutex>
#include <stdexcept>
#include <thread>
#include <vector>

namespace antitone_dng {

// One synchronous development owns this budget. A failed worker stops peers at
// their next sniff; all workers join before the original exception is rethrown.
class Execution {
    uint32_t budget;
    std::atomic<bool> stopped{false};

public:
    explicit Execution(uint32_t threads) : budget(threads) {
        if (threads < 1 || threads > 8) {
            throw std::invalid_argument("Invalid DNG worker budget");
        }
    }

    // Bound aggregate SDK tile buffers, or retain one serial buffer pair when
    // even that pair exceeds the cap. This does not bound full-image residency.
    static uint32_t scratch_workers(uint32_t requested, uint64_t per_worker) {
        constexpr uint64_t limit = 64 * 1024 * 1024;
        return uint32_t(std::min<uint64_t>(
            requested, std::max<uint64_t>(1, limit / std::max<uint64_t>(1, per_worker))));
    }

    uint32_t threads() const {
        return budget;
    }

    bool aborted() const {
        return stopped.load(std::memory_order_relaxed);
    }

    template <typename Function> void run(uint32_t count, Function function) {
        if (count < 1 || count > budget) {
            throw std::invalid_argument("Invalid DNG job count");
        }
        if (count == 1) {
            function(0);
            return;
        }
        std::mutex failure_mutex;
        std::exception_ptr failure;
        std::vector<std::thread> workers;
        workers.reserve(count - 1);
        auto work = [&](uint32_t index) {
            try {
                function(index);
            } catch (...) {
                std::lock_guard<std::mutex> lock(failure_mutex);
                if (!failure) {
                    failure = std::current_exception();
                }
                stopped.store(true, std::memory_order_relaxed);
            }
        };
        try {
            for (uint32_t index = 1; index < count; ++index) {
                workers.emplace_back(work, index);
            }
            work(0);
        } catch (...) {
            // Thread creation can fail after some workers have started.
            stopped.store(true, std::memory_order_relaxed);
            for (auto &worker : workers) {
                worker.join();
            }
            throw;
        }
        for (auto &worker : workers) {
            worker.join();
        }
        if (failure) {
            std::rethrow_exception(failure);
        }
    }

    template <typename Function> uint32_t rows(uint32_t width, uint32_t height, Function function) {
        const uint64_t work = uint64_t(width) * height;
        const auto count = uint32_t(std::min<uint64_t>(
            std::min(budget, height), std::max<uint64_t>(1, (work + 65535) / 65536)));
        run(count, [&](uint32_t index) {
            const auto first = uint32_t(uint64_t(height) * index / count);
            const auto end = uint32_t(uint64_t(height) * (index + 1) / count);
            function(first, end);
        });
        return count;
    }
};

} // namespace antitone_dng
