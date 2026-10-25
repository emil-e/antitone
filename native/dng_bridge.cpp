#include "dng_bridge.h"
#include "logging.h"
#include "dng_execution.h"

#include "dng_abort_sniffer.h"
#include "dng_area_task.h"
#include "dng_auto_ptr.h"
#include "dng_camera_profile.h"
#include "dng_color_spec.h"
#include "dng_exceptions.h"
#include "dng_file_stream.h"
#include "dng_filter_task.h"
#include "dng_host.h"
#include "dng_gain_map.h"
#include "dng_ifd.h"
#include "dng_image.h"
#include "dng_info.h"
#include "dng_linearization_info.h"
#include "dng_memory.h"
#include "dng_memory_stream.h"
#include "dng_negative.h"
#include "dng_opcode_list.h"
#include "dng_opcodes.h"
#include "dng_pixel_buffer.h"
#include "dng_shared.h"
#include "dng_tag_types.h"
#include "dng_tag_values.h"
#include "dng_xmp_sdk.h"
#include "dng_xy_coord.h"
#include "dng_utils.h"

#include <algorithm>
#include <array>
#include <cmath>
#include <chrono>
#include <cstdio>
#include <cstring>
#include <limits>
#include <memory>
#include <mutex>
#include <stdexcept>
#include <string>
#include <vector>
#if defined(_WIN32)
#include <windows.h>
#endif

namespace {
static_assert(kDNGSDK_MajorVersion == 1 && kDNGSDK_MinorVersion == 7 && kDNGSDK_DotVersion == 1 &&
                  kDNGSDK_BuildVersion == 2768,
              "This adapter's interpretation is pinned to DNG SDK 1.7.1 build 2768");

using Clock = std::chrono::steady_clock;

double elapsed(Clock::time_point start) {
    return std::chrono::duration<double>(Clock::now() - start).count();
}

constexpr uint8_t sensor_white = 1;
constexpr uint8_t sensor_black = 2;
constexpr uint8_t repeated_border = 4;
constexpr uint8_t processing_limit = 8;

void require(bool condition, const char *message) {
    if (!condition) {
        throw std::runtime_error(message);
    }
}

struct Runtime {
    std::mutex jobs;

    Runtime() {
        dng_xmp_sdk::InitializeSDK();
    }

    ~Runtime() {
        dng_xmp_sdk::TerminateSDK();
    }
};

Runtime &runtime() {
    static Runtime value;
    return value;
}

class Sniffer final : public dng_abort_sniffer {
    const AntitoneDngOptions &options;
    antitone_dng::Execution &execution;

public:
    Sniffer(const AntitoneDngOptions &value, antitone_dng::Execution &workers)
        : options(value), execution(workers) {}

    bool ThreadSafe() const override {
        return true;
    }

    void Sniff() override {
        if (execution.aborted() ||
            (options.cancelled && options.cancelled(options.cancel_context))) {
            ThrowUserCanceled();
        }
    }
};

// Integer input still uses the SDK linearizer, but avoids its intermediate u16 rounding.
class FloatNegative final : public dng_negative {
public:
    explicit FloatNegative(dng_host &host) : dng_negative(host) {
        Initialize();
    }

protected:
    void DoBuildStage2(dng_host &host) override {
        auto &info = *fLinearizationInfo.Get();
        fStage2Image.Reset(host.Make_dng_image(info.fActiveArea.Size(), 1, ttFloat));
        info.Linearize(host, *this, *fStage1Image.Get(), *fStage2Image.Get());
    }
};

struct Source {
    AntitoneDngView view{};
    std::vector<float> rgb;
    std::vector<uint16_t> original;
    std::vector<uint8_t> flags;
    std::vector<uint8_t> program;

    void finish() {
        view.rgb = rgb.data();
        view.original = original.data();
        view.flags = flags.data();
        view.program = program.data();
        view.program_size = program.size();
        view.retained_bytes = sizeof(Source) + rgb.capacity() * sizeof(float) +
                              original.capacity() * sizeof(uint16_t) + flags.capacity() +
                              program.capacity();
    }
};

template <typename T> std::vector<T> pixels(const dng_image &image, uint32 type) {
    require(image.PixelType() == type, "Unexpected SDK image representation");
    const size_t count = size_t(image.Width()) * image.Height() * image.Planes();
    std::vector<T> values(count);
    dng_pixel_buffer buffer(image.Bounds(), 0, image.Planes(), type, pcInterleaved, values.data());
    image.Get(buffer);
    return values;
}

class Host final : public dng_host {
    Source &source;
    antitone_dng::Execution &execution;
    std::vector<uint8_t> sites;
    uint32 stage = 0;
    uint32 current_opcode = 0;

    size_t site_index(uint32 x, uint32 y) const {
        return size_t(y + source.view.active_top) * source.view.raw_width + x +
               source.view.active_left;
    }

    void note_limits(const dng_image &image) {
        const auto &v = source.view;
        if (stage == 1) {
            const auto values = pixels<uint16_t>(image, ttShort);
            require(values.size() == sites.size(), "Opcode changed original sensor geometry");
            for (size_t i = 0; i < values.size(); ++i) {
                if (i % v.raw_width == 0) {
                    SniffForAbort();
                }
                if (values[i] <= v.black || values[i] >= v.white) {
                    sites[i] |= processing_limit;
                }
            }
            return;
        }
        const auto values = pixels<float>(image, ttFloat);
        const uint32 planes = stage == 2 ? 1 : 3;
        require(image.Width() == v.width && image.Height() == v.height && image.Planes() == planes,
                "Opcode changed the admitted image layout");
        for (uint32 y = 0; y < v.height; ++y) {
            SniffForAbort();
            for (uint32 x = 0; x < v.width; ++x) {
                const size_t i = size_t(y) * v.width + x;
                for (uint32 p = 0; p < planes; ++p) {
                    const float value = values[i * planes + p];
                    require(std::isfinite(value) && value >= 0 && value <= 1,
                            "SDK output outside the admitted normalized camera range");
                    if (value <= 0 || value >= 1) {
                        if (stage == 2) {
                            sites[site_index(x, y)] |= processing_limit;
                        } else {
                            source.flags[i] |= processing_limit;
                        }
                    }
                }
            }
        }
    }

    void propagate_bad_pixels(dng_filter_task &filter, const dng_rect &area) {
        const int32 width = int32(source.view.raw_width);
        const int32 height = int32(source.view.raw_height);
        require(area == dng_rect(height, width), "Unexpected bad-pixel repair bounds");
        std::vector<uint8_t> next(sites.size());
        const auto workers = execution.rows(width, height, [&](uint32 first, uint32 end) {
            for (int32 y = int32(first); y < int32(end); ++y) {
                SniffForAbort();
                for (int32 x = 0; x < width; ++x) {
                    // FixBadPixelsConstant reads an immutable source, with a two-site halo
                    // and 2x2 edge repeat. Union the full halo, not only selected neighbours.
                    const auto src = filter.SrcArea(dng_rect(y, x, y + 1, x + 1));
                    require(src.W() == 5 && src.H() == 5,
                            "Unrecognized bad-pixel dependency kernel");
                    uint8_t flags = src.l < 0 || src.t < 0 || src.r > width || src.b > height
                                        ? repeated_border
                                        : 0;
                    for (int32 sy = std::max(0, src.t); sy < std::min(height, src.b); ++sy) {
                        for (int32 sx = std::max(0, src.l); sx < std::min(width, src.r); ++sx) {
                            flags |= sites[size_t(sy) * width + sx];
                        }
                    }
                    next[size_t(y) * width + x] = flags;
                }
            }
        });
        source.view.mask_workers = std::max(source.view.mask_workers, workers);
        sites.swap(next);
    }

    void propagate_warp(dng_filter_task &filter, const dng_rect &area) {
        const int32 width = int32(source.view.width);
        const int32 height = int32(source.view.height);
        require(area == dng_rect(height, width), "Unexpected SDK warp bounds");
        std::vector<uint8_t> next(source.flags.size());
        const auto workers = execution.rows(width, height, [&](uint32 first, uint32 end) {
            for (int32 y = int32(first); y < int32(end); ++y) {
                SniffForAbort();
                for (int32 x = 0; x < width; ++x) {
                    // SDK returns the union of all planes' reconstruction read bounds.
                    const auto src = filter.SrcArea(dng_rect(y, x, y + 1, x + 1));
                    require(src.W() > 0 && src.H() > 0 && src.W() <= 64 && src.H() <= 64,
                            "Warp dependency span exceeds the prototype's 64-site bound");
                    // SrcArea already intersects its read bounds with the source image. Touching
                    // an edge may therefore hide a clamped position, even if prior warps made that
                    // edge otherwise eligible. Include the clamp-adjusted kernel conservatively.
                    const bool touches_border =
                        src.l <= 0 || src.t <= 0 || src.r >= width || src.b >= height;
                    const int32 pad = touches_border ? 2 : 0;
                    const int32 left = std::max(0, std::clamp(src.l, 0, width - 1) - pad);
                    const int32 top = std::max(0, std::clamp(src.t, 0, height - 1) - pad);
                    const int32 right =
                        std::min(width, std::clamp(src.r - 1, 0, width - 1) + 1 + pad);
                    const int32 bottom =
                        std::min(height, std::clamp(src.b - 1, 0, height - 1) + 1 + pad);
                    uint8_t flags = touches_border ? repeated_border : 0;
                    for (int32 sy = top; sy < bottom; ++sy) {
                        for (int32 sx = left; sx < right; ++sx) {
                            flags |= source.flags[size_t(sy) * width + sx];
                        }
                    }
                    next[size_t(y) * width + x] = flags;
                }
            }
        });
        source.view.mask_workers = std::max(source.view.mask_workers, workers);
        source.flags.swap(next);
    }

    void perform_sdk(dng_area_task &task, const dng_rect &area, dng_area_task_progress *progress) {
        // Only the admitted warp is parallelized here. Its thread-indexed SDK tile
        // buffers and immutable parameters support disjoint, tile-aligned stripes.
        if (stage != 3 || progress) {
            dng_host::PerformAreaTask(task, area, progress);
            source.view.sdk_workers = std::max(source.view.sdk_workers, uint32(1));
            return;
        }
        require(std::strcmp(task.Name(), "dng_filter_warp") == 0 &&
                    area == dng_rect(source.view.height, source.view.width),
                "Unrecognized SDK parallel warp task");
        const auto tile = task.FindTileSize(area);
        const uint32 tile_rows = (area.H() + tile.v - 1) / tile.v;
        const uint64 pixels = uint64(area.W()) * area.H();
        const uint32 useful =
            uint32(std::max<uint64>(1, pixels / std::max(uint32(1), task.MinTaskArea())));
        auto *filter = dynamic_cast<dng_filter_task *>(&task);
        require(filter, "Unexpected SDK warp filter");
        const uint64 tile_bytes =
            uint64(ComputeBufferSize(ttFloat, filter->SrcTileSize(tile), 3, padSIMDBytes)) +
            ComputeBufferSize(ttFloat, tile, 3, padSIMDBytes);
        const uint32 workers = antitone_dng::Execution::scratch_workers(
            std::min({execution.threads(), task.MaxThreads(), tile_rows, useful}), tile_bytes);
        source.view.warp_tile_scratch_bytes =
            std::max(source.view.warp_tile_scratch_bytes, tile_bytes * workers);
        require(workers > 0, "SDK warp has no available worker");
        source.view.sdk_workers = std::max(source.view.sdk_workers, workers);
        if (workers == 1) {
            dng_host::PerformAreaTask(task, area, progress);
            return;
        }
        task.Start(workers, area, tile, &Allocator(), Sniffer());
        try {
            execution.run(workers, [&](uint32 index) {
                const int32 top = area.t + int32(uint64(tile_rows) * index / workers) * tile.v;
                const int32 bottom = std::min(
                    area.b, area.t + int32(uint64(tile_rows) * (index + 1) / workers) * tile.v);
                task.ProcessOnThread(index, dng_rect(top, area.l, bottom, area.r), tile, Sniffer(),
                                     nullptr);
            });
        } catch (...) {
            // Match the SDK serial lifecycle, but only after every worker joined.
            task.Finish(workers);
            throw;
        }
        task.Finish(workers);
    }

public:
    Host(class Sniffer &sniffer, Source &value, antitone_dng::Execution &workers)
        : dng_host(nullptr, &sniffer), source(value), execution(workers) {}

    uint32 PerformAreaTaskThreads() override {
        return execution.threads();
    }

    dng_negative *Make_dng_negative() override {
        return new FloatNegative(*this);
    }

    void capture_sites() {
        const auto &v = source.view;
        sites.resize(source.original.size());
        for (size_t i = 0; i < sites.size(); ++i) {
            if (i % v.raw_width == 0) {
                SniffForAbort();
            }
            sites[i] = (source.original[i] >= v.white ? sensor_white : 0) |
                       (source.original[i] <= v.black ? sensor_black : 0);
        }
    }

    void prepare_bilinear(const dng_image &stage2) {
        const auto start = Clock::now();
        stage = 2;
        note_limits(stage2);
        const auto &v = source.view;
        source.flags.resize(size_t(v.width) * v.height);
        // Admitted rectangular 2x2 Bayer uses InterpolateGeneric: its source tiles have
        // two-site padding. Union that entire halo, including unused/zero-weight sites.
        const auto workers = execution.rows(v.width, v.height, [&](uint32 first, uint32 end) {
            for (int32 y = int32(first); y < int32(end); ++y) {
                SniffForAbort();
                for (int32 x = 0; x < int32(v.width); ++x) {
                    uint8_t flags = 0;
                    for (int32 dy = -2; dy <= 2; ++dy) {
                        for (int32 dx = -2; dx <= 2; ++dx) {
                            const int32 sx = x + dx;
                            const int32 sy = y + dy;
                            if (sx < 0 || sy < 0 || sx >= int32(v.width) || sy >= int32(v.height)) {
                                flags |= repeated_border;
                            } else {
                                flags |= sites[site_index(uint32(sx), uint32(sy))];
                            }
                        }
                    }
                    source.flags[size_t(y) * v.width + x] = flags;
                }
            }
        });
        source.view.mask_workers = std::max(source.view.mask_workers, workers);
        sites.clear();
        sites.shrink_to_fit();
        source.view.bilinear_mask_seconds += elapsed(start);
    }

    void ApplyOpcodeList(dng_opcode_list &list, dng_negative &negative,
                         AutoPtr<dng_image> &image) override {
        stage = &list == &negative.OpcodeList1() ? 1 : &list == &negative.OpcodeList2() ? 2 : 3;
        // Unsupported entries have already been removed/rejected. In particular, no
        // WarpRectilinear2 is admitted, so its conditional fallback skip rule cannot arise.
        for (uint32 i = 0; i < list.Count(); ++i) {
            SniffForAbort();
            auto &opcode = list.Entry(i);
            current_opcode = opcode.OpcodeID();
            if (opcode.AboutToApply(*this, negative, image->Bounds(), image->Planes())) {
                opcode.Apply(*this, negative, image);
                note_limits(*image.Get());
                ++source.view.applied[stage - 1];
            } else {
                ++source.view.skipped[stage - 1];
            }
        }
        current_opcode = 0;
    }

    void PerformAreaTask(dng_area_task &task, const dng_rect &area,
                         dng_area_task_progress *progress = nullptr) override {
        auto start = Clock::now();
        perform_sdk(task, area, progress);
        const auto sdk_seconds = elapsed(start);
        source.view.sdk_area_seconds += sdk_seconds;
        if (stage == 3) {
            source.view.warp_sdk_seconds += sdk_seconds;
        }
        start = Clock::now();
        if (stage == 1 && current_opcode == 4) {
            auto *filter = dynamic_cast<dng_filter_task *>(&task);
            require(filter && std::strcmp(task.Name(), "dng_filter_opcode_task") == 0,
                    "Unrecognized bad-pixel repair task");
            propagate_bad_pixels(*filter, area);
            source.view.bad_pixel_mask_seconds += elapsed(start);
        }
        if (stage == 3) {
            auto *filter = dynamic_cast<dng_filter_task *>(&task);
            require(filter && std::strcmp(task.Name(), "dng_filter_warp") == 0,
                    "Unrecognized SDK dependency task");
            propagate_warp(*filter, area);
            source.view.warp_mask_seconds += elapsed(start);
        }
    }
};

uint32 integral(const dng_urational &value) {
    require(value.d && value.n % value.d == 0,
            "Fractional crop/scale is outside the prototype domain");
    return value.n / value.d;
}

void admit(const dng_ifd &ifd, dng_negative &negative, const AntitoneDngOptions &options,
           Source &source) {
    require(options.max_pixels > 0 && options.max_pixels <= std::numeric_limits<uint32_t>::max(),
            "Supply a positive pixel bound no greater than u32::MAX");
    require(ifd.fImageWidth >= 8 && ifd.fImageLength >= 8 &&
                uint64_t(ifd.fImageWidth) * ifd.fImageLength <= options.max_pixels,
            "DNG sensor exceeds the caller's pixel bound or is too small");
    require(
        ifd.fPhotometricInterpretation == 32803 && ifd.fSamplesPerPixel == 1 &&
            ifd.fBitsPerSample[0] >= 8 && ifd.fBitsPerSample[0] <= 16 &&
            ifd.fSampleFormat[0] == 1 && (ifd.fCompression == 1 || ifd.fCompression == 7) &&
            ifd.fCFALayout == 1 && ifd.fCFARepeatPatternRows == 2 &&
            ifd.fCFARepeatPatternCols == 2 && negative.ColorChannels() == 3,
        "SDK developer requires unsigned 8-16 bit rectangular Bayer, uncompressed or lossless JPEG");
    require(ifd.fCFAPlaneColor[0] == 0 && ifd.fCFAPlaneColor[1] == 1 && ifd.fCFAPlaneColor[2] == 2,
            "Prototype requires RGB CFA plane order");
    std::array<uint32, 3> counts{};
    uint32 red = 4, blue = 4;
    for (uint32 i = 0; i < 4; ++i) {
        const uint32 color = ifd.fCFAPattern[i / 2][i % 2];
        require(color < 3, "Unsupported CFA color");
        ++counts[color];
        source.view.cfa_pattern |= color << (8 * i);
        if (color == 0) {
            red = i;
        }
        if (color == 2) {
            blue = i;
        }
    }
    require(counts == std::array<uint32, 3>{1, 2, 1} && (red ^ blue) == 3,
            "Unsupported Bayer pattern");
    require(ifd.fBlackLevelRepeatRows >= 1 && ifd.fBlackLevelRepeatRows <= 2 &&
                ifd.fBlackLevelRepeatCols >= 1 && ifd.fBlackLevelRepeatCols <= 2 &&
                !ifd.fBlackLevelDeltaHCount && !ifd.fBlackLevelDeltaVCount &&
                !ifd.fLinearizationTableCount,
            "SDK developer requires uniform black with no deltas or linearization table");
    for (uint32 y = 0; y < ifd.fBlackLevelRepeatRows; ++y) {
        for (uint32 x = 0; x < ifd.fBlackLevelRepeatCols; ++x) {
            require(ifd.fBlackLevel[y][x][0] == ifd.fBlackLevel[0][0][0],
                    "Non-uniform black pattern is outside the supported SDK domain");
        }
    }
    require(integral(ifd.fDefaultScaleH) == 1 && integral(ifd.fDefaultScaleV) == 1,
            "Prototype requires square native-resolution pixels");
    require(ifd.fDefaultUserCropT.As_real64() == 0 && ifd.fDefaultUserCropL.As_real64() == 0 &&
                ifd.fDefaultUserCropB.As_real64() == 1 && ifd.fDefaultUserCropR.As_real64() == 1,
            "DefaultUserCrop is outside the prototype domain");
    require(!negative.TransparencyMask() && !negative.HasDepthMap() && !negative.HasSemanticMask(),
            "Auxiliary image processing is outside the prototype domain");
    auto &v = source.view;
    v.black = ifd.fBlackLevel[0][0][0];
    v.white = ifd.fWhiteLevel[0];
    require(std::isfinite(v.black) && std::isfinite(v.white) && v.black >= 0 && v.black < v.white &&
                v.white <= 65535,
            "Invalid original capture limits");
    const auto area = ifd.fActiveArea;
    require(area.l >= 0 && area.t >= 0 && area.r <= int64_t(ifd.fImageWidth) &&
                area.b <= int64_t(ifd.fImageLength) && area.W() >= 8 && area.H() >= 8,
            "Invalid active area");
    v.raw_width = ifd.fImageWidth;
    v.raw_height = ifd.fImageLength;
    v.width = area.W();
    v.height = area.H();
    v.active_left = area.l;
    v.active_top = area.t;
    v.crop_left = integral(ifd.fDefaultCropOriginH);
    v.crop_top = integral(ifd.fDefaultCropOriginV);
    v.crop_width = integral(ifd.fDefaultCropSizeH);
    v.crop_height = integral(ifd.fDefaultCropSizeV);
    require(v.crop_width && v.crop_height && uint64_t(v.crop_left) + v.crop_width <= v.width &&
                uint64_t(v.crop_top) + v.crop_height <= v.height,
            "Invalid default crop");
    v.orientation = negative.BaseOrientation().GetTIFF();
    require(v.orientation >= 1 && v.orientation <= 8, "Invalid orientation");
}

void admit_opcodes(Host &host, dng_negative &negative, Source &source) {
    std::array<dng_opcode_list *, 3> lists{&negative.OpcodeList1(), &negative.OpcodeList2(),
                                           &negative.OpcodeList3()};
    double amplification = *std::max_element(source.view.wb, source.view.wb + 3);
    for (uint32 stage = 0; stage < 3; ++stage) {
        auto &list = *lists[stage];
        require(list.Count() <= 64, "Opcode count exceeds prototype bound");
        AutoPtr<dng_memory_block> bytes(list.Spool(host));
        // Frame each canonical list, including ignored optional instructions, for identity.
        const uint32 length = bytes.Get() ? bytes->LogicalSize() : 0;
        for (uint32 shift = 0; shift < 32; shift += 8) {
            source.program.push_back(uint8_t(length >> shift));
        }
        if (length) {
            const auto *data = bytes->Buffer_uint8();
            source.program.insert(source.program.end(), data, data + length);
        }
        for (uint32 i = list.Count(); i > 0; --i) {
            auto &opcode = list.Entry(i - 1);
            const bool supported =
                opcode.MinVersion() <= dngVersion_Current &&
                ((opcode.OpcodeID() == 9 && stage < 2) || (opcode.OpcodeID() == 4 && stage == 0) ||
                 (opcode.OpcodeID() == 1 && stage == 2));
            if (!supported) {
                if (!opcode.Optional()) {
                    throw std::runtime_error("Unsupported required DNG opcode " +
                                             std::to_string(opcode.OpcodeID()) + " in list " +
                                             std::to_string(stage + 1) + "; no fallback");
                }
                list.Remove(i - 1);
                ++source.view.skipped[stage];
            } else {
                require((opcode.Flags() & ~uint32(3)) == 0, "Unknown opcode flags");
                if (opcode.OpcodeID() == 9) {
                    const auto *gain = dynamic_cast<const dng_opcode_GainMap *>(&opcode);
                    require(gain, "Unexpected GainMap implementation");
                    const auto &map = gain->GainMap();
                    double maximum = 1.0;
                    for (uint32 y = 0; y < uint32(map.Points().v); ++y) {
                        host.SniffForAbort();
                        for (uint32 x = 0; x < uint32(map.Points().h); ++x) {
                            for (uint32 p = 0; p < map.Planes(); ++p) {
                                const double value = map.Entry(y, x, p);
                                require(std::isfinite(value) && value > 0,
                                        "Invalid gain map range");
                                maximum = std::max(maximum, value);
                            }
                        }
                    }
                    // List 1 rounds back to sensor integers: a one-code input
                    // change can span ceil(gain) output codes, not merely gain.
                    amplification *= stage == 0 ? std::ceil(maximum) : maximum;
                } else {
                    if (opcode.OpcodeID() == 4) {
                        // The SDK casts the marker to u16 and masks the phase without
                        // validating their ranges. Reject truncation/phase reinterpretation.
                        dng_memory_stream data(host.Allocator());
                        opcode.PutData(data);
                        data.Flush();
                        data.SetReadPosition(0);
                        require(data.Length() == 12 && data.Get_uint32() == 8,
                                "Invalid bad-pixel parameters");
                        const uint32 marker = data.Get_uint32();
                        const uint32 phase = data.Get_uint32();
                        require(marker <= 65535 && phase < 4,
                                "Invalid bad-pixel marker or Bayer phase");
                        // Existing original-limit flags can carry sentinel invalidity only
                        // outside the valid capture range; in-range markers need a new state.
                        require(
                            double(marker) <= source.view.black ||
                                double(marker) >= source.view.white,
                            "In-range bad-pixel markers are outside the supported capture domain");
                    }
                    if (!opcode.IsNOP()) {
                        // Warps have cubic absolute-weight norm <=1.890625; bad-pixel repair
                        // averages original neighbours with <=0.5 sensor-code rounding error.
                        // A factor of two conservatively covers either operation.
                        amplification *= 2.0;
                    }
                }
            }
        }
    }
    source.view.input_floor =
        std::max(1.0 / 65535.0, amplification / (source.view.white - source.view.black));
    require(std::isfinite(source.view.input_floor) && source.view.input_floor <= 256.0 / 65535.0,
            "DNG source precision exceeds the supported processing floor");
}

void color_basis(dng_negative &negative, const dng_vector &as_shot,
                 const AntitoneDngOptions &options, Source &source) {
    auto &v = source.view;
    const bool automatic = options.wb[0] == 0 && options.wb[1] == 0 && options.wb[2] == 0;
    if (automatic) {
        require(as_shot.Count() == 3, "Prototype requires AsShotNeutral or explicit fixed WB");
    }
    if (as_shot.Count() == 3) {
        double minimum = std::numeric_limits<double>::infinity();
        for (uint32 p = 0; p < 3; ++p) {
            const double value = 1.0 / as_shot[p];
            require(std::isfinite(value) && value > 0, "Invalid as-shot camera WB");
            v.as_shot_wb[p] = value;
            minimum = std::min(minimum, value);
        }
        for (double &value : v.as_shot_wb) {
            value /= minimum;
        }
    }
    for (uint32 p = 0; p < 3; ++p) {
        // dng_negative rounds its camera neutral to six decimals. Use the parsed tag
        // for automatic selection so this additional rounding does not alter fixed gains.
        v.wb[p] = automatic ? 1.0 / as_shot[p] : options.wb[p];
        require(std::isfinite(v.wb[p]) && v.wb[p] > 0, "Invalid fixed camera WB");
    }
    const double minimum = *std::min_element(v.wb, v.wb + 3);
    dng_vector neutral(3);
    for (uint32 p = 0; p < 3; ++p) {
        v.wb[p] /= minimum;
        require(v.wb[p] <= 256, "Fixed WB exceeds prototype camera range");
        neutral[p] = 1.0 / v.wb[p];
    }
    AutoPtr<dng_color_spec> spec(negative.MakeColorSpec(dng_camera_profile_id()));
    spec->SetWhiteXY(spec->NeutralToXY(neutral));
    const auto &matrix = spec->CameraToPCS();
    require(matrix.Rows() == 3 && matrix.Cols() == 3, "Invalid camera color matrix");
    // CameraToPCS already includes chromatic adaptation/WB. Remove the explicit
    // post-opcode gains from its columns to avoid applying them a second time.
    for (uint32 row = 0; row < 3; ++row) {
        for (uint32 col = 0; col < 3; ++col) {
            const double entry = matrix[row][col] / v.wb[col];
            require(std::isfinite(entry), "Nonfinite camera color matrix");
            v.camera_to_xyz_d50[row * 3 + col] = entry;
        }
    }
    // Viewing whites use the profile's own adaptation, independently of the
    // development neutral. These matrices consume the already WB-scaled buffer.
    const auto viewing_matrix = [&](const dng_xy_coord &white, double *output) {
        spec->SetWhiteXY(white);
        const auto &view = spec->CameraToPCS();
        for (uint32 row = 0; row < 3; ++row) {
            for (uint32 col = 0; col < 3; ++col) {
                output[row * 3 + col] = view[row][col] / v.wb[col];
            }
        }
    };
    viewing_matrix(D65_xy_coord(), v.daylight_to_xyz_d50);
    if (as_shot.Count() == 3) {
        viewing_matrix(spec->NeutralToXY(as_shot), v.as_shot_to_xyz_d50);
    }
    const auto *m = v.camera_to_xyz_d50;
    const double determinant = m[0] * (m[4] * m[8] - m[5] * m[7]) -
                               m[1] * (m[3] * m[8] - m[5] * m[6]) +
                               m[2] * (m[3] * m[7] - m[4] * m[6]);
    require(std::isfinite(determinant) && std::abs(determinant) > 1e-12,
            "Singular camera color matrix");
}

// The SDK's narrow path goes through the ANSI code page on Windows, which cannot represent most
// non-ASCII names; the wide constructor keeps UTF-8 paths intact.
static std::wstring wide_path(const char *path) {
    const int length = MultiByteToWideChar(CP_UTF8, 0, path, -1, nullptr, 0);
    if (length <= 0) {
        throw std::runtime_error("Source path is not valid UTF-8");
    }
    std::wstring wide(static_cast<size_t>(length), wchar_t(0));
    MultiByteToWideChar(CP_UTF8, 0, path, -1, wide.data(), length);
    wide.resize(static_cast<size_t>(length - 1));
    return wide;
}

// One constructor per platform: the SDK needs the wide name on Windows.
static std::unique_ptr<dng_file_stream> open_source(const char *path) {
#if defined(_WIN32)
    return std::make_unique<dng_file_stream>(wide_path(path).c_str());
#else
    return std::make_unique<dng_file_stream>(path);
#endif
}

std::unique_ptr<Source> develop(const char *path, const AntitoneDngOptions &options,
                                bool inspect_only) {
    const auto start = Clock::now();
    require(options.threads >= 1 && options.threads <= 8,
            "SDK execution budget must be 1..8 threads");
    antitone_dng::Execution execution(options.threads);
    Sniffer sniffer(options, execution);
    sniffer.Sniff();
    auto &global = runtime();
    std::lock_guard<std::mutex> lock(global.jobs);
    sniffer.Sniff();
    auto source = std::make_unique<Source>();
    Host host(sniffer, *source, execution);
    auto stream_owner = open_source(path);
    dng_file_stream &stream = *stream_owner;
    dng_info info;
    info.Parse(host, stream);
    info.PostParse(host);
    require(info.IsValidDNG() && info.fMainIndex >= 0 && size_t(info.fMainIndex) < info.fIFD.size(),
            "Invalid DNG");
    require(info.fMaskIndex < 0 && info.fDepthIndex < 0 && info.fEnhancedIndex < 0 &&
                info.fGainMapIndex < 0,
            "Auxiliary image processing is outside the prototype domain");
    AutoPtr<dng_negative> negative(host.Make_dng_negative());
    negative->Parse(host, stream, info);
    negative->PostParse(host, stream, info);
    admit(*info.fIFD[info.fMainIndex], *negative.Get(), options, *source);
    color_basis(*negative.Get(), info.fShared->fAsShotNeutral, options, *source);
    if (inspect_only) {
        negative->ReadOpcodeLists(host, stream, info);
        admit_opcodes(host, *negative.Get(), *source);
        source->view.total_seconds = elapsed(start);
        source->finish();
        return source;
    }
    negative->ReadStage1Image(host, stream, info);
    const auto &stage1 = *negative->Stage1Image();
    require(stage1.Planes() == 1 &&
                stage1.Bounds() == dng_rect(source->view.raw_height, source->view.raw_width),
            "Unexpected original capture image");
    source->original = pixels<uint16_t>(stage1, ttShort);
    host.capture_sites();
    admit_opcodes(host, *negative.Get(), *source);
    negative->BuildStage2Image(host);
    require(negative->Stage3BlackLevel() == 0,
            "Preserved-black development is outside the prototype");
    host.prepare_bilinear(*negative->Stage2Image());
    negative->BuildStage3Image(host);
    require(negative->Stage3Gain() == 1, "Unexpected SDK camera gain");
    const auto &stage3 = *negative->Stage3Image();
    require(stage3.Planes() == 3 &&
                stage3.Bounds() == dng_rect(source->view.height, source->view.width),
            "Unexpected developed camera geometry");
    source->rgb = pixels<float>(stage3, ttFloat);
    for (size_t i = 0; i < source->rgb.size(); ++i) {
        if (i % (size_t(source->view.width) * 3) == 0) {
            sniffer.Sniff();
        }
        const double value = double(source->rgb[i]) * source->view.wb[i % 3];
        require(std::isfinite(value) && value >= 0 && value <= 256,
                "Developed sample outside the prototype camera range");
        source->rgb[i] = float(value);
    }
    sniffer.Sniff();
    source->view.total_seconds = elapsed(start);
    source->finish();
    return source;
}
} // namespace

static int complete(const char *path, const AntitoneDngOptions *options, void **owner,
                    AntitoneDngView *view, char *error, size_t error_size, bool inspect_only) {
    antitone::NativeLog diagnostic("dng", inspect_only ? "inspect" : "develop");
    if (owner) {
        *owner = nullptr;
    }
    if (view) {
        *view = {};
    }
    try {
        require(path && options && owner && view, "Invalid DNG adapter arguments");
        auto source = develop(path, *options, inspect_only);
        *view = source->view;
        *owner = source.release();
        diagnostic.complete();
        return 0;
    } catch (const dng_exception &exception) {
        const bool cancelled = exception.ErrorCode() == dng_error_user_canceled;
        if (error && error_size) {
            std::snprintf(error, error_size, "%s (SDK code %u)",
                          cancelled ? "DNG development cancelled" : "DNG SDK rejected development",
                          exception.ErrorCode());
        }
        diagnostic.complete(cancelled ? 1 : 2, cancelled ? 3 : 2);
        return cancelled ? 1 : 2;
    } catch (const std::exception &exception) {
        diagnostic.complete(2, 2);
        if (error && error_size) {
            std::snprintf(error, error_size, "%s", exception.what());
        }
        return 2;
    } catch (...) {
        diagnostic.complete(2, 2);
        if (error && error_size) {
            std::snprintf(error, error_size, "Unknown DNG development failure");
        }
        return 2;
    }
}

extern "C" int antitone_dng_develop(const char *path, const AntitoneDngOptions *options,
                                    void **owner, AntitoneDngView *view, char *error,
                                    size_t error_size) {
    return complete(path, options, owner, view, error, error_size, false);
}

extern "C" int antitone_dng_inspect(const char *path, const AntitoneDngOptions *options,
                                    void **owner, AntitoneDngView *view, char *error,
                                    size_t error_size) {
    return complete(path, options, owner, view, error, error_size, true);
}

extern "C" void antitone_dng_close(void *owner) {
    delete static_cast<Source *>(owner);
}
