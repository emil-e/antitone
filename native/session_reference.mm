#import <CoreGraphics/CoreGraphics.h>
#include <cstdio>
#include <initializer_list>

int main() {
    CFDictionaryRef session = CGSessionCopyCurrentDictionary();
    if (!session) { std::puts("GUI_session=unavailable"); return 1; }
    for (CFStringRef key : { kCGSessionOnConsoleKey, kCGSessionLoginDoneKey, CFSTR("CGSSessionScreenIsLocked") }) {
        CFTypeRef value = CFDictionaryGetValue(session, key);
        char name[128];
        CFStringGetCString(key, name, sizeof(name), kCFStringEncodingUTF8);
        const char *state = !value ? "unknown" : (CFGetTypeID(value) == CFBooleanGetTypeID() ? (CFBooleanGetValue((CFBooleanRef)value) ? "true" : "false") : "nonboolean");
        std::printf("%s=%s\n", name, state);
    }
    CFRelease(session);
}
