#pragma once
#include <OpenColorIO/OpenColorIO.h>
#include <array>
#include <stdexcept>

namespace antitone {
namespace OCIO = OCIO_NAMESPACE;

inline const std::array<double, 9> &output_parameters(unsigned target) {
    static constexpr auto rec2020 = [](double peak) {
        return std::array<double, 9>{peak, .708, .292, .170, .797, .131, .046, .3127, .3290};
    };
    static constexpr std::array targets{
        std::array{100., .64, .33, .30, .60, .15, .06, .3127, .3290},
        std::array{100., .68, .32, .265, .69, .15, .06, .3127, .3290},
        rec2020(500.),
        rec2020(1000.),
        rec2020(2000.),
        rec2020(4000.),
    };
    return targets.at(target);
}

inline OCIO::ConstProcessorRcPtr rendering(unsigned target, bool ap0_reference = false) {
    auto group = OCIO::GroupTransform::Create();
    if (ap0_reference) {
        auto toAP1 = OCIO::BuiltinTransform::Create();
        toAP1->setStyle("ACEScg_to_ACES2065-1");
        toAP1->setDirection(OCIO::TRANSFORM_DIR_INVERSE);
        group->appendTransform(toAP1);
    }
    auto inputRange = OCIO::RangeTransform::Create();
    inputRange->setMinInValue(0);
    inputRange->setMaxInValue(1024);
    inputRange->setMinOutValue(0);
    inputRange->setMaxOutValue(1024);
    group->appendTransform(inputRange);
    auto toAP0 = OCIO::BuiltinTransform::Create();
    toAP0->setStyle("ACEScg_to_ACES2065-1");
    group->appendTransform(toAP0);
    const auto &params = output_parameters(target);
    group->appendTransform(OCIO::FixedFunctionTransform::Create(
        OCIO::FIXED_FUNCTION_ACES_OUTPUT_TRANSFORM_20, params.data(), params.size()));
    // ACES display-linear uses 100 cd/m² per unit, independently of the target peak.
    const double peak = params[0] / 100.;
    auto outputRange = OCIO::RangeTransform::Create();
    outputRange->setMinInValue(0);
    outputRange->setMaxInValue(peak);
    outputRange->setMinOutValue(0);
    outputRange->setMaxOutValue(peak);
    group->appendTransform(outputRange);
    return OCIO::Config::CreateRaw()->getProcessor(group);
}
}
