// Header/embedded-preview diagnostic only; never unpacks or develops sensor data.
#include <libraw/libraw.h>
#include <chrono>
#include <cstdio>

using Clock = std::chrono::steady_clock;
static double milliseconds(Clock::time_point start) {
    return std::chrono::duration<double, std::milli>(Clock::now() - start).count();
}
int main(int argc, char **argv) {
    if (argc < 2) {
        std::fprintf(stderr, "usage: raw-thumbnail-probe RAW...\n");
        return 2;
    }
    bool failed = false;
    for (int file = 1; file < argc; ++file) {
        LibRaw raw;
        auto start = Clock::now();
        int status = raw.open_file(argv[file]);
        double open_ms = milliseconds(start);
        if (status != LIBRAW_SUCCESS) {
            std::fprintf(stderr, "%s: %s\n", argv[file], libraw_strerror(status));
            failed = true;
            continue;
        }
        const auto &list = raw.imgdata.thumbs_list;
        std::printf("embedded_header file=%s open_ms=%.6f count=%d\n", argv[file], open_ms, list.thumbcount);
        for (int index = 0; index < list.thumbcount; ++index) {
            const auto item = list.thumblist[index];
            start = Clock::now();
            status = raw.unpack_thumb_ex(index);
            double extract_ms = milliseconds(start);
            const auto &thumb = raw.imgdata.thumbnail;
            bool jpeg_soi = status == LIBRAW_SUCCESS && thumb.tformat == LIBRAW_THUMBNAIL_JPEG && thumb.tlength >= 2 &&
                static_cast<unsigned char>(thumb.thumb[0]) == 0xff && static_cast<unsigned char>(thumb.thumb[1]) == 0xd8;
            std::printf("embedded_preview file=%s index=%d descriptor_format=%d dimensions=%ux%u flip=%u descriptor_bytes=%u status=%d format=%d dimensions_out=%ux%u bytes=%u jpeg_soi=%d extract_ms=%.6f\n",
                argv[file], index, static_cast<int>(item.tformat), item.twidth, item.theight, item.tflip, item.tlength,
                status, status == LIBRAW_SUCCESS ? static_cast<int>(thumb.tformat) : -1,
                status == LIBRAW_SUCCESS ? thumb.twidth : 0, status == LIBRAW_SUCCESS ? thumb.theight : 0,
                status == LIBRAW_SUCCESS ? thumb.tlength : 0, jpeg_soi, extract_ms);
            failed |= status != LIBRAW_SUCCESS;
        }
    }
    return failed ? 1 : 0;
}
