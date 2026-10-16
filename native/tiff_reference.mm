// Independently reopens one TIFF via ImageIO with explicit expected dimensions and ICC bytes.
// Requires macOS frameworks; inputs are FILE WIDTH HEIGHT PROFILE. Writes no artifacts.
// Gates one-image TIFF, RGB16 decoding, exact dimensions/profile and sufficient decoded storage.
// Pixel agreement, photographic quality and physical display matching require separate checks.
#import <Foundation/Foundation.h>
#import <CoreGraphics/CoreGraphics.h>
#import <ImageIO/ImageIO.h>
#include <charconv>
#include <cstdio>
#include <cstring>
#include <limits>

static bool dimension(const char *argument, size_t &value) {
    const auto result = std::from_chars(argument, argument + std::strlen(argument), value);
    return result.ec == std::errc{} && *result.ptr == '\0' && value > 0;
}

template <typename T> struct ScopedCF {
    T value;

    ~ScopedCF() {
        if (value)
            CFRelease(value);
    }
};

int main(int argc, char **argv) {
    @autoreleasepool {
        size_t width = 0, height = 0;
        if (argc != 5 || !dimension(argv[2], width) || !dimension(argv[3], height) ||
            width > static_cast<size_t>(std::numeric_limits<CFIndex>::max()) / height / 6) {
            std::fprintf(
                stderr,
                "Usage: tiff-reference FILE.TIF WIDTH HEIGHT PROFILE.ICC (positive dimensions)\n");
            return 1;
        }
        NSString *path = [NSString stringWithUTF8String:argv[1]];
        NSData *expected = [NSData dataWithContentsOfFile:[NSString stringWithUTF8String:argv[4]]];
        if (!expected || expected.length == 0) {
            std::fprintf(stderr, "Expected ICC profile is missing or empty\n");
            return 1;
        }
        ScopedCF<CGImageSourceRef> source{
            CGImageSourceCreateWithURL((__bridge CFURLRef)[NSURL fileURLWithPath:path], nullptr)};
        if (!source.value || CGImageSourceGetCount(source.value) != 1 ||
            !CFEqual(CGImageSourceGetType(source.value), CFSTR("public.tiff"))) {
            std::fprintf(stderr, "Expected one-image TIFF\n");
            return 2;
        }
        ScopedCF<CGImageRef> image{CGImageSourceCreateImageAtIndex(source.value, 0, nullptr)};
        if (!image.value || CGImageGetWidth(image.value) != width ||
            CGImageGetHeight(image.value) != height ||
            CGImageGetBitsPerComponent(image.value) != 16 ||
            CGColorSpaceGetModel(CGImageGetColorSpace(image.value)) != kCGColorSpaceModelRGB ||
            CGColorSpaceGetNumberOfComponents(CGImageGetColorSpace(image.value)) != 3) {
            std::fprintf(stderr, "Expected dimensions and RGB16 decoding\n");
            return 3;
        }
        ScopedCF<CFDataRef> profile{CGColorSpaceCopyICCData(CGImageGetColorSpace(image.value))};
        if (!profile.value || !CFEqual(profile.value, (__bridge CFDataRef)expected)) {
            std::fprintf(stderr, "ICC profile differs from expected bytes\n");
            return 4;
        }
        ScopedCF<CFDataRef> pixels{CGDataProviderCopyData(CGImageGetDataProvider(image.value))};
        if (!pixels.value ||
            CFDataGetLength(pixels.value) < static_cast<CFIndex>(width * height * 6)) {
            std::fprintf(stderr, "Insufficient decoded RGB16 storage\n");
            return 5;
        }
        std::printf(
            "ImageIO TIFF file=%s size=%zux%zu bits=%zu decoded_bytes=%ld pinned_ICC=exact\n",
            argv[1], width, height, CGImageGetBitsPerComponent(image.value),
            CFDataGetLength(pixels.value));
        return 0;
    }
}
