// Offline coordinate encoding through Antitone's pinned authoring reference.
// OpenColorIO is a build-time generator dependency, not an application dependency.
#include <OpenColorIO/OpenColorIO.h>
#include <array>
#include <cmath>
#include <fstream>
#include <iostream>
#include <stdexcept>

namespace OCIO = OCIO_NAMESPACE;

int main(int argc, char **argv) {
    try {
        if (std::string(OCIO::GetVersion()) != "2.6.0")
            throw std::runtime_error("use the pinned OCIO 2.6.0 authoring reference");
        if (argc != 13)
            throw std::runtime_error(
                "expected inverse|forward input.f32 output.f32 and nine reference parameters");
        std::array<double, 9> parameters{};
        for (int i = 0; i < 9; ++i)
            parameters[i] = std::stod(argv[4 + i]);
        auto group = OCIO::GroupTransform::Create();
        auto ap0 = OCIO::BuiltinTransform::Create();
        ap0->setStyle("ACEScg_to_ACES2065-1");
        auto drt = OCIO::FixedFunctionTransform::Create(
            OCIO::FIXED_FUNCTION_ACES_OUTPUT_TRANSFORM_20, parameters.data(), parameters.size());
        if (std::string(argv[1]) == "inverse") {
            drt->setDirection(OCIO::TRANSFORM_DIR_INVERSE);
            ap0->setDirection(OCIO::TRANSFORM_DIR_INVERSE);
            group->appendTransform(drt);
            group->appendTransform(ap0);
        } else if (std::string(argv[1]) == "forward") {
            group->appendTransform(ap0);
            group->appendTransform(drt);
        } else {
            throw std::runtime_error("unknown direction");
        }
        auto processor = OCIO::Config::CreateRaw()->getProcessor(group)->getDefaultCPUProcessor();
        std::ifstream input(argv[2], std::ios::binary);
        std::ofstream output(argv[3], std::ios::binary);
        if (!input || !output)
            throw std::runtime_error("could not open reference streams");
        std::array<float, 3> rgb{};
        while (input.read(reinterpret_cast<char *>(rgb.data()), sizeof(rgb))) {
            processor->applyRGB(rgb.data());
            for (float value : rgb)
                if (!std::isfinite(value))
                    throw std::runtime_error("nonfinite reference output");
            output.write(reinterpret_cast<const char *>(rgb.data()), sizeof(rgb));
        }
        if (!input.eof() || input.gcount() != 0 || !output)
            throw std::runtime_error("invalid reference stream length or write failure");
    } catch (const std::exception &error) {
        std::cerr << error.what() << '\n';
        return 1;
    }
}
