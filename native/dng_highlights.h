#pragma once
#include "dng_execution.h"
#include "dng_host.h"
#include "dng_image.h"
#include "dng_opcode_list.h"
#include "dng_opcodes.h"
#include "highlights.h"
#include <cstdint>
#include <vector>

namespace antitone_dng {

// The default SDK image lends writable storage. Jobs own disjoint row ranges;
// every worker joins before the stage continues or the image can be destroyed.
template <typename Function>
void highlight_rows(dng_host &host, dng_image &image, Execution &execution, Function process) {
    execution.run(execution.threads(), [&](uint32 worker) {
        const auto bounds = image.Bounds();
        const int32 top = bounds.t + int32(uint64(bounds.H()) * worker / execution.threads());
        const int32 bottom =
            bounds.t + int32(uint64(bounds.H()) * (worker + 1) / execution.threads());
        for (int32 y = top; y < bottom; y += 64) {
            host.SniffForAbort();
            dng_dirty_tile_buffer tile(image,
                                       dng_rect(y, bounds.l, std::min(y + 64, bottom), bounds.r));
            for (int32 row = tile.fArea.t; row < tile.fArea.b; ++row)
                for (int32 x = bounds.l; x < bounds.r; ++x)
                    process(tile, row, x, worker);
        }
    });
}

// Returns the temporary scale to undo during final WB/output. This protects our
// reconstructed overrange through SDK warps; it intentionally raises their effective
// positive ceiling in source units, without changing the SDK's opcode implementation.
inline double repair_highlights(dng_host &host, dng_image &image, Execution &execution,
                                const double *as_shot, const double *output_wb,
                                dng_opcode_list &later, double maximum_output) {
    // A requested output balance is not independent evidence of the source colour.
    // Missing/unsupported context must leave development usable, not invent a repair.
    for (unsigned c = 0; c < 3; ++c)
        if (!std::isfinite(as_shot[c]) || as_shot[c] <= 0)
            return 1.;
    unsigned warps = 0;
    for (uint32 i = 0; i < later.Count(); ++i)
        warps += later.Entry(i).OpcodeID() == 1 && !later.Entry(i).IsNOP();
    const double maximum_balance = std::max({as_shot[0], as_shot[1], as_shot[2]});
    const double optical_bound = std::pow(1.890625, warps);
    for (unsigned c = 0; c < 3; ++c)
        if (output_wb[c] * maximum_balance / as_shot[c] * optical_bound > maximum_output)
            return 1.;
    const std::array<float, 3> gains = {float(as_shot[0]), float(as_shot[1]), float(as_shot[2])};
    const antitone::HighlightBlend repair({1.f, 1.f, 1.f}, gains);
    if (!repair.supported())
        return 1.;
    std::vector<uint8_t> clipped(execution.threads());
    highlight_rows(host, image, execution,
                   [&](dng_pixel_buffer &tile, int32 y, int32 x, uint32 worker) {
                       std::array<float, 3> pixel;
                       for (unsigned c = 0; c < 3; ++c)
                           pixel[c] = *tile.ConstPixel_real32(y, x, c);
                       clipped[worker] |= repair.clipped(pixel);
                   });
    if (std::none_of(clipped.begin(), clipped.end(), [](uint8_t value) { return value != 0; }))
        return 1.;

    std::vector<uint64_t> changed(execution.threads());
    highlight_rows(host, image, execution,
                   [&](dng_pixel_buffer &tile, int32 y, int32 x, uint32 worker) {
                       std::array<float, 3> pixel;
                       for (unsigned c = 0; c < 3; ++c)
                           pixel[c] = *tile.ConstPixel_real32(y, x, c);
                       if (!repair.apply(pixel))
                           return;
                       ++changed[worker];
                       for (unsigned c = 0; c < 3; ++c)
                           *tile.DirtyPixel_real32(y, x, c) = pixel[c];
                   });
    if (std::all_of(changed.begin(), changed.end(), [](uint64_t n) { return n == 0; }))
        return 1.;
    if (!warps)
        return 1.;
    const float maximum = std::max({gains[0], gains[1], gains[2]});
    const float minimum = std::min({gains[0], gains[1], gains[2]});
    // Cubic absolute-weight norm <=1.890625. Power-of-two scaling itself is exact
    // for ordinary float values; lower clipping and interpolation rounding remain.
    const float scale =
        std::exp2(-std::ceil(std::log2(maximum / minimum) + float(warps) * std::log2(1.890625f)));
    highlight_rows(host, image, execution, [&](dng_pixel_buffer &tile, int32 y, int32 x, uint32) {
        for (unsigned c = 0; c < 3; ++c)
            *tile.DirtyPixel_real32(y, x, c) *= scale;
    });
    return scale;
}

} // namespace antitone_dng
