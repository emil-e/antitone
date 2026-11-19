#pragma once
#include "rawspeed3_capi.h"

// LibRaw's interpretation of the full uncropped raster. The optional curve is
// borrowed from that owner for this unpack; RawSpeed must not substitute a
// dithered curve or its camera database's black/white/crop interpretation.
struct AntitoneRawSpeedRequest {
    uint32_t width, height, cpp, filters;
    const uint16_t *curve;
};

extern "C" {
void antitone_rawspeed_prepare(rawspeed3_handle_t handle, const AntitoneRawSpeedRequest *request);
int antitone_rawspeed_status(rawspeed3_handle_t handle);
uint64_t antitone_rawspeed_retained_bytes(rawspeed3_handle_t handle);
const char *antitone_rawspeed_version();
}
