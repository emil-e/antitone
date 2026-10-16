#pragma once
#include "material.h"
#include <OpenColorIO/OpenColorIO.h>

namespace looks {
namespace OCIO = OCIO_NAMESPACE;
inline V ocio(OCIO::ConstCPUProcessorRcPtr processor, V x) {
    float rgb[3] = {float(x[0]), float(x[1]), float(x[2])};
    processor->applyRGB(rgb);
    for (float c : rgb)
        if (!std::isfinite(c))
            throw std::runtime_error("nonfinite OCIO result");
    return {rgb[0], rgb[1], rgb[2]};
}
inline V xy(double x, double y) { return {x / y, 1, (1 - x - y) / y}; }
inline M primaries() {
    const auto &p = reference_parameters;
    M m{};
    for (int c = 0; c < 3; ++c) {
        V v = xy(p[1 + 2 * c], p[2 + 2 * c]);
        for (int r = 0; r < 3; ++r)
            m[r][c] = v[r];
    }
    V scale = mul(inverse(m), xy(p[7], p[8]));
    for (int r = 0; r < 3; ++r)
        for (int c = 0; c < 3; ++c)
            m[r][c] *= scale[c];
    return m;
}
inline OCIO::ConstCPUProcessorRcPtr rendering(bool inverse = false) {
    auto group = OCIO::GroupTransform::Create();
    auto ap0 = OCIO::BuiltinTransform::Create();
    ap0->setStyle("ACEScg_to_ACES2065-1");
    auto drt = OCIO::FixedFunctionTransform::Create(OCIO::FIXED_FUNCTION_ACES_OUTPUT_TRANSFORM_20,
                                                    reference_parameters, 9);
    if (inverse) {
        drt->setDirection(OCIO::TRANSFORM_DIR_INVERSE);
        group->appendTransform(drt);
        ap0->setDirection(OCIO::TRANSFORM_DIR_INVERSE);
        group->appendTransform(ap0);
    } else {
        group->appendTransform(ap0);
        group->appendTransform(drt);
    }
    return OCIO::Config::CreateRaw()->getProcessor(group)->getDefaultCPUProcessor();
}
struct Reference {
    Material source;
    M referenceXYZ, toReference;
    OCIO::ConstCPUProcessorRcPtr inverseDRT, forwardDRT;
    explicit Reference(double compression = exposure_compression)
        : source(compression), referenceXYZ(primaries()) {
        toReference = mul(mul(inverse(referenceXYZ),
                              adaptation(mul(AP1_XYZ, V{1, 1, 1}),
                                         xy(reference_parameters[7], reference_parameters[8]))),
                          AP1_XYZ);
        inverseDRT = rendering(true);
        forwardDRT = rendering();
    }
    V raw(V input) const { return source(input); }
    V map(V raw) const {
        V value = mul(toReference, raw);
        const double y = dot(referenceXYZ[1], value);
        if (!std::isfinite(y) || y <= 0 || y >= 1)
            throw std::runtime_error("reference luminance outside (0,1)");
        double radius = 0;
        for (double c : value)
            radius = std::max(radius, c >= y ? (c - y) / (1 - y) : (y - c) / y);
        double scale = 1;
        if (radius > .9) {
            const double compressed = .9 + .1 * (radius - .9) / (radius - .9 + .1);
            scale = compressed / radius;
        }
        for (double &c : value)
            c = y + scale * (c - y);
        if (!inCube(value))
            throw std::runtime_error("reference boundary mapping failed");
        return value;
    }
    V operator()(V input) const { return ocio(inverseDRT, map(raw(input))); }
};
} // namespace looks
