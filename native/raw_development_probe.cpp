// Read-only full RGB/metadata/immutable-diagnostics comparison through the production bridge.
#include <raw_bridge.h>
#include <CommonCrypto/CommonDigest.h>
#include <array>
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <memory>
#include <vector>
#ifndef ANTITONE_BASELINE_BRIDGE
#include <omp.h>
#endif

using Clock = std::chrono::steady_clock;
static double seconds(Clock::time_point start) {
    return std::chrono::duration<double>(Clock::now() - start).count();
}
static void print_digest(const char *name, const void *data, size_t bytes) {
    std::array<unsigned char, CC_SHA256_DIGEST_LENGTH> digest;
    CC_SHA256(data, static_cast<CC_LONG>(bytes), digest.data());
    std::printf(" %s=", name);
    for (auto byte : digest) std::printf("%02x", byte);
}
int main(int argc, char **argv) {
    if (argc < 3 || argc > 5) {
        std::fprintf(stderr, "usage: raw-development-probe SOURCE.ARW WB-REFERENCE.ARW [THREADS] [--decode-only]\n");
        return 2;
    }
    const int threads = argc >= 4 ? std::atoi(argv[3]) : 4;
    const bool decode_only = argc == 5 && !std::strcmp(argv[4], "--decode-only");
    char error[512] = {};
    AntitoneRawMetadata reference, metadata;
    std::memset(&reference, 0, sizeof(reference));
    std::memset(&metadata, 0, sizeof(metadata));
    using Raw = std::unique_ptr<void, decltype(&antitone_raw_close)>;
    Raw wb(antitone_raw_open(argv[2], &reference, error, sizeof(error)), antitone_raw_close);
    if (!wb) { std::fprintf(stderr, "%s\n", error); return 1; }
    auto start = Clock::now();
    Raw raw(antitone_raw_open(argv[1], &metadata, error, sizeof(error)), antitone_raw_close);
    if (!raw) { std::fprintf(stderr, "%s\n", error); return 1; }
    double open = seconds(start);
    start = Clock::now();
    AntitoneRawDiagnostics full = {}, region = {};
#ifdef ANTITONE_BASELINE_BRIDGE
    (void)threads;
    (void)decode_only;
    int code = antitone_raw_unpack(raw.get(), &full, error, sizeof(error));
#else
    const int previous_threads = omp_get_max_threads(), previous_dynamic = omp_get_dynamic(), previous_blocktime = kmp_get_blocktime();
    int code = antitone_raw_unpack(raw.get(), threads, error, sizeof(error));
    if (!code && !decode_only) code = antitone_raw_sensor_diagnostics(raw.get(), &full, error, sizeof(error));
#endif
    if (code) { std::fprintf(stderr, "%s\n", error); return 1; }
    double unpack = seconds(start);
    if (antitone_raw_region_diagnostics(raw.get(), 4000, 2600, 128, 128, &region, error, sizeof(error))) {
        std::fprintf(stderr, "%s\n", error); return 1;
    }
    start = Clock::now();
    AntitoneRawView view = {};
#ifdef ANTITONE_BASELINE_BRIDGE
    code = antitone_raw_develop(raw.get(), reference.cam_mul, 0, &view, error, sizeof(error));
#else
    code = antitone_raw_develop(raw.get(), reference.cam_mul, 0, threads, &view, error, sizeof(error));
    if (omp_get_max_threads() != previous_threads || omp_get_dynamic() != previous_dynamic || kmp_get_blocktime() != previous_blocktime) {
        std::fprintf(stderr, "OpenMP task policy was not restored\n"); return 1;
    }
#endif
    if (code) { std::fprintf(stderr, "%s\n", error); return 1; }
    double process = seconds(start);
    AntitoneRawDiagnostics after = {};
    if (antitone_raw_region_diagnostics(raw.get(), 4000, 2600, 128, 128, &after, error, sizeof(error)) || std::memcmp(&region, &after, sizeof(region))) {
        std::fprintf(stderr, "Immutable regional diagnostics changed after development: %s\n", error); return 1;
    }
#ifndef ANTITONE_BASELINE_BRIDGE
    after = {};
    if (antitone_raw_sensor_diagnostics(raw.get(), &after, error, sizeof(error)) || (!decode_only && std::memcmp(&full, &after, sizeof(full)))) {
        std::fprintf(stderr, "Immutable whole-frame diagnostics changed: %s\n", error); return 1;
    }
    full = after;
#endif
    // Every RGB code is hashed; the explicitly ignored fourth slot is excluded.
    CC_SHA256_CTX hash;
    CC_SHA256_Init(&hash);
    std::vector<uint16_t> row(view.width * 3);
    for (unsigned y = 0; y < view.height; ++y) {
        for (unsigned x = 0; x < view.width; ++x)
            for (unsigned c = 0; c < 3; ++c) row[x * 3 + c] = view.pixels[(y * view.width + x) * 4 + c];
        CC_SHA256_Update(&hash, row.data(), static_cast<CC_LONG>(row.size() * sizeof(uint16_t)));
    }
    std::array<unsigned char, CC_SHA256_DIGEST_LENGTH> digest;
    CC_SHA256_Final(digest.data(), &hash);
    std::printf("raw_native_probe decode_only=%d threads=%d version=%s size=%ux%u maximum=%u data_maximum=%u warnings=%u open_seconds=%.6f unpack_seconds=%.6f process_seconds=%.6f develop_seconds=%.6f rgb_sha256=", decode_only, threads, antitone_raw_version(), view.width, view.height, view.maximum, view.data_maximum, view.warnings, open, unpack, process, unpack + process);
    for (auto byte : digest) std::printf("%02x", byte);
    print_digest("metadata_sha256", &metadata, sizeof(metadata));
    print_digest("premul_sha256", view.pre_mul, sizeof(view.pre_mul));
    print_digest("full_sensor_sha256", &full, sizeof(full));
    print_digest("region_sensor_sha256", &region, sizeof(region));
    std::printf("\n");
}
