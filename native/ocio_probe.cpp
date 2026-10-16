// Generates upstream OCIO AP1/AP0 equivalence references, GLSL and LUTs for both SDR targets.
// Requires pinned OCIO and an existing .cache/probes; --camera requests authored AP1 inputs.
// Gates finite outputs/LUTs, neutral spread and basis agreement; prints resource/error statistics.
// Writes regenerable reference artifacts, not photographic or physical-display acceptance.
#include "rendering_transform.h"
#include <algorithm>
#include <array>
#include <cmath>
#include <cstdio>
#include <cstring>
#include <fstream>
#include <vector>

namespace OCIO = OCIO_NAMESPACE;

int main(int argc, char **argv) try {
    const bool withCamera = argc == 2 && std::strcmp(argv[1], "--camera") == 0;
    if (argc != 1 && !withCamera) {
        std::fprintf(stderr, "Usage: ocio-probe [--camera]\n");
        return 1;
    }
    std::printf("OCIO %s / input_space=ACEScg (linear AP1, D60)\n", OCIO::GetVersion());
    auto toAP0 = OCIO::BuiltinTransform::Create();
    toAP0->setStyle("ACEScg_to_ACES2065-1");
    auto ap1ToAP0 = OCIO::Config::CreateRaw()->getProcessor(toAP0)->getDefaultCPUProcessor();
    std::vector<std::array<float, 4>> inputs;
    for (float value : {0.f, .001f, .01f, .18f, 1.f, 10.f, 1000.f})
        inputs.push_back({value, value, value, 1.f});
    for (int r = -1; r <= 10; ++r)
        for (int g = -1; g <= 10; ++g)
            for (int b = -1; b <= 10; ++b)
                inputs.push_back({r * .2f, g * .2f, b * .2f, 1.f});
    for (float value : {10.f, 100.f, 1000.f}) {
        inputs.push_back({value, 0.f, 0.f, 1.f});
        inputs.push_back({0.f, value, 0.f, 1.f});
        inputs.push_back({0.f, 0.f, value, 1.f});
    }
    size_t cameraCount = 0;
    if (withCamera) {
        std::ifstream camera(".cache/probes/camera-working-ap1-model4.f32",
                             std::ios::binary | std::ios::ate);
        if (!camera) {
            std::fprintf(stderr, "Generate camera references before requesting --camera\n");
            return 7;
        }
        const auto bytes = camera.tellg();
        if (bytes <= 0 || bytes % sizeof(inputs[0]) != 0)
            return 7;
        cameraCount = bytes / sizeof(inputs[0]);
        const auto offset = inputs.size();
        inputs.resize(offset + cameraCount);
        camera.seekg(0);
        if (!camera.read(reinterpret_cast<char *>(inputs.data() + offset), bytes))
            return 7;
    }
    std::printf("camera_pixels=%zu (explicit --camera extension)\n", cameraCount);
    std::ofstream inputFile(".cache/probes/aces-inputs-ap1.f32", std::ios::binary);
    inputFile.write(reinterpret_cast<const char *>(inputs.data()),
                    inputs.size() * sizeof(inputs[0]));
    inputFile.close();
    for (int target = 0; target < 2; ++target) {
        auto processor = antitone::rendering(target);
        auto cpu = processor->getDefaultCPUProcessor();
        std::printf("TARGET %s\n", target ? "Display-P3-D65" : "sRGB-D65");
        float neutralError = 0.f;
        for (float value : {0.f, .001f, .01f, .18f, 1.f, 10.f, 1000.f}) {
            float pixel[4] = {value, value, value, 1.f};
            cpu->applyRGBA(pixel);
            for (int c = 0; c < 3; ++c)
                if (!std::isfinite(pixel[c]))
                    return 2;
            neutralError = std::max(neutralError, std::max(std::abs(pixel[0] - pixel[1]),
                                                           std::abs(pixel[0] - pixel[2])));
            std::printf("neutral %.6g -> %.8g %.8g %.8g\n", value, pixel[0], pixel[1], pixel[2]);
        }
        auto outputs = inputs;
        auto previousAP0 = antitone::rendering(target, true)->getDefaultCPUProcessor();
        float basisError = 0.f;
        for (size_t i = 0; i < outputs.size(); ++i) {
            auto &pixel = outputs[i];
            auto reference = inputs[i];
            ap1ToAP0->applyRGBA(reference.data());
            previousAP0->applyRGBA(reference.data());
            cpu->applyRGBA(pixel.data());
            for (int c = 0; c < 3; ++c) {
                if (!std::isfinite(pixel[c]) || !std::isfinite(reference[c]))
                    return 3;
                const float error = std::abs(pixel[c] - reference[c]);
                basisError = std::max(basisError, error);
                if (error > 2e-5f + 2e-4f * std::abs(reference[c]))
                    return 6;
            }
        }
        std::printf("AP1_entry_vs_converted_AP0_reference_max_error=%.8g\n", basisError);
        std::ofstream referenceFile(std::string(".cache/probes/") +
                                        (target ? "display-p3" : "srgb") + "-ap1-reference.f32",
                                    std::ios::binary);
        referenceFile.write(reinterpret_cast<const char *>(outputs.data()),
                            outputs.size() * sizeof(outputs[0]));
        std::printf("conformance_pixels=%zu\n", outputs.size());
        auto shader = OCIO::GpuShaderDesc::CreateShaderDesc();
        shader->setLanguage(OCIO::GPU_LANGUAGE_GLSL_VK_4_6);
        shader->setAllowTexture1D(false);
        shader->setFunctionName("antitone_render");
        shader->setResourcePrefix("aces_");
        processor->getDefaultGPUProcessor()->extractGpuShaderInfo(shader);
        const char *name = target ? "display-p3" : "srgb";
        std::ofstream code(std::string(".cache/probes/") + name + "-ap1.glsl");
        code << shader->getShaderText();
        std::printf("neutral_max_error=%.8g textures=%u textures3d=%u uniforms=%u\n", neutralError,
                    shader->getNumTextures(), shader->getNum3DTextures(), shader->getNumUniforms());
        for (unsigned i = 0; i < shader->getNumTextures(); ++i) {
            const char *texture, *sampler;
            unsigned width, height;
            OCIO::GpuShaderDesc::TextureType channels;
            OCIO::GpuShaderDesc::TextureDimensions dimensions;
            OCIO::Interpolation interpolation;
            shader->getTexture(i, texture, sampler, width, height, channels, dimensions,
                               interpolation);
            const float *values;
            shader->getTextureValues(i, values);
            const size_t count =
                width * height * (channels == OCIO::GpuShaderDesc::TEXTURE_RGB_CHANNEL ? 3 : 1);
            for (size_t v = 0; v < count; ++v)
                if (!std::isfinite(values[v]))
                    return 4;
            std::printf(
                "texture=%s sampler=%s width=%u height=%u channels=%d binding=%u interpolation=%d\n",
                texture, sampler, width, height, static_cast<int>(channels),
                shader->getTextureShaderBindingIndex(i), static_cast<int>(interpolation));
            std::ofstream data(std::string(".cache/probes/") + name + "-ap1-lut-" +
                                   std::to_string(i) + ".f32",
                               std::ios::binary);
            data.write(reinterpret_cast<const char *>(values), count * sizeof(float));
        }
        if (neutralError > 2e-5f)
            return 5;
    }
    return 0;
} catch (const std::exception &e) {

    std::fprintf(stderr, "OCIO probe: %s\n", e.what());
    return 1;
}
