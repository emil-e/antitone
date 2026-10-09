#include "rendering_transform.h"
#include <algorithm>
#include <array>
#include <cmath>
#include <cstdio>
#include <string>

namespace OCIO = OCIO_NAMESPACE;

int main(int argc, char **argv) try {
    const bool compare = argc == 2 && std::string(argv[1]) == "--compare-rec2020";
    if (argc != 1 && !compare) throw std::invalid_argument("usage: gamut-probe [--compare-rec2020]");
    struct Target {
        const char *name;
        std::array<double, 9> parameters;
    };
    const Target targets[] {
        {"sRGB-100nit", antitone::output_parameters(0)},
        {"Display-P3-100nit", antitone::output_parameters(1)},
        {"Rec2020-D65-100nit", {100., .708, .292, .170, .797, .131, .046, .3127, .3290}},
        {"Rec2020-D65-500nit", {500., .708, .292, .170, .797, .131, .046, .3127, .3290}},
    };
    const float levels[] {0.f, .01f, .05f, .1f, .18f, .2f, .4f, .5f, .75f, 1.f};
    unsigned failures = 0;
    std::printf("OCIO %s / direct ACES2 fixed-function / AP1 input / no inversion or presentation\n", OCIO::GetVersion());
    for (unsigned index = 0; index < (compare ? 4u : 2u); ++index) {
        const auto &target = targets[index];
        auto group = OCIO::GroupTransform::Create();
        auto basis = OCIO::BuiltinTransform::Create();
        basis->setStyle("ACEScg_to_ACES2065-1");
        group->appendTransform(basis);
        group->appendTransform(OCIO::FixedFunctionTransform::Create(
            OCIO::FIXED_FUNCTION_ACES_OUTPUT_TRANSFORM_20,
            target.parameters.data(), target.parameters.size()));
        auto cpu = OCIO::Config::CreateRaw()->getProcessor(group)->getDefaultCPUProcessor();
        unsigned checked = 0, collapsed = 0;
        for (float r : levels) for (float g : levels) for (float b : levels) {
            const std::array<float, 3> input {r, g, b};
            auto output = input;
            cpu->applyRGB(output.data());
            for (float value : output) {
                if (!std::isfinite(value)) {
                    std::fprintf(stderr, "target=%s nonfinite output\n", target.name);
                    return 2;
                }
            }
            const auto source = std::minmax_element(input.begin(), input.end());
            if (*source.second - *source.first < .009f) continue;
            ++checked;
            const auto rendered = std::minmax_element(output.begin(), output.end());
            // These nonneutral, moderate-exposure samples should not become gray.
            // This is not a requirement that different targets have matching RGB.
            if (*rendered.second - *rendered.first < 1e-6f) {
                ++collapsed;
                std::printf("target=%s collapsed AP1=(%.9g %.9g %.9g) native_linear=(%.9g %.9g %.9g)\n",
                    target.name, r, g, b, output[0], output[1], output[2]);
            }
        }
        failures += collapsed;
        std::printf("target=%s colored_checked=%u collapsed=%u\n", target.name, checked, collapsed);
    }
    return failures ? 1 : 0;
} catch (const std::exception &error) {
    std::fprintf(stderr, "Gamut probe: %s\n", error.what());
    return 2;
}
