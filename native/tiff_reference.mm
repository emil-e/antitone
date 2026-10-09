#import <Foundation/Foundation.h>
#import <CoreGraphics/CoreGraphics.h>
#import <ImageIO/ImageIO.h>
#include <cstdio>

int main(int argc, char **argv) { @autoreleasepool {
    if (argc != 2) return 1;
    NSString *directory = [NSString stringWithUTF8String:argv[1]];
    for (NSString *name in @[@"display-p3", @"srgb"]) {
        NSString *path = [directory stringByAppendingPathComponent:[NSString stringWithFormat:@"DSC04416-%@.tif", name]];
        CGImageSourceRef source = CGImageSourceCreateWithURL((__bridge CFURLRef)[NSURL fileURLWithPath:path], nullptr);
        if (!source || CGImageSourceGetCount(source) != 1) return 2;
        CGImageRef image = CGImageSourceCreateImageAtIndex(source, 0, nullptr);
        if (!image || CGImageGetWidth(image) != 8704 || CGImageGetHeight(image) != 6144 || CGImageGetBitsPerComponent(image) != 16) return 3;
        CFDataRef profile = CGColorSpaceCopyICCData(CGImageGetColorSpace(image));
        NSData *expected = [NSData dataWithContentsOfFile:[NSString stringWithFormat:@"assets/profiles/antitone-%@-v1.icc", name]];
        if (!profile || !CFEqual(profile, (__bridge CFDataRef)expected)) return 4;
        CFDataRef pixels = CGDataProviderCopyData(CGImageGetDataProvider(image));
        if (!pixels || CFDataGetLength(pixels) < 8704L * 6144L * 6) return 5;
        std::printf("ImageIO TIFF target=%s size=%zux%zu bits=%zu decoded_bytes=%ld pinned_ICC=exact\n", name.UTF8String, CGImageGetWidth(image), CGImageGetHeight(image), CGImageGetBitsPerComponent(image), CFDataGetLength(pixels));
        CFRelease(pixels); CFRelease(profile); CGImageRelease(image); CFRelease(source);
    }
    return 0;
} }
