#pragma once
#include <stddef.h>
#include <stdint.h>

// The decode-only ABI consumed by pinned LibRaw 0.22.3's RawSpeed3 integration.
// Pixels belong to the handle until release, another decode, or close.
extern "C" {
enum rawspeed3_error_codes {
    rawspeed_inited = -2,
    rawspeed3_param_error = -1,
    rawspeed3_ok = 0,
    rawspeed3_ok_warnings = 1,
    rawspeed3_not_inited = 2,
    rawspeed3_processing_error = 3,
    rawspeed3_no_decoder = 4,
    rawspeed3_not_supported = 5,
};

using rawspeed3_handle_t = void *;

struct rawspeed3_ret_t {
    int status;
    uint16_t width, height, bpp, cpp;
    unsigned pitch, filters;
    const void *pixeldata;
};

void rawspeed3_clearresult(rawspeed3_ret_t *result);
rawspeed3_handle_t rawspeed3_initdefault();
int rawspeed3_decodefile(rawspeed3_handle_t handle, rawspeed3_ret_t *result, const void *data,
                         size_t size, bool allow_unknown);
void rawspeed3_release(rawspeed3_handle_t handle);
void rawspeed3_close(rawspeed3_handle_t handle);
}
