// Exercise the patched implementation itself, including its private workspace.
#include "src/demosaic/dht_demosaic.cpp"
#include <array>
#include <atomic>
#include <cstdio>
#include <stdexcept>
#include <thread>
#include <vector>

namespace {
std::atomic<int> fail_array{0};
std::atomic<int> live_arrays{0};
std::atomic<int> array_calls{0};

void check(bool condition, const char *message) {
    if (!condition)
        throw std::runtime_error(message);
}
} // namespace

// Faults occur at the real array allocations, not at a substitute workspace.
void *operator new[](std::size_t bytes) {
    ++array_calls;
    if (fail_array.load() && --fail_array == 0)
        throw std::bad_alloc();
    void *memory = std::malloc(bytes ? bytes : 1);
    if (!memory)
        throw std::bad_alloc();
    ++live_arrays;
    return memory;
}

void operator delete[](void *memory) noexcept {
    if (memory) {
        --live_arrays;
        std::free(memory);
    }
}

void operator delete[](void *memory, std::size_t) noexcept {
    operator delete[](memory);
}

namespace {
using Pixel = std::array<ushort, 4>;

struct Developer : LibRaw {
    using LibRaw::border_interpolate;
    using LibRaw::dht_interpolate;
};

struct Sensor {
    std::unique_ptr<Developer> raw;
    std::vector<Pixel> pixels;

    Sensor(unsigned filters, unsigned seed) : raw(new Developer), pixels(128 * 128) {
        raw->imgdata.sizes.width = raw->imgdata.sizes.iwidth = 128;
        raw->imgdata.sizes.height = raw->imgdata.sizes.iheight = 128;
        raw->imgdata.idata.filters = filters;
        raw->imgdata.idata.colors = 3;
        raw->imgdata.image = reinterpret_cast<ushort(*)[4]>(pixels.data());
        for (int y = 0; y < 128; ++y)
            for (int x = 0; x < 128; ++x) {
                seed = seed * 1664525u + 1013904223u;
                const int c = raw->COLOR(y, x);
                // Structured weak-channel data, plus hot/dead samples for hide/restore.
                unsigned value = (c == 2 ? 8 : c == 1 ? 90 : 250) + ((seed >> 16) % 29);
                if (seed % 47 == 0)
                    value = 65535;
                if (seed % 59 == 0)
                    value = 0;
                pixels[y * 128 + x][c] = ushort(value);
            }
    }

    ~Sensor() {
        raw->imgdata.image = nullptr;
    }
};

void team(int count) {
#ifdef LIBRAW_USE_OPENMP
    omp_set_dynamic(0);
    omp_set_num_threads(count);
    int observed = 0;
#pragma omp parallel
    {
#pragma omp single
        observed = omp_get_num_threads();
    }
    check(observed == count, "requested OpenMP team was not exercised");
#else
    (void)count;
#endif
}

std::vector<Pixel> develop(unsigned filters, unsigned seed, int threads) {
    Sensor sensor(filters, seed);
    const auto measured = sensor.pixels;
    team(threads);
    sensor.raw->dht_interpolate();
    for (int y = 0; y < 128; ++y)
        for (int x = 0; x < 128; ++x) {
            const int c = sensor.raw->COLOR(y, x);
            check(sensor.pixels[y * 128 + x][c] == measured[y * 128 + x][c],
                  "DHT changed a measured CFA sample, including hot/dead samples");
        }
    return sensor.pixels;
}

void constant_fields() {
    for (unsigned filters : {0x94949494u, 0x49494949u, 0x61616161u, 0x16161616u}) {
        for (const std::array<ushort, 3> light : {std::array<ushort, 3>{0, 0, 0},
                                                  {65535, 65535, 65535},
                                                  {0, 256, 0},
                                                  {256, 0, 0},
                                                  {0, 0, 256},
                                                  {4096, 1024, 128}}) {
            Sensor sensor(filters, 1);
            for (int y = 0; y < 128; ++y)
                for (int x = 0; x < 128; ++x) {
                    auto &pixel = sensor.pixels[y * 128 + x];
                    pixel = {};
                    const int c = sensor.raw->COLOR(y, x);
                    pixel[c] = light[c];
                }
            sensor.raw->dht_interpolate();
            for (const auto &pixel : sensor.pixels)
                for (int c = 0; c < 3; ++c) {
                    const int tolerance = light[c] ? 1 : 0;
                    if (std::abs(int(pixel[c]) - int(light[c])) > tolerance) {
                        std::fprintf(
                            stderr,
                            "constant field filters=%x light=%u,%u,%u pixel=%zu channel=%d actual=%u\n",
                            filters, light[0], light[1], light[2],
                            std::size_t(&pixel - sensor.pixels.data()), c, pixel[c]);
                        check(false,
                              "constant light gained a border colour or absent-channel signal");
                    }
                }
        }
    }
}

void geometry_cases() {
    for (unsigned filters : {0x94949494u, 0x49494949u, 0x61616161u, 0x16161616u})
        for (const std::array<int, 2> size :
             {std::array<int, 2>{2, 2}, {17, 13}, {18, 13}, {17, 14}}) {
            Sensor sensor(filters, 1);
            const auto [width, height] = size;
            sensor.pixels.resize(width * height);
            sensor.raw->imgdata.image = reinterpret_cast<ushort(*)[4]>(sensor.pixels.data());
            sensor.raw->imgdata.sizes.width = sensor.raw->imgdata.sizes.iwidth = ushort(width);
            sensor.raw->imgdata.sizes.height = sensor.raw->imgdata.sizes.iheight = ushort(height);
            const std::array<ushort, 3> light{4096, 1024, 128};
            for (int y = 0; y < height; ++y)
                for (int x = 0; x < width; ++x) {
                    auto &pixel = sensor.pixels[y * width + x];
                    pixel = {};
                    const int c = sensor.raw->COLOR(y, x);
                    pixel[c] = light[c];
                }
            sensor.raw->dht_interpolate();
            for (const auto &pixel : sensor.pixels)
                for (int c = 0; c < 3; ++c)
                    check(std::abs(int(pixel[c]) - int(light[c])) <= 1,
                          "odd, small or non-square geometry changed constant light");
        }
}

void zero_light_region() {
    for (unsigned filters : {0x94949494u, 0x49494949u, 0x61616161u, 0x16161616u}) {
        Sensor sensor(filters, 1);
        const std::array<ushort, 3> light{4096, 1024, 128};
        for (int y = 0; y < 128; ++y)
            for (int x = 0; x < 128; ++x) {
                auto &pixel = sensor.pixels[y * 128 + x];
                pixel = {};
                const int c = sensor.raw->COLOR(y, x);
                pixel[c] = x < 64 ? light[c] : 0;
            }
        sensor.raw->dht_interpolate();
        for (int y = 16; y < 112; ++y)
            for (int x = 80; x < 112; ++x)
                for (int c = 0; c < 3; ++c)
                    check(sensor.pixels[y * 128 + x][c] == 0,
                          "global range clamp invented light inside the zero-signal region");
    }
}

void allocation_failures() {
    Sensor sensor(0x94949494, 7);
    const int initial_live = live_arrays;
    const int initial_calls = array_calls;
    int allocations;
    {
        DHT workspace(*sensor.raw);
        allocations = array_calls - initial_calls;
        check(allocations > 0, "workspace fault-injection control found no allocations");
        check(live_arrays == initial_live + allocations, "workspace arrays not retained by owner");
    }
    check(live_arrays == initial_live, "successful workspace leaked");
    for (int failure = 1; failure <= allocations; ++failure) {
        const int before = live_arrays;
        const auto measured = sensor.pixels;
        const int calls = array_calls;
        fail_array = failure;
        bool rejected = false;
        try {
            DHT workspace(*sensor.raw);
        } catch (const std::bad_alloc &) {
            rejected = true;
        }
        fail_array = 0;
        check(rejected, "fault injection did not reach a workspace allocation");
        check(array_calls - calls == failure, "wrong allocation failure site exercised");
        check(live_arrays == before, "partially constructed workspace leaked");
        check(sensor.pixels == measured, "allocation failure modified source pixels");
    }
    const int calls = array_calls;
    sensor.raw->imgdata.sizes.iheight = sensor.raw->imgdata.sizes.iwidth = 65535;
    bool rejected = false;
    try {
        DHT workspace(*sensor.raw);
    } catch (const LibRaw_exceptions &error) {
        rejected = error == LIBRAW_EXCEPTION_TOOBIG;
    }
    check(rejected && array_calls == calls, "oversized offsets not rejected before allocation");
}

// A TSan-friendly dispatch of the actual parallel row kernels. std::thread joins
// expose stage barriers without relying on an instrumented OpenMP runtime.
template <typename Row> void rows(Row row) {
    std::thread a([&] {
        for (int i = 0; i < 128; i += 2)
            row(i);
    });
    std::thread b([&] {
        for (int i = 1; i < 128; i += 2)
            row(i);
    });
    a.join();
    b.join();
}

void row_kernels() {
    Sensor sensor(0x94949494, 123);
    const auto expected = develop(0x94949494, 123, 1);
    {
        DHT dht(*sensor.raw);
        dht.hide_hots();
        rows([&](int i) { dht.make_hv_dline(i); });
        rows([&](int i) { dht.refine_hv_dirs(i, i & 1); });
        rows([&](int i) { dht.refine_hv_dirs(i, (i & 1) ^ 1); });
        for (int i = 0; i < 128; ++i)
            dht.refine_ihv_dirs(i);
        rows([&](int i) { dht.make_gline(i); });
        rows([&](int i) { dht.make_diag_dline(i); });
        for (int i = 0; i < 128; ++i)
            dht.refine_idiag_dirs(i);
        rows([&](int i) { dht.make_rbdiag(i); });
        rows([&](int i) { dht.make_rbhv(i); });
        dht.restore_hots();
        dht.copy_to_image();
        sensor.raw->border_interpolate(4);
    }
    check(sensor.pixels == expected, "parallel row kernels changed ordered development");
}
} // namespace

int main() {
    try {
#if defined(_OPENMP) && !defined(LIBRAW_USE_OPENMP)
#error OpenMP build did not enable LibRaw parallel stages
#endif
        allocation_failures();
        team(2);
        constant_fields();
        geometry_cases();
        zero_light_region();
        row_kernels();
        for (unsigned filters : {0x94949494u, 0x49494949u, 0x61616161u, 0x16161616u})
            for (unsigned seed : {1u, 13u, 931u, 34501u}) {
                const auto expected = develop(filters, seed, 1);
                for (int threads : {2, 8})
                    check(develop(filters, seed, threads) == expected, "DHT team-size dependence");
                std::vector<Pixel> a, b;
                std::thread first([&] { a = develop(filters, seed, 2); });
                std::thread second([&] { b = develop(filters, seed + 1, 2); });
                first.join();
                second.join();
                check(a == expected && b == develop(filters, seed + 1, 1),
                      "DHT owner interference");
            }
        std::puts(
            "DHT: allocation cleanup, bounds, constant/zero light, row kernels, measured sites, teams and owners passed");
    } catch (const std::exception &error) {
        std::fprintf(stderr, "DHT safety failure: %s\n", error.what());
        return 1;
    }
}
