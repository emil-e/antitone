// Header-only metadata inventory; never unpack, develop, hash or modify the input RAW.
#include <libraw/libraw.h>
#include <array>
#include <chrono>
#include <cstring>
#include <iomanip>
#include <iostream>
#include <string>
#include <utility>
#include <vector>

using Tags = std::vector<std::pair<int, std::string>>;

static void exif(void *context, int tag, int type, int len, unsigned int,
                 void *input, INT64) {
    if (type != 2 || len < 1 || len > 128) return;
    switch (tag & 0xffff) {
    case 0x0132: // DateTime
    case 0x9003: // DateTimeOriginal
    case 0x9004: // DateTimeDigitized
    case 0x9010: // OffsetTime
    case 0x9011: // OffsetTimeOriginal
    case 0x9012: // OffsetTimeDigitized
    case 0x9290: // SubSecTime
    case 0x9291: // SubSecTimeOriginal
    case 0x9292: // SubSecTimeDigitized
    case 0xa420: // ImageUniqueID
    case 0xa431: // BodySerialNumber
        break;
    default:
        return;
    }
    auto *stream = static_cast<LibRaw_abstract_datastream *>(input);
    const INT64 position = stream->tell();
    std::array<char, 129> value{};
    const int count = stream->read(value.data(), 1, static_cast<size_t>(len));
    stream->seek(position, SEEK_SET);
    if (count == len) {
        static_cast<Tags *>(context)->emplace_back(
            tag, std::string(value.data(), strnlen(value.data(), static_cast<size_t>(len))));
    }
}

static std::string hex(const char *data, size_t size) {
    const char *digits = "0123456789abcdef";
    std::string result;
    for (size_t i = 0; i < size; ++i) {
        const auto byte = static_cast<unsigned char>(data[i]);
        result += digits[byte >> 4];
        result += digits[byte & 15];
    }
    return result;
}

int main(int argc, char **argv) {
    if (argc < 2) {
        std::cerr << "usage: raw-identity-probe INPUT.RAW [...]\n";
        return 2;
    }
    for (int i = 1; i < argc; ++i) {
        LibRaw raw;
        Tags tags;
        raw.set_exifparser_handler(exif, &tags);
        const auto started = std::chrono::steady_clock::now();
        const int status = raw.open_file(argv[i]);
        if (status != LIBRAW_SUCCESS) {
            std::cerr << argv[i] << ": " << libraw_strerror(status) << '\n';
            return 1;
        }
        const auto &data = raw.imgdata;
        const auto &sony = data.makernotes.sony;
        std::cout << "{\"path\":" << std::quoted(argv[i])
                  << ",\"make\":" << std::quoted(data.idata.make)
                  << ",\"model\":" << std::quoted(data.idata.model)
                  << ",\"image_unique_id\":" << std::quoted(data.color.ImageUniqueID)
                  << ",\"raw_data_unique_id_hex\":" << std::quoted(hex(data.color.RawDataUniqueID, 16))
                  << ",\"body_serial\":" << std::quoted(data.shootinginfo.BodySerial)
                  << ",\"internal_body_serial\":" << std::quoted(data.shootinginfo.InternalBodySerial)
                  << ",\"timestamp\":" << data.other.timestamp
                  << ",\"shot_order\":" << data.other.shot_order
                  << ",\"sony_image_count3\":" << sony.ImageCount3
                  << ",\"sony_since_power_up\":" << sony.ShotNumberSincePowerUp
                  << ",\"sony_sequence_image\":" << sony.Sony0x9400_SequenceImageNumber
                  << ",\"sony_datetime\":" << std::quoted(sony.SonyDateTime)
                  << ",\"open_ms\":"
                  << std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - started).count()
                  << ",\"exif\":[";
        bool comma = false;
        for (const auto &entry : tags) {
            if (comma) std::cout << ',';
            comma = true;
            std::cout << "{\"tag\":" << entry.first << ",\"value\":" << std::quoted(entry.second) << '}';
        }
        std::cout << "]}\n";
    }
}
