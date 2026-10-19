#pragma once
#include <stddef.h>
#include <stdint.h>

struct AntitoneDngOptions {
    uint64_t max_pixels;
    uint32_t threads; // Execution budget 1..8; does not change processing identity.
    // Zero selects AsShotNeutral. Otherwise positive gains, normalized by their minimum.
    double wb[3];
    // May be invoked concurrently; callback/context must support atomic read-only checks.
    int (*cancelled)(const void *);
    const void *cancel_context;
};

struct AntitoneDngView {
    const float *rgb;
    const uint16_t *original;
    const uint8_t *flags;
    const uint8_t *program;
    size_t program_size;
    uint32_t width, height, raw_width, raw_height;
    uint32_t active_left, active_top;
    uint32_t crop_left, crop_top, crop_width, crop_height, orientation, cfa_pattern;
    uint32_t applied[3], skipped[3];
    double black, white, wb[3], as_shot_wb[3], camera_to_xyz_d50[9], input_floor;
    uint64_t retained_bytes;
    // Wall timings are diagnostic only and do not participate in interpretation identity.
    double sdk_area_seconds, warp_sdk_seconds;
    double bad_pixel_mask_seconds, bilinear_mask_seconds, warp_mask_seconds, total_seconds;
    uint32_t sdk_workers, mask_workers;
    uint64_t warp_tile_scratch_bytes;
};

extern "C" {
// A completed owner contains immutable plain buffers, not a live SDK negative/host.
// Status 1 is cancellation, 2 is rejection/failure. No partial result is published.
int antitone_dng_develop(const char *path, const AntitoneDngOptions *options, void **owner,
                         AntitoneDngView *view, char *error, size_t error_size);
// Header/opcode inspection only: no sensor decode or RGB allocation.
int antitone_dng_inspect(const char *path, const AntitoneDngOptions *options, void **owner,
                         AntitoneDngView *view, char *error, size_t error_size);
void antitone_dng_close(void *owner);
}
