#import <Foundation/Foundation.h>
#import <CoreGraphics/CoreGraphics.h>
int main(int argc, char **argv) {
    if (argc != 2) return 1;
    int pid = atoi(argv[1]);
    @autoreleasepool {
        NSArray *windows = CFBridgingRelease(CGWindowListCopyWindowInfo(kCGWindowListOptionOnScreenOnly, kCGNullWindowID));
        for (NSDictionary *window in windows) {
            if ([window[(id)kCGWindowOwnerPID] intValue] == pid && [window[(id)kCGWindowLayer] intValue] == 0) {
                printf("%u\n", [window[(id)kCGWindowNumber] unsignedIntValue]);
                return 0;
            }
        }
    }
    return 2;
}
