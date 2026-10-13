// Private source/result contacts for visual diagnostics; no colour-acceptance claim.
#import <Foundation/Foundation.h>
#import <ImageIO/ImageIO.h>
#import <CoreGraphics/CoreGraphics.h>
#import <CoreText/CoreText.h>

static CGImageRef thumbnail(NSString *path, int size) {
    CGImageSourceRef source = CGImageSourceCreateWithURL((__bridge CFURLRef)[NSURL fileURLWithPath:path], NULL);
    if (!source) return NULL;
    NSDictionary *options = @{
        (id)kCGImageSourceCreateThumbnailFromImageAlways: @YES,
        (id)kCGImageSourceCreateThumbnailWithTransform: @YES,
        (id)kCGImageSourceThumbnailMaxPixelSize: @(size)
    };
    CGImageRef image = CGImageSourceCreateThumbnailAtIndex(source, 0, (__bridge CFDictionaryRef)options);
    CFRelease(source);
    return image;
}
static BOOL save(CGImageRef image, NSString *path) {
    CGImageDestinationRef out = CGImageDestinationCreateWithURL((__bridge CFURLRef)[NSURL fileURLWithPath:path], CFSTR("public.png"), 1, NULL);
    if (!out) return NO;
    CGImageDestinationAddImage(out, image, NULL);
    BOOL ok = CGImageDestinationFinalize(out);
    CFRelease(out);
    return ok;
}
int main(int argc, const char **argv) {
    @autoreleasepool {
        if (argc < 4) { fprintf(stderr, "image-contacts previews OUTPUT_DIR RAW... | metadata OUTPUT.json RAW... | sheet OUTPUT.png [--columns N] IMAGE... | raster OUTPUT.ppm IMAGE\n"); return 2; }
        NSString *mode = @(argv[1]), *output = @(argv[2]);
        if ([mode isEqualToString:@"metadata"]) {
            NSMutableArray *records = [NSMutableArray array];
            for (int i = 3; i < argc; ++i) {
                NSString *input = @(argv[i]);
                CGImageSourceRef source = CGImageSourceCreateWithURL((__bridge CFURLRef)[NSURL fileURLWithPath:input], NULL);
                if (!source) return 1;
                NSDictionary *properties = CFBridgingRelease(CGImageSourceCopyPropertiesAtIndex(source, 0, NULL));
                CFRelease(source);
                NSDictionary *exif = properties[(id)kCGImagePropertyExifDictionary];
                NSMutableDictionary *record = [NSMutableDictionary dictionaryWithDictionary:@{@"file":input.lastPathComponent}];
                for (NSString *key in @[@"DateTimeOriginal", @"ExposureTime", @"FNumber", @"ISOSpeedRatings", @"LensModel"]) {
                    if (exif[key]) record[key] = exif[key];
                }
                [records addObject:record];
            }
            NSError *error = nil;
            NSData *json = [NSJSONSerialization dataWithJSONObject:records options:NSJSONWritingPrettyPrinted error:&error];
            if (!json || ![json writeToFile:output options:0 error:&error]) {
                fprintf(stderr, "metadata failed: %s\n", error.localizedDescription.UTF8String);
                return 1;
            }
            return 0;
        }
        if ([mode isEqualToString:@"previews"]) {
            [[NSFileManager defaultManager] createDirectoryAtPath:output withIntermediateDirectories:YES attributes:nil error:nil];
            for (int i = 3; i < argc; ++i) {
                NSString *input = @(argv[i]);
                NSString *name = [[[input lastPathComponent] stringByDeletingPathExtension] stringByAppendingPathExtension:@"png"];
                NSString *path = [output stringByAppendingPathComponent:name];
                if ([[NSFileManager defaultManager] fileExistsAtPath:path]) continue;
                CGImageRef image = thumbnail(input, 384);
                if (!image || !save(image, path)) { fprintf(stderr, "thumbnail failed: %s\n", argv[i]); return 1; }
                CGImageRelease(image);
            }
            return 0;
        }
        if ([mode isEqualToString:@"raster"]) {
            if (argc != 4) return 2;
            CGImageSourceRef source = CGImageSourceCreateWithURL((__bridge CFURLRef)[NSURL fileURLWithPath:@(argv[3])], NULL);
            if (!source) return 1;
            CGImageRef image = CGImageSourceCreateImageAtIndex(source, 0, NULL);
            CFRelease(source);
            if (!image) return 1;
            size_t width = CGImageGetWidth(image), height = CGImageGetHeight(image);
            CGColorSpaceRef color = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
            CGContextRef bitmap = CGBitmapContextCreate(NULL, width, height, 8, width * 4, color, kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big);
            CGColorSpaceRelease(color);
            if (!bitmap) { CGImageRelease(image); return 1; }
            CGContextDrawImage(bitmap, CGRectMake(0, 0, width, height), image);
            CGImageRelease(image);
            FILE *file = fopen(output.fileSystemRepresentation, "wb");
            if (!file) { CGContextRelease(bitmap); return 1; }
            fprintf(file, "P6\n%zu %zu\n255\n", width, height);
            const unsigned char *pixels = CGBitmapContextGetData(bitmap);
            BOOL ok = YES;
            for (size_t i = 0; i < width * height; ++i) {
                if (fwrite(pixels + i * 4, 1, 3, file) != 3) { ok = NO; break; }
            }
            if (fclose(file)) ok = NO;
            CGContextRelease(bitmap);
            return ok ? 0 : 1;
        }
        if (![mode isEqualToString:@"sheet"]) return 2;
        int columns = 6, firstImage = 3;
        if ([@(argv[3]) isEqualToString:@"--columns"]) {
            if (argc < 6) return 2;
            char *end = NULL;
            long value = strtol(argv[4], &end, 10);
            if (*end || value < 1 || value > 12) return 2;
            columns = (int)value;
            firstImage = 5;
        }
        const int cellWidth = 1440 / columns, cellHeight = cellWidth * 2 / 3 + 24;
        int rows = (argc - firstImage + columns - 1) / columns;
        CGColorSpaceRef color = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
        CGContextRef canvas = CGBitmapContextCreate(NULL, columns * cellWidth, rows * cellHeight, 8, 0, color, (CGBitmapInfo)kCGImageAlphaPremultipliedLast);
        CGColorSpaceRelease(color);
        CGContextSetRGBFillColor(canvas, 0.08, 0.08, 0.08, 1.);
        CGContextFillRect(canvas, CGRectMake(0, 0, columns * cellWidth, rows * cellHeight));
        CTFontRef font = CTFontCreateWithName(CFSTR("Menlo"), 12, NULL);
        CGColorRef white = CGColorCreateGenericRGB(1, 1, 1, 1);
        for (int i = firstImage; i < argc; ++i) {
            NSString *input = @(argv[i]);
            CGImageRef image = thumbnail(input, cellWidth - 8);
            if (!image) { fprintf(stderr, "image failed: %s\n", argv[i]); return 1; }
            CGFloat x = ((i - firstImage) % columns) * cellWidth, y = (rows - 1 - (i - firstImage) / columns) * cellHeight;
            CGFloat w = CGImageGetWidth(image), h = CGImageGetHeight(image);
            CGFloat scale = MIN((cellWidth - 8) / w, (cellHeight - 24) / h);
            CGContextDrawImage(canvas, CGRectMake(x + (cellWidth - w * scale) / 2, y + 22 + (cellHeight - 24 - h * scale) / 2, w * scale, h * scale), image);
            CGImageRelease(image);
            NSString *label = [[input lastPathComponent] stringByDeletingPathExtension];
            NSAttributedString *text = [[NSAttributedString alloc] initWithString:label attributes:@{(id)kCTFontAttributeName:(__bridge id)font, (id)kCTForegroundColorAttributeName:(__bridge id)white}];
            CTLineRef line = CTLineCreateWithAttributedString((__bridge CFAttributedStringRef)text);
            CGContextSetTextPosition(canvas, x + 6, y + 6);
            CTLineDraw(line, canvas);
            CFRelease(line);
        }
        CFRelease(font);
        CGColorRelease(white);
        CGImageRef image = CGBitmapContextCreateImage(canvas);
        BOOL ok = save(image, output);
        CGImageRelease(image);
        CGContextRelease(canvas);
        return ok ? 0 : 1;
    }
}
