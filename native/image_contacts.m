// ImageIO source previews and labelled six-column contacts for optional Analyze inspection.
// Takes explicit read-only image paths; writes PNGs, skipping existing source previews.
// Invalid input or image I/O fails; generated contacts are not colour/photographic acceptance.
#import <Foundation/Foundation.h>
#import <ImageIO/ImageIO.h>
#import <CoreGraphics/CoreGraphics.h>
#import <CoreText/CoreText.h>

static CGImageRef thumbnail(NSString *path, int size) {
    CGImageSourceRef source =
        CGImageSourceCreateWithURL((__bridge CFURLRef)[NSURL fileURLWithPath:path], NULL);
    if (!source)
        return NULL;
    NSDictionary *options = @{
        (id)kCGImageSourceCreateThumbnailFromImageAlways : @YES,
        (id)kCGImageSourceCreateThumbnailWithTransform : @YES,
        (id)kCGImageSourceThumbnailMaxPixelSize : @(size)
    };
    CGImageRef image =
        CGImageSourceCreateThumbnailAtIndex(source, 0, (__bridge CFDictionaryRef)options);
    CFRelease(source);
    return image;
}

static BOOL save(CGImageRef image, NSString *path) {
    CGImageDestinationRef out = CGImageDestinationCreateWithURL(
        (__bridge CFURLRef)[NSURL fileURLWithPath:path], CFSTR("public.png"), 1, NULL);
    if (!out)
        return NO;
    CGImageDestinationAddImage(out, image, NULL);
    BOOL ok = CGImageDestinationFinalize(out);
    CFRelease(out);
    return ok;
}

int main(int argc, const char **argv) {
    @autoreleasepool {
        if (argc < 4) {
            fprintf(stderr,
                    "image-contacts previews OUTPUT_DIR RAW... | sheet OUTPUT.png IMAGE...\n");
            return 2;
        }
        NSString *mode = @(argv[1]), *output = @(argv[2]);
        if ([mode isEqualToString:@"previews"]) {
            [[NSFileManager defaultManager] createDirectoryAtPath:output
                                      withIntermediateDirectories:YES
                                                       attributes:nil
                                                            error:nil];
            for (int i = 3; i < argc; ++i) {
                NSString *input = @(argv[i]);
                NSString *name = [[[input lastPathComponent] stringByDeletingPathExtension]
                    stringByAppendingPathExtension:@"png"];
                NSString *path = [output stringByAppendingPathComponent:name];
                if ([[NSFileManager defaultManager] fileExistsAtPath:path])
                    continue;
                CGImageRef image = thumbnail(input, 384);
                if (!image || !save(image, path)) {
                    fprintf(stderr, "thumbnail failed: %s\n", argv[i]);
                    return 1;
                }
                CGImageRelease(image);
            }
            return 0;
        }
        if (![mode isEqualToString:@"sheet"])
            return 2;
        const int columns = 6, firstImage = 3;
        const int cellWidth = 1440 / columns, cellHeight = cellWidth * 2 / 3 + 24;
        int rows = (argc - firstImage + columns - 1) / columns;
        CGColorSpaceRef color = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
        CGContextRef canvas =
            CGBitmapContextCreate(NULL, columns * cellWidth, rows * cellHeight, 8, 0, color,
                                  (CGBitmapInfo)kCGImageAlphaPremultipliedLast);
        CGColorSpaceRelease(color);
        CGContextSetRGBFillColor(canvas, 0.08, 0.08, 0.08, 1.);
        CGContextFillRect(canvas, CGRectMake(0, 0, columns * cellWidth, rows * cellHeight));
        CTFontRef font = CTFontCreateWithName(CFSTR("Menlo"), 12, NULL);
        CGColorRef white = CGColorCreateGenericRGB(1, 1, 1, 1);
        for (int i = firstImage; i < argc; ++i) {
            NSString *input = @(argv[i]);
            CGImageRef image = thumbnail(input, cellWidth - 8);
            if (!image) {
                fprintf(stderr, "image failed: %s\n", argv[i]);
                return 1;
            }
            CGFloat x = ((i - firstImage) % columns) * cellWidth,
                    y = (rows - 1 - (i - firstImage) / columns) * cellHeight;
            CGFloat w = CGImageGetWidth(image), h = CGImageGetHeight(image);
            CGFloat scale = MIN((cellWidth - 8) / w, (cellHeight - 24) / h);
            CGContextDrawImage(canvas,
                               CGRectMake(x + (cellWidth - w * scale) / 2,
                                          y + 22 + (cellHeight - 24 - h * scale) / 2, w * scale,
                                          h * scale),
                               image);
            CGImageRelease(image);
            NSString *label = [[input lastPathComponent] stringByDeletingPathExtension];
            NSAttributedString *text = [[NSAttributedString alloc]
                initWithString:label
                    attributes:@{
                        (id)kCTFontAttributeName : (__bridge id)font,
                        (id)kCTForegroundColorAttributeName : (__bridge id)white
                    }];
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
