#import <Foundation/Foundation.h>
#import <ImageIO/ImageIO.h>

int main(int argc, const char *argv[]) {
    @autoreleasepool {
        if (argc != 2) {
            fprintf(stderr, "Usage: contrast-saturation-to-png TIFF_DIRECTORY\n");
            return 1;
        }
        NSString *directory = [NSString stringWithUTF8String:argv[1]];
        NSError *error = nil;
        NSArray<NSString *> *files = [[NSFileManager defaultManager]
            contentsOfDirectoryAtPath:directory error:&error];
        if (!files) {
            fprintf(stderr, "%s\n", error.localizedDescription.UTF8String);
            return 1;
        }
        NSUInteger count = 0;
        for (NSString *name in files) {
            if (![name.pathExtension isEqualToString:@"tif"]) continue;
            @autoreleasepool {
                NSString *input = [directory stringByAppendingPathComponent:name];
                NSString *output = [input.stringByDeletingPathExtension stringByAppendingPathExtension:@"png"];
                if ([[NSFileManager defaultManager] fileExistsAtPath:output]) {
                    fprintf(stderr, "Refusing existing output: %s\n", output.UTF8String);
                    return 1;
                }
                CGImageSourceRef source = CGImageSourceCreateWithURL(
                    (__bridge CFURLRef)[NSURL fileURLWithPath:input], NULL);
                CGImageDestinationRef destination = CGImageDestinationCreateWithURL(
                    (__bridge CFURLRef)[NSURL fileURLWithPath:output], CFSTR("public.png"), 1, NULL);
                if (!source || !destination) {
                    fprintf(stderr, "Cannot convert: %s\n", input.UTF8String);
                    if (source) CFRelease(source);
                    if (destination) CFRelease(destination);
                    return 1;
                }
                // No resizing or requested destination-space conversion. ImageIO
                // can substitute named profiles; verify-png.py checks every code
                // and reinstates the exact TIFF ICC. TIFFs remain the reference.
                CGImageDestinationAddImageFromSource(destination, source, 0, NULL);
                bool success = CGImageDestinationFinalize(destination);
                CFRelease(destination);
                CFRelease(source);
                if (!success) {
                    fprintf(stderr, "PNG write failed: %s\n", output.UTF8String);
                    return 1;
                }
                count++;
            }
        }
        printf("Converted %lu profiled TIFFs to PNG.\n", (unsigned long)count);
    }
    return 0;
}
