#include "raw_bridge.h"
#include <libraw/libraw.h>
#include <algorithm>
#include <array>
#include <cstdio>
#if defined(__APPLE__)
#include <malloc/malloc.h>
#elif defined(__linux__) || defined(_WIN32)
#include <malloc.h>
#endif
#include <memory>

static_assert(sizeof(AntitoneRawMetadata) == 664);
static_assert(sizeof(AntitoneRawSonyLens) == 160);
static_assert(sizeof(AntitoneRawDiagnostics) == 112);
static_assert(sizeof(AntitoneRawView) == 64);
static_assert(sizeof(AntitoneRawThumbnail) == 32);

namespace {
int check(int code, char *error, size_t size) {
    if (code)
        std::snprintf(error, size, "%s", LibRaw::strerror(code));
    return code;
}

// OpenMP ICVs belong to the calling task; restore them rather than changing process policy.
class ThreadBudget {
    int threads = omp_get_max_threads();
    int dynamic = omp_get_dynamic();
    int blocktime = kmp_get_blocktime();

public:
    explicit ThreadBudget(int count) {
        omp_set_dynamic(0);
        omp_set_num_threads(count);
        kmp_set_blocktime(0); // Sleeping workers must not compete with UI work after a tile/job.
    }

    ~ThreadBudget() {
        kmp_set_blocktime(blocktime);
        omp_set_num_threads(threads);
        omp_set_dynamic(dynamic);
    }
};

int check_threads(int threads, char *error, size_t size) {
    if (threads >= 1 && threads <= 8)
        return 0;
    std::snprintf(error, size, "RAW thread budget must be between 1 and 8");
    return -1;
}

// Capture limits use the immutable RAW levels, before WB/scaling/demosaicing.
uint8_t capture_flags(const libraw_colordata_t &color, unsigned channel, uint16_t value) {
    const auto white = color.linear_max[channel] ? color.linear_max[channel] : color.maximum;
    return uint8_t(value >= white) | (uint8_t(value <= color.black + color.cblack[channel]) << 1);
}

enum SonyLensField : uint32_t {
    DistortionParameters = 1,
    CaParameters = 2,
    DistortionSetting = 4,
    CaSetting = 8,
    VignettingParameters = 16,
};

struct SonyLensPage {
    AntitoneRawSonyLens lens{};
    INT64 data_offset = -1;
    uint32_t photometric = 0;
};

struct SonyLensCapture {
    std::array<SonyLensPage, LIBRAW_IFD_MAXCOUNT> pages{};
};

bool read_words(LibRaw_abstract_datastream &stream, unsigned order, uint16_t *words, int count) {
    std::array<uint8_t, 66> bytes{};
    if (count < 1 || count > 33 || (order != 0x4949 && order != 0x4d4d))
        return false;
    const auto position = stream.tell();
    const auto size = stream.size();
    if (position < 0 || position > size || count * 2 > size - position)
        return false;
    const auto read = stream.read(bytes.data(), 1, count * 2);
    const auto restored = stream.seek(position, SEEK_SET);
    if (read != count * 2 || restored != 0)
        return false;
    for (int i = 0; i < count; ++i) {
        const auto a = bytes[i * 2], b = bytes[i * 2 + 1];
        words[i] = order == 0x4949 ? uint16_t(a | (b << 8)) : uint16_t((a << 8) | b);
    }
    return true;
}

bool read_unsigned(LibRaw_abstract_datastream &stream, int type, unsigned order, uint32_t &value) {
    uint16_t words[2]{};
    if ((type != 3 && type != 4) || !read_words(stream, order, words, type == 3 ? 1 : 2))
        return false;
    if (type == 3)
        value = words[0];
    else if (order == 0x4949)
        value = uint32_t(words[0]) | (uint32_t(words[1]) << 16);
    else
        value = (uint32_t(words[0]) << 16) | words[1];
    return true;
}

void sony_lens_tag(void *context, int encoded_tag, int type, int count, unsigned order, void *input,
                   INT64 base) {
    // Pinned LibRaw encodes TIFF IFD indexes above bit 20; EXIF/maker-note paths use other bits.
    const auto tag = uint32_t(encoded_tag);
    const auto ifd = tag >> 20;
    if (!ifd || ifd > LIBRAW_IFD_MAXCOUNT || (tag & 0xf0000))
        return;
    auto &page = static_cast<SonyLensCapture *>(context)->pages[ifd - 1];
    auto &stream = *static_cast<LibRaw_abstract_datastream *>(input);
    const auto id = tag & 0xffff;
    uint32_t value = 0;
    if (id == 0x0106 && count == 1 && read_unsigned(stream, type, order, value))
        page.photometric = value;
    if (id == 0x0111 && count > 0 && read_unsigned(stream, type, order, value) && base >= 0 &&
        base <= stream.size() && value <= uint64_t(stream.size() - base))
        page.data_offset = base + value;
    // LibRaw selects the tile-offset table itself for tiled Sony lossless-compressed RAWs.
    if (id == 0x0144 && type == 4 && count > 0) {
        if (count > 1)
            page.data_offset = stream.tell();
        else if (read_unsigned(stream, type, order, value))
            page.data_offset = value;
    }
    uint32_t bit;
    switch (id) {
    case 0x7032:
        bit = VignettingParameters;
        break;
    case 0x7037:
        bit = DistortionParameters;
        break;
    case 0x7035:
        bit = CaParameters;
        break;
    case 0x7036:
        bit = DistortionSetting;
        break;
    case 0x7034:
        bit = CaSetting;
        break;
    default:
        return;
    }
    auto &lens = page.lens;
    if (lens.present & bit)
        lens.invalid |= bit;
    lens.present |= bit;
    if (bit == DistortionSetting || bit == CaSetting) {
        if (count != 1 || type != 3 || !read_unsigned(stream, type, order, value))
            lens.invalid |= bit;
        else if (bit == DistortionSetting)
            lens.distortion_setting = uint16_t(value);
        else
            lens.ca_setting = uint16_t(value);
        return;
    }
    uint16_t words[33]{};
    const auto capacity = bit == CaParameters ? 33 : 17;
    if (type != 8 || count < 1 || count > capacity || !read_words(stream, order, words, count)) {
        lens.invalid |= bit;
        return;
    }
    auto *target = bit == DistortionParameters ? lens.distortion
                   : bit == CaParameters       ? lens.ca
                                               : lens.vignetting;
    for (int i = 0; i < count; ++i)
        target[i] = int16_t(words[i] <= 32767 ? int32_t(words[i]) : int32_t(words[i]) - 65536);
    auto &length = bit == DistortionParameters ? lens.distortion_length
                   : bit == CaParameters       ? lens.ca_length
                                               : lens.vignetting_length;
    length = uint32_t(count);
}

AntitoneRawSonyLens selected_sony_lens(LibRaw &raw, const SonyLensCapture &capture) {
    if (raw.imgdata.idata.maker_index != LIBRAW_CAMERAMAKER_Sony || raw.imgdata.idata.dng_version)
        return {};
    const auto offset = raw.get_internal_data_pointer()->unpacker_data.data_offset;
    const AntitoneRawSonyLens *selected = nullptr;
    for (const auto &page : capture.pages) {
        if (page.photometric != 32803 || page.data_offset != offset)
            continue;
        if (selected) {
            AntitoneRawSonyLens ambiguous{};
            ambiguous.invalid = DistortionParameters | CaParameters | DistortionSetting |
                                CaSetting | VignettingParameters;
            return ambiguous;
        }
        selected = &page.lens;
    }
    return selected ? *selected : AntitoneRawSonyLens{};
}

void metadata(LibRaw &raw, AntitoneRawMetadata &out) {
    const auto &d = raw.imgdata;
    const auto &s = d.sizes;
    const auto &c = d.color;
    const auto &inset = s.raw_inset_crops[0];
    libraw_decoder_info_t decoder{};
    raw.get_decoder_info(&decoder);
    std::snprintf(out.make, sizeof out.make, "%s", d.idata.make);
    std::snprintf(out.model, sizeof out.model, "%s", d.idata.model);
    std::snprintf(out.decoder, sizeof out.decoder, "%s", decoder.decoder_name);
    out.lens_disabled = 0;
    out.width = s.width;
    out.height = s.height;
    out.raw_width = s.raw_width;
    out.raw_height = s.raw_height;
    out.left_margin = s.left_margin;
    out.top_margin = s.top_margin;
    out.inset_left = inset.cleft;
    out.inset_top = inset.ctop;
    out.inset_width = inset.cwidth;
    out.inset_height = inset.cheight;
    out.flip = s.flip;
    out.colors = d.idata.colors;
    out.filters = d.idata.filters;
    out.raw_bps = c.raw_bps;
    out.sony_file_type = d.makernotes.sony.SonyRawFileType;
    out.black = c.black;
    out.maximum = c.maximum;
    for (int i = 0; i < 4; ++i) {
        out.cblack[i] = c.cblack[i];
        out.linear_max[i] = c.linear_max[i];
        out.cam_mul[i] = c.cam_mul[i];
        out.daylight_mul[i] = c.pre_mul[i];
    }
    // The bridge uses column-major storage, matching glam. rgb_cam is forward camera -> sRGB.
    for (int row = 0; row < 3; ++row)
        for (int col = 0; col < 3; ++col)
            out.rgb_cam[col * 3 + row] = c.rgb_cam[row][col];
}
}

extern "C" uint64_t antitone_raw_retained_bytes(void *handle) {
#if defined(__APPLE__) || defined(__linux__) || defined(_WIN32)
    auto &raw = *static_cast<LibRaw *>(handle);
    const auto &d = raw.imgdata;
    const auto &internal = *raw.get_internal_data_pointer();
    // Match the explicit allocations released by pinned LibRaw::recycle, plus the owner.
    // SDK-private pools, datastream/stdlib storage and allocator-held VM are not measured.
    std::array<const void *, 11 + LIBRAW_AFDATA_MAXCOUNT> blocks = {
        handle,
        d.image,
        d.rawdata.raw_alloc,
        d.thumbnail.thumb,
        internal.internal_data.meta_data,
        internal.output_data.histogram,
        internal.output_data.oprof,
        d.color.profile,
        d.rawdata.ph1_cblack,
        d.rawdata.ph1_rblack,
        d.idata.xmpdata};
    for (size_t i = 0; i < LIBRAW_AFDATA_MAXCOUNT; ++i)
        blocks[11 + i] = d.makernotes.common.afdata[i].AFInfoData;
    uint64_t total = 0;
    for (size_t i = 0; i < blocks.size(); ++i) {
        const auto *block = blocks[i];
        if (!block || std::find(blocks.begin(), blocks.begin() + i, block) != blocks.begin() + i)
            continue;
#if defined(__APPLE__)
        const auto bytes = malloc_size(block);
#elif defined(__linux__)
        const auto bytes = malloc_usable_size(const_cast<void *>(block));
#else
        const auto bytes = _msize(const_cast<void *>(block));
#endif
        if (!bytes || bytes == SIZE_MAX || bytes > UINT64_MAX - total)
            return 0;
        total += bytes;
    }
    return total;
#else
    return 0;
#endif
}

extern "C" void antitone_raw_free_image(void *handle) {
    static_cast<LibRaw *>(handle)->free_image();
}

extern "C" const char *antitone_raw_version() {
    return LibRaw::version();
}

extern "C" void *antitone_raw_open(const char *path, AntitoneRawMetadata *out, char *error,
                                   size_t size) try {
    auto raw = std::make_unique<LibRaw>();
    raw->imgdata.params.use_camera_matrix = 1;
    SonyLensCapture lens;
    raw->set_exifparser_handler(sony_lens_tag, &lens);
    int code = raw->open_file(path);
    raw->set_exifparser_handler(nullptr, nullptr);
    if (check(code, error, size))
        return nullptr;
    if (raw->imgdata.idata.colors != 3 || !raw->imgdata.idata.filters ||
        raw->imgdata.idata.filters == 9) {
        std::snprintf(error, size, "Initial developer requires a three-color Bayer RAW");
        return nullptr;
    }
    metadata(*raw, *out);
    out->sony_lens = selected_sony_lens(*raw, lens);
    return raw.release();
} catch (const std::bad_alloc &) {

    // Allocation failure must not unwind across the Rust C ABI.
    std::snprintf(error, size, "Unable to allocate LibRaw developer");
    return nullptr;
}

extern "C" int antitone_raw_thumbnail(void *handle, AntitoneRawThumbnail *out, char *error,
                                      size_t size) try {
    *out = {};
    auto &raw = *static_cast<LibRaw *>(handle);
    const auto &list = raw.imgdata.thumbs_list;
    int selected = -1;
    uint64_t selected_area = 0;
    bool sufficient = false;
    // Tiny EXIF thumbnails can letterbox a 3:2 camera image into a 4:3 canvas.
    // Prefer enough resolution for a 256-pixel filmstrip proxy before minimizing decode cost.
    constexpr unsigned preview_edge = 256;
    for (int index = 0; index < list.thumbcount; ++index) {
        const auto &item = list.thumblist[index];
        if (item.tformat != LIBRAW_INTERNAL_THUMBNAIL_JPEG || !item.twidth || !item.theight ||
            item.twidth > 2048 || item.theight > 2048 || !item.tlength ||
            item.tlength > 1024 * 1024)
            continue;
        const bool enough = item.twidth >= preview_edge || item.theight >= preview_edge;
        const uint64_t area = uint64_t(item.twidth) * item.theight;
        const bool better_size = enough ? area < selected_area : area > selected_area;
        if (selected < 0 || (enough && !sufficient) || (enough == sufficient && better_size)) {
            selected = index;
            selected_area = area;
            sufficient = enough;
        }
    }
    if (selected < 0)
        return 0;
    int code = raw.unpack_thumb_ex(selected);
    if (check(code, error, size))
        return code;
    const auto &thumb = raw.imgdata.thumbnail;
    if (thumb.tformat != LIBRAW_THUMBNAIL_JPEG || !thumb.thumb || !thumb.tlength ||
        thumb.tlength > 1024 * 1024 || !thumb.twidth || !thumb.theight || thumb.twidth > 2048 ||
        thumb.theight > 2048)
        return 0;
    // Borrowed until the handle's next thumbnail extraction or close; Rust copies before either.
    out->jpeg = reinterpret_cast<const uint8_t *>(thumb.thumb);
    out->bytes = thumb.tlength;
    out->width = thumb.twidth;
    out->height = thumb.theight;
    out->flip = list.thumblist[selected].tflip == 0xffff ? raw.imgdata.sizes.flip
                                                         : list.thumblist[selected].tflip;
    out->color_space = raw.imgdata.color.ExifColorSpace;
    return 0;
} catch (const std::bad_alloc &) {

    std::snprintf(error, size, "Unable to allocate embedded JPEG preview");
    return -1;
}

extern "C" int antitone_raw_is_dng(const void *handle) {
    return static_cast<const LibRaw *>(handle)->imgdata.idata.dng_version != 0;
}

extern "C" int antitone_raw_unpack(void *handle, int threads, char *error, size_t size) {
    if (check_threads(threads, error, size))
        return -1;
    ThreadBudget budget(threads);
    auto &raw = *static_cast<LibRaw *>(handle);
    int code = raw.unpack();
    if (check(code, error, size))
        return code;
    const auto &d = raw.imgdata.rawdata;
    if (!d.raw_image || d.color.cblack[4] || d.color.cblack[5]) {
        std::snprintf(error, size,
                      "Probe requires integer Bayer data without a repeated black pattern");
        return -1;
    }
    return 0;
}

extern "C" int antitone_raw_sensor_diagnostics(const void *handle, AntitoneRawDiagnostics *out,
                                               char *error, size_t size) {
    const auto &s = static_cast<const LibRaw *>(handle)->imgdata.rawdata.sizes;
    return antitone_raw_region_diagnostics(handle, 0, 0, s.width, s.height, out, error, size);
}

extern "C" int antitone_raw_region_diagnostics(const void *handle, uint32_t x0, uint32_t y0,
                                               uint32_t width, uint32_t height,
                                               AntitoneRawDiagnostics *out, char *error,
                                               size_t size) {
    const auto &d = static_cast<const LibRaw *>(handle)->imgdata.rawdata;
    const auto &s = d.sizes;
    const auto &c = d.color;
    if (!d.raw_image || !width || !height || x0 >= s.width || y0 >= s.height ||
        width > s.width - x0 || height > s.height - y0) {
        std::snprintf(error, size, "Invalid sensor sampling rectangle");
        return -1;
    }
    const auto pitch = s.raw_pitch / sizeof(uint16_t);
    for (unsigned y = y0; y < y0 + height; ++y) {
        for (unsigned x = x0; x < x0 + width; ++x) {
            // LibRaw's FC encoding, using the immutable RAW CFA rather than the processed green merge.
            const unsigned color = (d.iparams.filters >> ((((y << 1) & 14) + (x & 1)) << 1)) & 3;
            const auto value = d.raw_image[(y + s.top_margin) * pitch + x + s.left_margin];
            const auto flags = capture_flags(c, color, value);
            ++out->sites[color];
            out->below_black[color] += (flags & 2) != 0;
            out->at_white[color] += (flags & 1) != 0;
            out->peak[color] = std::max(out->peak[color], static_cast<uint32_t>(value));
        }
    }
    return 0;
}

extern "C" int antitone_raw_capture_flags(const void *handle, uint64_t *words, size_t count,
                                          char *error, size_t size) {
    const auto &data = static_cast<const LibRaw *>(handle)->imgdata.rawdata;
    const auto &s = data.sizes;
    const size_t row_words = (size_t(s.width) + 31) / 32;
    if (!data.raw_image || !s.width || !s.height || !words || count != row_words * s.height) {
        std::snprintf(error, size, "Invalid capture mask dimensions");
        return -1;
    }
    const auto pitch = s.raw_pitch / sizeof(uint16_t);
    for (unsigned y = 0; y < s.height; ++y) {
        for (size_t word = 0; word < row_words; ++word) {
            uint64_t flags = 0;
            for (unsigned x = unsigned(word * 32); x < std::min(size_t(s.width), (word + 1) * 32);
                 ++x) {
                const unsigned channel =
                    (data.iparams.filters >> ((((y << 1) & 14) + (x & 1)) << 1)) & 3;
                const auto value = data.raw_image[(y + s.top_margin) * pitch + x + s.left_margin];
                flags |= uint64_t(capture_flags(data.color, channel, value)) << (2 * (x % 32));
            }
            words[y * row_words + word] = flags;
        }
    }
    return 0;
}

extern "C" int antitone_raw_develop(void *handle, const double *wb, int output_color, int threads,
                                    AntitoneRawView *view, char *error, size_t size) {
    if (check_threads(threads, error, size))
        return -1;
    ThreadBudget budget(threads);
    auto &raw = *static_cast<LibRaw *>(handle);
    auto &p = raw.imgdata.params;
    for (int c = 0; c < 4; ++c)
        p.user_mul[c] = static_cast<float>(wb[c]);
    p.output_color = output_color;
    p.output_bps = 16;
    p.gamm[0] = p.gamm[1] = 1.0;
    p.no_auto_bright = 1;
    p.adjust_maximum_thr = 0.0f;
    p.use_auto_wb = p.use_camera_wb = 0;
    p.no_auto_scale = 0;
    p.highlight = 1;
    p.user_qual = 3;
    p.half_size = 0;
    p.four_color_rgb = 0;
    p.med_passes = p.fbdd_noiserd = p.green_matching = 0;
    p.threshold = 0.0f;
    p.exp_correc = 0;
    p.user_flip = 0;
    p.use_fuji_rotate = 0;
    p.camera_profile = p.output_profile = nullptr;
    int code = raw.dcraw_process();
    if (check(code, error, size))
        return code;
    const auto &d = raw.imgdata;
    if (!d.image || d.idata.colors != 3) {
        std::snprintf(error, size, "Developer did not return a three-channel camera image");
        return -1;
    }
    view->pixels = reinterpret_cast<const uint16_t *>(d.image);
    view->width = d.sizes.width;
    view->height = d.sizes.height;
    view->maximum = d.color.maximum;
    view->data_maximum = d.color.data_maximum;
    view->warnings = d.process_warnings;
    for (int c = 0; c < 4; ++c)
        view->pre_mul[c] = d.color.pre_mul[c];
    return 0;
}

extern "C" void antitone_raw_close(void *handle) {
    delete static_cast<LibRaw *>(handle);
}
