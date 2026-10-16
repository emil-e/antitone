// Bounded camera-code geometry only; no RAW development, repair or processing eligibility.
#import <Foundation/Foundation.h>
#include <algorithm>
#include <array>
#include <cmath>
#include <cstddef>
#include <cstdlib>
#include <cstring>
#include <vector>
#include "scan_region_lines.h"

extern "C" char *antitone_scan_region_lines(const uint16_t *input, unsigned w, unsigned h) {
    @autoreleasepool {
        try {
            if (!input || w < 16 || h < 16 || w > 768 || h > 768)
                return nullptr;
            std::vector<uint16_t> rgb(input, input + size_t(w) * h * 3);
            std::vector<float> gx(size_t(w) * h), gy(size_t(w) * h);
            for (unsigned y = 1; y + 1 < h; ++y)
                for (unsigned x = 1; x + 1 < w; ++x) {
                    const size_t i = size_t(y) * w + x;
                    double dx = 0, dy = 0;
                    for (unsigned c = 0; c < 3; ++c) {
                        const double a =
                            (int(rgb[(i + 1) * 3 + c]) - int(rgb[(i - 1) * 3 + c])) / 65535.;
                        const double b =
                            (int(rgb[(i + w) * 3 + c]) - int(rgb[(i - w) * 3 + c])) / 65535.;
                        dx += a * a;
                        dy += b * b;
                    }
                    gx[i] = std::sqrt(dx);
                    gy[i] = std::sqrt(dy);
                }
            NSDictionary *lines = @{
                @"left" :
                    orientedLines(gx, rgb, w, h, true, 1, w / 4, .1, .9, 96, 5, false, {}, true),
                @"right" : orientedLines(gx, rgb, w, h, true, w * 3 / 4, w - 1, .1, .9, 96, 5,
                                         false, {}, true),
                @"top" :
                    orientedLines(gy, rgb, w, h, false, 1, h / 4, .1, .9, 96, 5, false, {}, true),
                @"bottom" : orientedLines(gy, rgb, w, h, false, h * 3 / 4, h - 1, .1, .9, 96, 5,
                                          false, {}, true)
            };
            const auto poses = segmentPoseGuides(lines);
            NSDictionary *record = @{
                @"lines" : lines,
                @"segments" : @{
                    @"left" : segmentLines(gx, rgb, w, h, true, 1, w / 4, poses, true),
                    @"right" : segmentLines(gx, rgb, w, h, true, w * 3 / 4, w - 1, poses, true),
                    @"top" : segmentLines(gy, rgb, w, h, false, 1, h / 4, poses, true),
                    @"bottom" : segmentLines(gy, rgb, w, h, false, h * 3 / 4, h - 1, poses, true)
                }
            };
            NSError *error = nil;
            NSData *json = [NSJSONSerialization dataWithJSONObject:record options:0 error:&error];
            if (!json)
                return nullptr;
            auto *result = static_cast<char *>(std::malloc(json.length + 1));
            if (!result)
                return nullptr;
            std::memcpy(result, json.bytes, json.length);
            result[json.length] = 0;
            return result;
        } catch (...) {
            return nullptr;
        }
    }
}

extern "C" void antitone_scan_region_lines_free(char *data) {
    std::free(data);
}
