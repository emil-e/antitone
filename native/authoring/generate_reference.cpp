// Offline OCIO shader/resource generation; never linked into the application.
#include <OpenColorIO/OpenColorIO.h>
#include <array>
#include <cmath>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <stdexcept>
#include <string>
#include <vector>
namespace OCIO = OCIO_NAMESPACE;
namespace fs = std::filesystem;

int main(int argc, char **argv) {
    try {
        if (argc != 11 || std::string(OCIO::GetVersion()) != "2.6.0")
            throw std::runtime_error(
                "usage: generate-reference FRESH_OUTPUT_DIR NINE_PARAMETERS; requires OCIO 2.6.0");
        std::array<double, 9> parameters{};
        for (int i = 0; i < 9; ++i) {
            std::string text = argv[i + 2];
            size_t used = 0;
            parameters[i] = std::stod(text, &used);
            if (used != text.size() || !std::isfinite(parameters[i]))
                throw std::runtime_error("invalid reference parameter");
        }
        fs::path output = argv[1];
        if (!fs::create_directory(output))
            throw std::runtime_error("choose a fresh output directory");
        std::array<std::vector<float>, 2> tables;
        for (bool inverse : {false, true}) {
            auto group = OCIO::GroupTransform::Create();
            auto ap0 = OCIO::BuiltinTransform::Create();
            ap0->setStyle("ACEScg_to_ACES2065-1");
            auto drt =
                OCIO::FixedFunctionTransform::Create(OCIO::FIXED_FUNCTION_ACES_OUTPUT_TRANSFORM_20,
                                                     parameters.data(), parameters.size());
            if (inverse) {
                drt->setDirection(OCIO::TRANSFORM_DIR_INVERSE);
                group->appendTransform(drt);
                ap0->setDirection(OCIO::TRANSFORM_DIR_INVERSE);
                group->appendTransform(ap0);
            } else {
                group->appendTransform(ap0);
                group->appendTransform(drt);
            }
            auto processor = OCIO::Config::CreateRaw()->getProcessor(group);
            auto shader = OCIO::GpuShaderDesc::CreateShaderDesc();
            shader->setLanguage(OCIO::GPU_LANGUAGE_GLSL_VK_4_6);
            shader->setAllowTexture1D(false);
            shader->setFunctionName("look_reference");
            shader->setResourcePrefix("aces_");
            processor->getDefaultGPUProcessor()->extractGpuShaderInfo(shader);
            if (shader->getNumTextures() != 2 || shader->getNum3DTextures() ||
                shader->getNumUniforms())
                throw std::runtime_error("unexpected fixed-reference resources");
            std::ofstream code(output / (inverse ? "inverse.glsl" : "forward.glsl"));
            code << shader->getShaderText();
            code.close();
            if (!code)
                throw std::runtime_error("cannot write reference shader");
            for (unsigned i = 0; i < 2; ++i) {
                const char *texture, *sampler;
                unsigned width, height;
                OCIO::GpuShaderDesc::TextureType channels;
                OCIO::GpuShaderDesc::TextureDimensions dimensions;
                OCIO::Interpolation interpolation;
                shader->getTexture(i, texture, sampler, width, height, channels, dimensions,
                                   interpolation);
                const unsigned count = i ? 3 : 1;
                if (width != 363 || height != 1 ||
                    channels != (i ? OCIO::GpuShaderDesc::TEXTURE_RGB_CHANNEL
                                   : OCIO::GpuShaderDesc::TEXTURE_RED_CHANNEL) ||
                    shader->getTextureShaderBindingIndex(i) != i + 1)
                    throw std::runtime_error("unexpected fixed-reference table layout");
                const float *values;
                shader->getTextureValues(i, values);
                std::vector<float> data(values, values + width * count);
                for (float value : data)
                    if (!std::isfinite(value))
                        throw std::runtime_error("nonfinite reference table");
                if (!inverse)
                    tables[i] = data;
                else if (tables[i] != data)
                    throw std::runtime_error(
                        "forward/inverse table values differ; do not deduplicate");
            }
        }
        for (unsigned i = 0; i < 2; ++i) {
            std::ofstream file(output / (i ? "cusp.f32" : "reach.f32"), std::ios::binary);
            for (float value : tables[i]) {
                // The supported generation host is little-endian Apple Silicon.
                file.write(reinterpret_cast<const char *>(&value), 4);
            }
            file.close();
            if (!file)
                throw std::runtime_error("cannot write reference table");
        }
        return 0;
    } catch (const std::exception &error) {
        std::cerr << error.what() << '\n';
        return 1;
    }
}
