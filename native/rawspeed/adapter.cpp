#include "adapter.h"
#include "RawSpeed-API.h"
#include "antitone_rawspeed_camera_db.h"
#include <pugixml.hpp>
#include <omp.h>
#include <algorithm>
#include <cstring>
#include <limits>
#include <memory>
#include <optional>
#include <stdexcept>

namespace {
struct Decoder {
    AntitoneRawSpeedRequest request{};
    std::optional<rawspeed::RawImage> image;
    int status = rawspeed_inited;
};

const rawspeed::CameraMetaData &cameras() {
    // Immutable after construction; separate native owners share only this database.
    static const auto metadata = [] {
        rawspeed::CameraMetaData result;
        pugi::xml_document doc;
        if (!doc.load_buffer(antitone_rawspeed_camera_db, sizeof(antitone_rawspeed_camera_db) - 1))
            throw std::runtime_error("Invalid embedded RawSpeed camera database");
        auto add = [&](std::unique_ptr<rawspeed::Camera> camera) -> const rawspeed::Camera * {
            rawspeed::CameraId id{rawspeed::trimSpaces(camera->make),
                                  rawspeed::trimSpaces(camera->model),
                                  rawspeed::trimSpaces(camera->mode)};
            auto [at, inserted] = result.cameras.emplace(std::move(id), std::move(camera));
            if (!inserted)
                return nullptr;
            auto *stored = at->second.get();
            if (stored->mode.find("chdk") != std::string::npos) {
                const auto size = stored->hints.get("filesize", std::string());
                if (!size.empty())
                    result.chdkCameras[std::stoul(size)] = stored;
            }
            return stored;
        };
        for (auto node : doc.child("Cameras").children("Camera")) {
            const auto *camera = add(std::make_unique<rawspeed::Camera>(node));
            if (camera)
                for (size_t alias = 0; alias < camera->aliases.size(); ++alias)
                    add(std::make_unique<rawspeed::Camera>(camera, alias));
        }
        if (result.cameras.empty())
            throw std::runtime_error("Empty embedded RawSpeed camera database");
        return result;
    }();
    return metadata;
}
}

extern "C" int rawspeed_get_number_of_processor_cores() {
    // The bridge scopes this calling task's ICV on the shared LLVM runtime.
    return omp_get_max_threads();
}

extern "C" const char *antitone_rawspeed_version() {
    return ANTITONE_RAWSPEED_REVISION ";adapter-v1";
}

extern "C" rawspeed3_handle_t rawspeed3_initdefault() {
    try {
        return new Decoder;
    } catch (...) {
        return nullptr;
    }
}

extern "C" void rawspeed3_clearresult(rawspeed3_ret_t *result) {
    *result = {};
    result->status = rawspeed_inited;
}

extern "C" void rawspeed3_release(rawspeed3_handle_t handle) {
    if (handle)
        static_cast<Decoder *>(handle)->image.reset();
}

extern "C" void rawspeed3_close(rawspeed3_handle_t handle) {
    delete static_cast<Decoder *>(handle);
}

extern "C" void antitone_rawspeed_prepare(rawspeed3_handle_t handle,
                                          const AntitoneRawSpeedRequest *request) {
    auto &decoder = *static_cast<Decoder *>(handle);
    decoder.image.reset();
    decoder.request = *request;
    decoder.status = rawspeed_inited;
}

extern "C" int antitone_rawspeed_status(rawspeed3_handle_t handle) {
    return handle ? static_cast<Decoder *>(handle)->status : rawspeed_inited;
}

extern "C" uint64_t antitone_rawspeed_retained_bytes(rawspeed3_handle_t handle) {
    if (!handle)
        return 0;
    const auto &decoder = *static_cast<Decoder *>(handle);
    uint64_t bytes = sizeof(decoder);
    if (decoder.image) {
        const auto &image = *decoder.image;
        // Explicit pixel backing, including row alignment. The shared camera database
        // and allocator bookkeeping are not per-source retained allocations.
        bytes +=
            sizeof(rawspeed::RawImageDataU16) + uint64_t(image->pitch) * image->getUncroppedDim().y;
        // Decoder workers have joined before retention is observed. Some codecs
        // retain potentially large bad-pixel lists even with interpolation disabled.
        bytes += image->mBadPixelPositions.capacity() * sizeof(uint32_t) +
                 image->mBadPixelMap.capacity();
        bytes += image->blackAreas.capacity() * sizeof(rawspeed::BlackArea);
        bytes += image->metadata.colorMatrix.capacity() * sizeof(rawspeed::NotARational<int>);
    }
    return bytes;
}

extern "C" int rawspeed3_decodefile(rawspeed3_handle_t handle, rawspeed3_ret_t *result,
                                    const void *data, size_t size, bool allow_unknown) {
    rawspeed3_clearresult(result);
    if (!handle)
        return result->status = rawspeed3_param_error;
    auto &owner = *static_cast<Decoder *>(handle);
    owner.image.reset();
    auto finish = [&](int status) { return owner.status = result->status = status; };
    if (!data || size == 0 || size > std::numeric_limits<uint32_t>::max())
        return finish(rawspeed3_param_error);
    const auto &expected = owner.request;
    if (!expected.width || !expected.height || expected.width > UINT16_MAX ||
        expected.height > UINT16_MAX ||
        (expected.cpp != 1 && expected.cpp != 3 && expected.cpp != 4))
        return finish(rawspeed3_not_supported);
    try {
        rawspeed::RawParser parser(rawspeed::Buffer(static_cast<const uint8_t *>(data), size));
        auto decoder = parser.getDecoder(&cameras());
        if (!decoder)
            return finish(rawspeed3_no_decoder);
        decoder->failOnUnknown = !allow_unknown;
        decoder->interpolateBadPixels = false;
        decoder->applyStage1DngOpcodes = false;
        decoder->applyCrop = false;
        decoder->fujiRotate = false;
        decoder->uncorrectedRawValues = true;
        // LibRaw applies its own PhaseOne corrections during development.
        decoder->iiq.quadrantMultipliers = false;
        try {
            decoder->checkSupport(&cameras());
        } catch (const rawspeed::RawspeedException &) {
            return finish(rawspeed3_not_supported);
        }
        decoder->decodeMetaData(&cameras());
        decoder->decodeRaw();
        auto image = decoder->mRaw;
        if (image->isTooManyErrors(1))
            return finish(rawspeed3_processing_error);
        const auto dimensions = image->getUncroppedDim();
        if (image->getDataType() != rawspeed::RawImageType::UINT16 ||
            image->getCpp() != expected.cpp || dimensions.x != int(expected.width) ||
            dimensions.y != int(expected.height) || (expected.filters && !image->isCFA) ||
            image->pitch < int(expected.width * expected.cpp * sizeof(uint16_t)) ||
            image->pitch % sizeof(uint16_t))
            return finish(rawspeed3_not_supported);
        auto pixels = image->getU16DataAsUncroppedArray2DRef();
        if (expected.curve) {
#pragma omp parallel for schedule(static)
            for (int row = 0; row < dimensions.y; ++row)
                for (uint32_t col = 0; col < expected.width * expected.cpp; ++col)
                    pixels(row, col) = expected.curve[pixels(row, col)];
        }
        // No future lookup is needed, and the pixel lease must not retain a decoder
        // whose file view will expire when LibRaw frees its temporary input buffer.
        image->setTable(nullptr);
        owner.image = image;
        result->width = expected.width;
        result->height = expected.height;
        result->bpp = expected.cpp * sizeof(uint16_t);
        result->cpp = expected.cpp;
        result->pitch = image->pitch;
        // LibRaw uses this field only to admit a single-plane raw_image, not to
        // replace its CFA. Monochrome data also belongs in that single-plane buffer.
        result->filters = expected.filters ? expected.filters : (expected.cpp == 1 ? 2 : 0);
        result->pixeldata = &pixels(0, 0);
        return finish(rawspeed3_ok);
    } catch (...) {
        // No result buffer has been published. LibRaw's original unpacker can run.
        return finish(rawspeed3_processing_error);
    }
}
