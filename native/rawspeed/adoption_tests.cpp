#include "adapter.h"
#include <libraw/libraw.h>
#include <omp.h>
#include <cstdio>
#include <cstring>
#include <fstream>
#include <memory>
#include <stdexcept>
#include <vector>

namespace {
void require(bool condition, const char *message) {
    if (!condition)
        throw std::runtime_error(message);
}

std::vector<uint8_t> fixture_with_curve(const char *path) {
    std::ifstream input(path, std::ios::binary | std::ios::ate);
    require(bool(input), "Read authored DNG");
    std::vector<uint8_t> bytes(static_cast<size_t>(input.tellg()));
    input.seekg(0);
    input.read(reinterpret_cast<char *>(bytes.data()), bytes.size());
    require(bool(input) && bytes.at(0) == 'I' && bytes.at(1) == 'I', "Little-endian fixture read");
    auto word = [&](size_t at) { return bytes.at(at) | (uint32_t(bytes.at(at + 1)) << 8); };
    const size_t old_directory = word(4) | (word(6) << 16);
    const size_t count = word(old_directory);
    const std::vector<uint8_t> entries(bytes.begin() + old_directory + 2,
                                       bytes.begin() + old_directory + 2 + count * 12);
    auto append_word = [&](uint32_t value) {
        bytes.push_back(value & 255);
        bytes.push_back((value >> 8) & 255);
    };
    auto append_long = [&](uint32_t value) {
        append_word(value);
        append_word(value >> 16);
    };
    const uint32_t table = bytes.size();
    for (unsigned value = 0; value < 4096; ++value)
        append_word(value / 2);
    const uint32_t directory = bytes.size();
    for (unsigned i = 0; i < 4; ++i)
        bytes[4 + i] = (directory >> (8 * i)) & 255;
    append_word(count + 1);
    bool inserted = false;
    for (size_t i = 0; i < count; ++i) {
        const auto at = entries.begin() + i * 12;
        const unsigned tag = at[0] | (unsigned(at[1]) << 8);
        require(tag != 50712, "Base fixture must not already contain a linearization table");
        if (!inserted && tag > 50712) {
            append_word(50712); // DNG LinearizationTable, SHORT, 4096 entries.
            append_word(3);
            append_long(4096);
            append_long(table);
            inserted = true;
        }
        bytes.insert(bytes.end(), at, at + 12);
    }
    require(inserted, "Linearization tag insertion");
    append_long(0);
    return bytes;
}

class TestRaw : public LibRaw {
public:
    void enable(bool incompatible_layout) {
        _rawspeed3_handle = rawspeed3_initdefault();
        require(_rawspeed3_handle != nullptr, "RawSpeed owner allocation");
        const AntitoneRawSpeedRequest request{
            uint32_t(imgdata.sizes.raw_width + (incompatible_layout ? 2 : 0)),
            imgdata.sizes.raw_height, 1, imgdata.idata.filters, imgdata.color.curve};
        antitone_rawspeed_prepare(_rawspeed3_handle, &request);
        imgdata.rawparams.use_rawspeed = LIBRAW_RAWSPEEDV3_USE | LIBRAW_RAWSPEEDV3_FAILONUNKNOWN;
    }

    bool accepted() const {
        return imgdata.process_warnings & LIBRAW_WARN_RAWSPEED3_PROCESSED;
    }

    int provider_status() const {
        return antitone_rawspeed_status(_rawspeed3_handle);
    }
};

std::unique_ptr<TestRaw> unpack(const char *path, bool fast, bool incompatible_layout,
                                std::vector<uint8_t> *buffer) {
    auto raw = std::make_unique<TestRaw>();
    const int code =
        buffer ? raw->open_buffer(buffer->data(), buffer->size()) : raw->open_file(path);
    require(code == LIBRAW_SUCCESS, "LibRaw fixture identification");
    if (buffer) {
        libraw_decoder_info_t info{};
        raw->get_decoder_info(&info);
        require(raw->imgdata.color.curve[1000] == 500 &&
                    (info.decoder_flags & LIBRAW_DECODER_HASCURVE),
                "Authored nonidentity curve must reach the curve-bearing handoff");
    }
    raw->imgdata.rawparams.use_rawspeed = 0;
    if (fast)
        raw->enable(incompatible_layout);
    require(raw->unpack() == LIBRAW_SUCCESS, "LibRaw unpack including fallback");
    return raw;
}

void same_sensor(const TestRaw &expected, const TestRaw &actual) {
    const auto &sizes = expected.imgdata.sizes;
    require(actual.imgdata.sizes.raw_width == sizes.raw_width &&
                actual.imgdata.sizes.raw_height == sizes.raw_height &&
                actual.imgdata.rawdata.raw_image,
            "LibRaw adopted/fallback raster layout");
    for (unsigned row = 0; row < sizes.raw_height; ++row)
        require(std::memcmp(expected.imgdata.rawdata.raw_image + row * (sizes.raw_pitch / 2),
                            actual.imgdata.rawdata.raw_image +
                                row * (actual.imgdata.sizes.raw_pitch / 2),
                            sizes.raw_width * sizeof(uint16_t)) == 0,
                "Provider or fallback changed sensor samples");
}

std::vector<uint8_t> develop(TestRaw &raw) {
    auto &p = raw.imgdata.params;
    p.user_qual = 11;
    p.no_auto_bright = 1;
    p.output_bps = 16;
    p.gamm[0] = p.gamm[1] = 1;
    for (auto &gain : p.user_mul)
        gain = 1;
    require(raw.dcraw_process() == LIBRAW_SUCCESS, "LibRaw DHT development");
    int error = 0;
    auto *image = raw.dcraw_make_mem_image(&error);
    require(image && error == LIBRAW_SUCCESS, "Developed pixel storage");
    std::vector<uint8_t> pixels(image->data, image->data + image->data_size);
    LibRaw::dcraw_clear_mem(image);
    return pixels;
}
}

int main(int argc, char **argv) try {
    require(argc == 2, "Expected authored patches.dng path");
    require(LibRaw::capabilities() & LIBRAW_CAPS_RAWSPEED3, "LibRaw must be built with RawSpeed3");
    auto curved = fixture_with_curve(argv[1]);
    for (auto *fixture : {static_cast<std::vector<uint8_t> *>(nullptr), &curved}) {
        for (int threads : {1, 4}) {
            omp_set_num_threads(threads);
            auto original = unpack(argv[1], false, false, fixture);
            auto adopted = unpack(argv[1], true, false, fixture);
            auto rejected = unpack(argv[1], true, true, fixture);
            require(!original->accepted() && adopted->accepted() && !rejected->accepted(),
                    "LibRaw must accept only the compatible result");
            require(rejected->provider_status() == rawspeed3_not_supported,
                    "Layout rejection must reach the native unpack fallback");
            same_sensor(*original, *adopted);
            same_sensor(*original, *rejected);
            const auto expected = develop(*original);
            require(develop(*adopted) == expected && develop(*rejected) == expected,
                    "Provider/fallback must preserve LibRaw DHT output");
        }
    }
    std::puts(
        "LibRaw adoption: accepted/rejected sensor, curve and DHT parity passed at 1/4 threads");
    return 0;
} catch (const std::exception &error) {

    std::fprintf(stderr, "%s\n", error.what());
    return 1;
}
