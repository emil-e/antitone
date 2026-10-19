#pragma once
#include <stddef.h>
#include <stdint.h>

struct AntitoneRawSonyLens {
    uint32_t present, invalid, distortion_length, ca_length, vignetting_length;
    uint16_t distortion_setting, ca_setting;
    int16_t distortion[17], ca[33], vignetting[17];
};

struct AntitoneRawMetadata {
    char make[64], model[64], decoder[128];
    uint32_t width, height, raw_width, raw_height, left_margin, top_margin;
    uint32_t inset_left, inset_top, inset_width, inset_height;
    int32_t flip;
    uint32_t colors, filters, raw_bps, sony_file_type, black, maximum;
    uint32_t cblack[4], linear_max[4];
    double cam_mul[4], daylight_mul[4], rgb_cam[9];
    AntitoneRawSonyLens sony_lens;
    uint32_t lens_disabled;
};

struct AntitoneRawDiagnostics {
    uint64_t sites[4], below_black[4], at_white[4];
    uint32_t peak[4];
};

struct AntitoneRawView {
    const uint16_t *pixels;
    uint32_t width, height, maximum, data_maximum, warnings;
    double pre_mul[4];
};

struct AntitoneRawThumbnail {
    const uint8_t *jpeg;
    uint32_t bytes, width, height;
    int32_t flip, color_space;
};

extern "C" {
const char *antitone_raw_version();
void *antitone_raw_open(const char *path, AntitoneRawMetadata *metadata, char *error,
                        size_t error_size);
int antitone_raw_thumbnail(void *handle, AntitoneRawThumbnail *thumbnail, char *error,
                           size_t error_size);
int antitone_raw_unpack(void *handle, int threads, char *error, size_t error_size);
// Read already-unpacked capture data only; output buffers belong to the caller.
int antitone_raw_sensor_diagnostics(const void *handle, AntitoneRawDiagnostics *diagnostics,
                                    char *error, size_t error_size);
int antitone_raw_region_diagnostics(const void *handle, uint32_t x, uint32_t y, uint32_t width,
                                    uint32_t height, AntitoneRawDiagnostics *diagnostics,
                                    char *error, size_t error_size);
// Native visible coordinates, 32 pixels per u64, row-aligned; bit 0 white, bit 1 black per pixel.
int antitone_raw_capture_flags(const void *handle, uint64_t *words, size_t word_count, char *error,
                               size_t error_size);
int antitone_raw_develop(void *handle, const double *wb, int output_color, int threads,
                         AntitoneRawView *view, char *error, size_t error_size);
// Snapshot explicit owner/buffer allocations before freeze; zero means unavailable.
uint64_t antitone_raw_retained_bytes(void *handle);
// Release only developed RGB after correction; original sensor storage remains owned.
void antitone_raw_free_image(void *handle);
void antitone_raw_close(void *handle);
}
