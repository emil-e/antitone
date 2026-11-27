#pragma once
#include <stddef.h>
#include <stdint.h>

struct AntitoneRawSonyLens {
    uint32_t present, invalid, distortion_length, ca_length, vignetting_length;
    uint16_t distortion_setting, ca_setting;
    int16_t distortion[17], ca[33], vignetting[17];
};

struct AntitoneRawPanasonicLens {
    uint32_t status;                      // 0: absent, 1: captured, 2: malformed
    uint16_t words[16], sensor_bounds[4]; // Bounds in top, left, bottom, right order.
};

struct AntitoneRawVendorMetadata {
    uint32_t sony_file_type;
    AntitoneRawSonyLens sony_lens;
    AntitoneRawPanasonicLens panasonic_lens;
};

struct AntitoneRawMetadata {
    char make[64], model[64], decoder[128];
    uint32_t width, height, raw_width, raw_height, left_margin, top_margin;
    uint32_t inset_left, inset_top, inset_width, inset_height;
    int32_t flip;
    uint32_t colors, filters, raw_bps, black, maximum;
    uint32_t cblack[4], linear_max[4];
    double cam_mul[4], daylight_mul[4], rgb_cam[9];
    AntitoneRawVendorMetadata vendor;
    uint32_t lens_disabled;
    uint32_t as_shot_wb_applied;
    char capture_time[20];
};

struct AntitoneRawView {
    const uint16_t *pixels;
    const float *rgb; // Optional continuous camera RGB, three floats per pixel.
    uint32_t width, height, maximum, data_maximum, warnings;
    uint32_t custom_demosaic;
    double pre_mul[4];
};

// Borrowed only during development. Success replaces the complete prepared RGBG image;
// failure must leave it unchanged so LibRaw can demosaic on the CPU.
struct AntitoneRawDemosaicer {
    void *context;
    int (*run)(void *context, uint16_t *image, uint32_t width, uint32_t height,
               const uint8_t *bayer);
};

struct AntitoneRawThumbnail {
    const uint8_t *data;
    uint32_t bytes, width, height;
    int32_t flip, color_space;
    uint32_t format;
};

extern "C" {
const char *antitone_raw_version();
void *antitone_raw_open(const char *path, AntitoneRawMetadata *metadata, char *error,
                        size_t error_size);
int antitone_raw_thumbnail(void *handle, AntitoneRawThumbnail *thumbnail, char *error,
                           size_t error_size);
int antitone_raw_is_dng(const void *handle);
int antitone_raw_unpack(void *handle, int threads, char *error, size_t error_size);
const char *antitone_raw_unpack_backend(const void *handle);
int antitone_raw_unpack_status(const void *handle);
int antitone_raw_develop(void *handle, const double *wb, int output_color, int threads,
                         AntitoneRawView *view, char *error, size_t error_size);
int antitone_raw_develop_with_demosaic(void *handle, const double *wb, int output_color,
                                       int threads, const AntitoneRawDemosaicer *demosaicer,
                                       AntitoneRawView *view, char *error, size_t error_size);
// Snapshot explicit owner/buffer allocations before freeze; zero means unavailable.
uint64_t antitone_raw_retained_bytes(void *handle);
// Release LibRaw's developed RGB after materializing corrected camera RGB.
void antitone_raw_free_image(void *handle);
void antitone_raw_close(void *handle);
}
