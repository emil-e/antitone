#include "adapter.h"
#include <omp.h>
#include <array>
#include <future>
#include <latch>
#include <cstdio>
#include <fstream>
#include <memory>
#include <stdexcept>
#include <vector>

namespace {
void require(bool condition, const char *message) {
    if (!condition)
        throw std::runtime_error(message);
}

uint16_t short_at(const std::vector<uint8_t> &bytes, size_t at) {
    return bytes.at(at) | (uint16_t(bytes.at(at + 1)) << 8);
}

uint32_t long_at(const std::vector<uint8_t> &bytes, size_t at) {
    return short_at(bytes, at) | (uint32_t(short_at(bytes, at + 2)) << 16);
}

void check_pixels(const rawspeed3_ret_t &result, const std::vector<uint16_t> &sensor,
                  const uint16_t *curve = nullptr) {
    require(result.pixeldata && result.width == 256 && result.height == 192 && result.cpp == 1,
            "Decoded raster layout");
    auto *pixels = static_cast<const uint16_t *>(result.pixeldata);
    for (unsigned row = 0; row < result.height; ++row)
        for (unsigned col = 0; col < result.width; ++col) {
            const uint16_t value = sensor[row * result.width + col];
            require(pixels[row * (result.pitch / 2) + col] == (curve ? curve[value] : value),
                    "Decoded sample differs from authored uncompressed TIFF value");
        }
}
}

int main(int argc, char **argv) try {
    require(argc == 2, "Expected authored patches.dng path");
    std::ifstream stream(argv[1], std::ios::binary | std::ios::ate);
    require(bool(stream), "Cannot read authored DNG");
    std::vector<uint8_t> bytes(static_cast<size_t>(stream.tellg()));
    stream.seekg(0);
    stream.read(reinterpret_cast<char *>(bytes.data()), bytes.size());
    require(bool(stream), "Short fixture read");
    require(bytes.at(0) == 'I' && bytes.at(1) == 'I', "Fixture byte order");
    const size_t ifd = long_at(bytes, 4);
    uint32_t strip = 0;
    for (size_t i = 0; i < short_at(bytes, ifd); ++i) {
        const size_t entry = ifd + 2 + i * 12;
        if (short_at(bytes, entry) == 273)
            strip = long_at(bytes, entry + 8);
    }
    require(strip != 0, "Fixture strip offset");
    std::vector<uint16_t> sensor(256 * 192);
    for (size_t i = 0; i < sensor.size(); ++i)
        sensor[i] = short_at(bytes, strip + 2 * i);
    std::array<uint16_t, 65536> curve;
    for (size_t i = 0; i < curve.size(); ++i)
        curve[i] = 65535 - i;
    std::latch decoded(2);
    std::vector<std::future<void>> jobs;
    for (int threads : {1, 4}) {
        jobs.push_back(std::async(std::launch::async, [&, threads] {
            omp_set_num_threads(threads);
            std::unique_ptr<void, decltype(&rawspeed3_close)> local(rawspeed3_initdefault(),
                                                                    rawspeed3_close);
            const uint16_t *mapping = threads == 1 ? nullptr : curve.data();
            const AntitoneRawSpeedRequest expected{256, 192, 1, 0x94949494, mapping};
            rawspeed3_ret_t result{};
            int status = rawspeed3_not_inited;
            if (local) {
                antitone_rawspeed_prepare(local.get(), &expected);
                status =
                    rawspeed3_decodefile(local.get(), &result, bytes.data(), bytes.size(), false);
            }
            decoded.arrive_and_wait();
            require(status == rawspeed3_ok, "Concurrent provider decode");
            check_pixels(result, sensor, mapping);
        }));
    }
    for (auto &job : jobs)
        job.get();

    std::unique_ptr<void, decltype(&rawspeed3_close)> owner(rawspeed3_initdefault(),
                                                            rawspeed3_close);
    require(bool(owner), "Adapter owner allocation");
    rawspeed3_ret_t result{};
    require(rawspeed3_decodefile(owner.get(), &result, bytes.data(), bytes.size(), false) !=
                rawspeed3_ok,
            "Unconfigured owner must not publish pixels");
    AntitoneRawSpeedRequest request{256, 192, 1, 0x94949494, nullptr};
    antitone_rawspeed_prepare(owner.get(), &request);
    require(rawspeed3_decodefile(owner.get(), &result, bytes.data(), bytes.size(), false) ==
                rawspeed3_ok,
            "Authored integer Bayer DNG decode");
    check_pixels(result, sensor);
    require(antitone_rawspeed_retained_bytes(owner.get()) >= uint64_t(result.pitch) * result.height,
            "Source retention must include the provider's pixel backing");

    request.curve = curve.data();
    antitone_rawspeed_prepare(owner.get(), &request);
    require(rawspeed3_decodefile(owner.get(), &result, bytes.data(), bytes.size(), false) ==
                rawspeed3_ok,
            "Curve-mapped decode");
    check_pixels(result, sensor, curve.data());

    request.width += 2;
    antitone_rawspeed_prepare(owner.get(), &request);
    require(rawspeed3_decodefile(owner.get(), &result, bytes.data(), bytes.size(), false) !=
                    rawspeed3_ok &&
                !result.pixeldata && antitone_rawspeed_retained_bytes(owner.get()) < 4096,
            "Incompatible layout must release and withhold the decoded result");
    request.width -= 2;
    request.curve = nullptr;
    antitone_rawspeed_prepare(owner.get(), &request);
    require(rawspeed3_decodefile(owner.get(), &result, bytes.data(), 16, false) != rawspeed3_ok &&
                !result.pixeldata,
            "Truncated input must not publish partial samples");
    antitone_rawspeed_prepare(owner.get(), &request);
    require(rawspeed3_decodefile(owner.get(), &result, bytes.data(), bytes.size(), false) ==
                rawspeed3_ok,
            "Owner can decode after a failed attempt");
    std::vector<uint8_t>().swap(bytes);
    check_pixels(result, sensor);
    rawspeed3_release(owner.get());
    require(antitone_rawspeed_retained_bytes(owner.get()) < 4096,
            "Release drops source pixel storage");
    std::puts(
        "RawSpeed adapter: authored samples, curve, layout rejection, failure reuse and ownership passed");
    return 0;
} catch (const std::exception &error) {

    std::fprintf(stderr, "%s\n", error.what());
    return 1;
}
