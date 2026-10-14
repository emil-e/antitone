#pragma once
#include <stddef.h>
#include <stdint.h>

struct AntitoneRawMetadata {
    char make[64], model[64], decoder[128];
    uint32_t width, height, raw_width, raw_height, left_margin, top_margin;
    uint32_t inset_left, inset_top, inset_width, inset_height;
    int32_t flip;
    uint32_t colors, filters, raw_bps, sony_file_type, black, maximum;
    uint32_t cblack[4], linear_max[4];
    double cam_mul[4], daylight_mul[4], rgb_cam[9];
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
void *antitone_raw_open(const char *path, AntitoneRawMetadata *metadata, char *error, size_t error_size);
int antitone_raw_thumbnail(void *handle, AntitoneRawThumbnail *thumbnail, char *error, size_t error_size);
int antitone_raw_unpack(void *handle, int threads, char *error, size_t error_size);
// Read already-unpacked capture data only; output buffers belong to the caller.
int antitone_raw_sensor_diagnostics(const void *handle, AntitoneRawDiagnostics *diagnostics, char *error, size_t error_size);
int antitone_raw_region_diagnostics(const void *handle, uint32_t x, uint32_t y, uint32_t width, uint32_t height,
    AntitoneRawDiagnostics *diagnostics, char *error, size_t error_size);
int antitone_raw_develop(void *handle, const double *wb, int output_color, int threads,
    AntitoneRawView *view, char *error, size_t error_size);
// Snapshot explicit owner/buffer allocations before freeze; zero means unavailable.
uint64_t antitone_raw_retained_bytes(void *handle);
void antitone_raw_close(void *handle);
}
