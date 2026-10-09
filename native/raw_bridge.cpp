#include "raw_bridge.h"
#include <libraw/libraw.h>
#include <algorithm>
#include <cstdio>
#include <memory>

static_assert(sizeof(AntitoneRawMetadata) == 496);
static_assert(sizeof(AntitoneRawDiagnostics) == 112);
static_assert(sizeof(AntitoneRawView) == 64);

namespace {
int check(int code, char *error, size_t size) {
    if (code) std::snprintf(error, size, "%s", LibRaw::strerror(code));
    return code;
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

extern "C" const char *antitone_raw_version() { return LibRaw::version(); }

extern "C" void *antitone_raw_open(const char *path, AntitoneRawMetadata *out, char *error, size_t size) try {
    auto raw = std::make_unique<LibRaw>();
    raw->imgdata.params.use_camera_matrix = 1;
    int code = raw->open_file(path);
    if (check(code, error, size)) return nullptr;
    if (raw->imgdata.idata.colors != 3 || !raw->imgdata.idata.filters || raw->imgdata.idata.filters == 9) {
        std::snprintf(error, size, "Initial developer requires a three-color Bayer RAW");
        return nullptr;
    }
    metadata(*raw, *out);
    return raw.release();
} catch (const std::bad_alloc &) {
    // Allocation failure must not unwind across the Rust C ABI.
    std::snprintf(error, size, "Unable to allocate LibRaw developer");
    return nullptr;
}

extern "C" int antitone_raw_unpack(void *handle, AntitoneRawDiagnostics *out, char *error, size_t size) {
    auto &raw = *static_cast<LibRaw *>(handle);
    int code = raw.unpack();
    if (check(code, error, size)) return code;
    const auto &d = raw.imgdata;
    const auto &s = d.sizes;
    const auto &c = d.color;
    if (!d.rawdata.raw_image || c.cblack[4] || c.cblack[5]) {
        std::snprintf(error, size, "Probe requires integer Bayer data without a repeated black pattern");
        return -1;
    }
    const auto pitch = s.raw_pitch / sizeof(uint16_t);
    for (unsigned y = 0; y < s.height; ++y) {
        for (unsigned x = 0; x < s.width; ++x) {
            const unsigned color = raw.COLOR(y, x);
            const auto value = d.rawdata.raw_image[(y + s.top_margin) * pitch + x + s.left_margin];
            const auto black = c.black + c.cblack[color];
            const auto white = c.linear_max[color] ? c.linear_max[color] : c.maximum;
            ++out->sites[color];
            out->below_black[color] += value <= black;
            out->at_white[color] += value >= white;
            out->peak[color] = std::max(out->peak[color], static_cast<uint32_t>(value));
        }
    }
    return 0;
}

extern "C" int antitone_raw_region_diagnostics(void *handle, uint32_t x0, uint32_t y0, uint32_t width, uint32_t height,
    AntitoneRawDiagnostics *out, char *error, size_t size) {
    const auto &d = static_cast<LibRaw *>(handle)->imgdata.rawdata;
    const auto &s = d.sizes;
    const auto &c = d.color;
    if (!d.raw_image || !width || !height || x0 >= s.width || y0 >= s.height
        || width > s.width - x0 || height > s.height - y0) {
        std::snprintf(error, size, "Invalid sensor sampling rectangle");
        return -1;
    }
    const auto pitch = s.raw_pitch / sizeof(uint16_t);
    for (unsigned y = y0; y < y0 + height; ++y) {
        for (unsigned x = x0; x < x0 + width; ++x) {
            // LibRaw's FC encoding, using the immutable RAW CFA rather than the processed green merge.
            const unsigned color = (d.iparams.filters >> ((((y << 1) & 14) + (x & 1)) << 1)) & 3;
            const auto value = d.raw_image[(y + s.top_margin) * pitch + x + s.left_margin];
            const auto white = c.linear_max[color] ? c.linear_max[color] : c.maximum;
            ++out->sites[color];
            out->below_black[color] += value <= c.black + c.cblack[color];
            out->at_white[color] += value >= white;
            out->peak[color] = std::max(out->peak[color], static_cast<uint32_t>(value));
        }
    }
    return 0;
}

extern "C" int antitone_raw_develop(void *handle, const double *wb, int output_color,
    AntitoneRawView *view, char *error, size_t size) {
    auto &raw = *static_cast<LibRaw *>(handle);
    auto &p = raw.imgdata.params;
    for (int c = 0; c < 4; ++c) p.user_mul[c] = static_cast<float>(wb[c]);
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
    if (check(code, error, size)) return code;
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
    for (int c = 0; c < 4; ++c) view->pre_mul[c] = d.color.pre_mul[c];
    return 0;
}

extern "C" void antitone_raw_close(void *handle) { delete static_cast<LibRaw *>(handle); }
