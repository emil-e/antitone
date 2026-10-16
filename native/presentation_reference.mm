// Independent Core Image signed extended-sRGB transfer reference, including above-white values.
// Requires macOS frameworks and an existing .cache/probes; writes linear/encoded f32 fixtures.
// Fails on setup errors or maximum transfer error >= 3e-5; no window or photographs are needed.
// Does not establish physical-display matching.
#import <Foundation/Foundation.h>
#import <CoreGraphics/CoreGraphics.h>
#import <CoreImage/CoreImage.h>
#include <array>
#include <cmath>
#include <cstdio>
#include <fstream>
#include <vector>

static float encode(float x) {
    const float a = std::abs(x);
    return std::copysign(a <= .0031308f ? 12.92f * a : 1.055f * std::pow(a, 1.f / 2.4f) - .055f, x);
}

int main() {
    @autoreleasepool {
        CGColorSpaceRef linear = CGColorSpaceCreateWithName(kCGColorSpaceExtendedLinearSRGB);
        CGColorSpaceRef encoded = CGColorSpaceCreateWithName(kCGColorSpaceExtendedSRGB);
        if (!linear || !encoded)
            return 1;
        CIContext *context = [CIContext contextWithOptions:@{
            kCIContextUseSoftwareRenderer : @YES,
            kCIContextWorkingColorSpace : (__bridge id)linear,
            kCIContextWorkingFormat : @(kCIFormatRGBAf)
        }];
        std::vector<std::array<float, 4>> values;
        for (float x : {-1.f, -.5f, -.1f, -.0031308f, -.001f, 0.f, .001f, .0031308f, .01f, .18f,
                        .5f, 1.f, 1.5f, 2.f})
            values.push_back({x, .1f, .2f, 1.f});
        NSData *data = [NSData dataWithBytes:values.data()
                                      length:values.size() * sizeof(values[0])];
        CIImage *image = [CIImage imageWithBitmapData:data
                                          bytesPerRow:values.size() * sizeof(values[0])
                                                 size:CGSizeMake(values.size(), 1)
                                               format:kCIFormatRGBAf
                                           colorSpace:linear];
        auto result = values;
        [context render:image
               toBitmap:result.data()
               rowBytes:result.size() * sizeof(result[0])
                 bounds:image.extent
                 format:kCIFormatRGBAf
             colorSpace:encoded];
        float maximum = 0;
        for (size_t i = 0; i < values.size(); ++i) {
            std::printf("extended_srgb %.8g -> %.8g expected %.8g\n", values[i][0], result[i][0],
                        encode(values[i][0]));
            for (int c = 0; c < 3; ++c)
                maximum = std::max(maximum, std::abs(result[i][c] - encode(values[i][c])));
        }
        std::printf("CoreImage_ExtendedLinearSRGB_to_ExtendedSRGB_max_error=%.8g\n", maximum);
        std::ofstream input(".cache/probes/presentation-linear-srgb.f32", std::ios::binary);
        input.write(reinterpret_cast<const char *>(values.data()),
                    values.size() * sizeof(values[0]));
        std::ofstream output(".cache/probes/presentation-encoded-srgb-reference.f32",
                             std::ios::binary);
        output.write(reinterpret_cast<const char *>(result.data()),
                     result.size() * sizeof(result[0]));
        CGColorSpaceRelease(encoded);
        CGColorSpaceRelease(linear);
        return maximum < 3e-5f ? 0 : 3;
    }
}
