// Independent Core Image/ColorSync check of both output ICCs, including toes and gamut corners.
// Consumes rendering-probe ICC/encoded/linear fixtures in .cache/probes; writes no artifacts.
// Fails on malformed inputs, rejected ICCs, nonfinite conversion or error above 2e-4.
// Tests numerical profile conversion, not a monitor or photographic appearance.
#import <Foundation/Foundation.h>
#import <CoreGraphics/CoreGraphics.h>
#import <CoreImage/CoreImage.h>
#include <algorithm>
#include <array>
#include <cmath>
#include <cstdio>
#include <fstream>
#include <stdexcept>
#include <string>
#include <vector>

using Pixels = std::vector<std::array<float, 4>>;
static Pixels read(const std::string &path) {
    std::ifstream stream(path, std::ios::binary | std::ios::ate);
    const auto bytes = stream.tellg();
    if (!stream || bytes <= 0 || bytes % sizeof(Pixels::value_type)) throw std::runtime_error("missing or truncated profile fixture");
    Pixels values(bytes / sizeof(Pixels::value_type));
    stream.seekg(0);
    if (!stream.read(reinterpret_cast<char *>(values.data()), bytes)) throw std::runtime_error("profile fixture read failed");
    return values;
}

int main() { @autoreleasepool { try {
    CGColorSpaceRef linear = CGColorSpaceCreateWithName(kCGColorSpaceExtendedLinearSRGB);
    if (!linear) return 1;
    CIContext *context = [CIContext contextWithOptions:@{
        kCIContextUseSoftwareRenderer: @YES,
        kCIContextWorkingColorSpace: (__bridge id)linear,
        kCIContextWorkingFormat: @(kCIFormatRGBAf)
    }];
    for (const auto name : {"srgb", "display-p3"}) {
        const std::string base = ".cache/probes/";
        auto values = read(base + name + "-profile-encoded.f32");
        auto expected = read(base + name + "-profile-linear-srgb.f32");
        if (values.size() != expected.size()) throw std::runtime_error("profile fixture length mismatch");
        NSString *path = [NSString stringWithUTF8String:(base + "antitone-" + name + "-v1.icc").c_str()];
        NSData *profile = [NSData dataWithContentsOfFile:path];
        if (!profile) throw std::runtime_error("missing output ICC");
        CGColorSpaceRef encoded = CGColorSpaceCreateWithICCData((__bridge CFDataRef)profile);
        if (!encoded || CGColorSpaceGetModel(encoded) != kCGColorSpaceModelRGB) throw std::runtime_error("ColorSync rejected output ICC");
        NSData *data = [NSData dataWithBytes:values.data() length:values.size() * sizeof(values[0])];
        CIImage *image = [CIImage imageWithBitmapData:data bytesPerRow:values.size() * sizeof(values[0])
            size:CGSizeMake(values.size(), 1) format:kCIFormatRGBAf colorSpace:encoded];
        if (!image) throw std::runtime_error("CoreImage rejected profile image");
        auto result = values;
        [context render:image toBitmap:result.data() rowBytes:result.size() * sizeof(result[0])
            bounds:image.extent format:kCIFormatRGBAf colorSpace:linear];
        float maximum = 0;
        for (size_t i = 0; i < values.size(); ++i) {
            for (int c = 0; c < 3; ++c) {
                const float actual = result[i][c];
                const float error = std::abs(actual - expected[i][c]);
                if (!std::isfinite(actual) || error > 2e-4f) {
                    std::fprintf(stderr, "%s ICC mismatch pixel=%zu channel=%d actual=%.8g expected=%.8g\n", name, i, c, actual, expected[i][c]);
                    return 2;
                }
                maximum = std::max(maximum, error);
            }
        }
        std::printf("ColorSync/CoreImage target=%s ICC_to_extended_linear_sRGB_max_error=%.8g pixels=%zu (toes and gamut corners included)\n", name, maximum, values.size());
        CGColorSpaceRelease(encoded);
    }
    CGColorSpaceRelease(linear);
    return 0;
} catch (const std::exception &e) {
    std::fprintf(stderr, "Profile reference: %s\n", e.what());
    return 3;
} } }
