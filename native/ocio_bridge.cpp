#include "ocio_bridge.h"
#include "rendering_transform.h"
#include <algorithm>
#include <climits>
#include <cstdio>
#include <memory>
#include <string>

namespace {
struct Renderer {
    antitone::OCIO::ConstCPUProcessorRcPtr cpu;
};

void diagnostic(char *error, size_t size, const char *message) {
    if (size)
        std::snprintf(error, size, "%s", message);
}
}

const char *antitone_ocio_version() {
    return antitone::OCIO::GetVersion();
}

int antitone_output_parameters(uint32_t target, double *parameters) {
    try {
        const auto &values = antitone::output_parameters(target);
        std::copy(values.begin(), values.end(), parameters);
        return 0;
    } catch (const std::exception &) {
        return -1;
    }
}

void *antitone_ocio_create(uint32_t target, char *error, size_t error_size) {
    try {
        if (std::string(antitone::OCIO::GetVersion()) != "2.6.0")
            throw std::runtime_error("OCIO runtime version must be 2.6.0");
        auto renderer = std::make_unique<Renderer>();
        renderer->cpu = antitone::rendering(target)->getDefaultCPUProcessor();
        return renderer.release();
    } catch (const std::exception &e) {
        diagnostic(error, error_size, e.what());
        return nullptr;
    }
}

int antitone_ocio_apply(void *handle, float *rgb, size_t count, char *error, size_t error_size) {
    try {
        if (count > LONG_MAX)
            throw std::invalid_argument("pixel count exceeds OCIO image dimensions");
        if (count) {
            antitone::OCIO::PackedImageDesc image(rgb, static_cast<long>(count), 1, 3);
            static_cast<Renderer *>(handle)->cpu->apply(image);
        }
        return 0;
    } catch (const std::exception &e) {
        diagnostic(error, error_size, e.what());
        return -1;
    }
}

void antitone_ocio_destroy(void *handle) {
    delete static_cast<Renderer *>(handle);
}
