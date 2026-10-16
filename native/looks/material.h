#pragma once
#include "look_data.h"
#include "math.h"

namespace looks {
using namespace look_data;

struct Material {
    V exposure0{};
    double compression;
    static constexpr double logHPerStop = -.30102999566398119521;
#ifndef LOOK_NEUTRAL_DENSITY
    std::array<V, std::size(wavelengths)> weights{};
#endif
    explicit Material(double value = exposure_compression) : compression(value) {
        if (!std::isfinite(value) || value <= 0 || value > 1)
            throw std::runtime_error("exposure compression must be finite and within (0,1]");
#ifndef LOOK_NEUTRAL_DENSITY
        V white{};
        double norm = 0;
        for (size_t i = 0; i < weights.size(); ++i) {
            norm += illumination[i] * observer[i][1];
            for (int c = 0; c < 3; ++c)
                white[c] += illumination[i] * observer[i][c];
        }
        white = white * (1 / norm);
        M toAP1 = mul(inverse(AP1_XYZ), adaptation(white, mul(AP1_XYZ, V{1, 1, 1})));
        for (size_t i = 0; i < weights.size(); ++i)
            weights[i] = mul(toAP1, V{observer[i][0], observer[i][1], observer[i][2]}) *
                         (illumination[i] / norm);
#endif
        // Preserve the original solve sequence and its numerical identity.
        calibrate(.18);
        calibrate(paperGray());
    }

    V attenuate(V h) const {
#ifdef LOOK_NEUTRAL_DENSITY
        double x = std::clamp(h[0], log_exposure[0], log_exposure[255]);
        double z = -mono_parameters[2] * (x - mono_parameters[3]);
        double d = mono_parameters[0] + (mono_parameters[1] - mono_parameters[0]) *
                                            std::exp(-mono_parameters[4] * std::log1p(std::exp(z)));
        double r = std::pow(10., -d);
        return V{r, r, r};
#else
        V density{};
        for (int c = 0; c < 3; ++c) {
            double at = (h[c] - log_exposure[0]) / (log_exposure[1] - log_exposure[0]);
            at = std::clamp(at, 0., 255.);
            int low = std::min(254, int(at));
            double f = at - low;
            density[c] = density_curve[low][c] * (1 - f) + density_curve[low + 1][c] * f;
        }
        V rgb{};
        for (size_t i = 0; i < weights.size(); ++i) {
            double od = base[i];
            for (int c = 0; c < 3; ++c)
                od += dye[i][c] * density[c];
            rgb = rgb + weights[i] * std::pow(10., -od);
        }
        return rgb;
#endif
    }

    void calibrate(double gray) {
#ifdef LOOK_NEUTRAL_DENSITY
        double lo = log_exposure[0], hi = log_exposure[255];
        if (gray > attenuate(V{lo, lo, lo})[0] || gray < attenuate(V{hi, hi, hi})[0])
            throw std::runtime_error("scalar gray outside source response");
        for (int i = 0; i < 100; ++i) {
            double mid = (lo + hi) * .5;
            if (attenuate(V{mid, mid, mid})[0] > gray)
                lo = mid;
            else
                hi = mid;
        }
        exposure0.fill((lo + hi) * .5);
#else
        exposure0.fill((log_exposure[0] + log_exposure[255]) * .5);
        for (int iteration = 0; iteration < 30; ++iteration) {
            V error = attenuate(exposure0) - V{gray, gray, gray};
            if (maxabs(error) < 1e-12)
                break;
            M j{};
            for (int c = 0; c < 3; ++c) {
                V h = exposure0;
                h[c] += 1e-5;
                V diff = (attenuate(h) - attenuate(exposure0)) * (1e5);
                for (int r = 0; r < 3; ++r)
                    j[r][c] = diff[r];
            }
            V delta = mul(inverse(j), error);
            if (maxabs(delta) > .25)
                delta = delta * (.25 / maxabs(delta));
            exposure0 = exposure0 - delta;
        }
#endif
        if (maxabs(attenuate(exposure0) - V{gray, gray, gray}) > 1e-9)
            throw std::runtime_error("gray calibration failed");
    }

    V operator()(V input) const {
        for (double c : input)
            if (!std::isfinite(c))
                throw std::runtime_error("nonfinite AP1 input");
#ifdef LOOK_NEUTRAL_DENSITY
        // This reduction must precede the nonlinear exposure mapping.
        double y = dot(AP1_XYZ[1], input), stops = 17.52 * (encode(y) - encode(.18));
        double h = exposure0[0] + stops * (logHPerStop * compression);
        return attenuate(V{h, h, h});
#else
        V stops{};
        for (int c = 0; c < 3; ++c)
            stops[c] = 17.52 * (encode(input[c]) - encode(.18));
        return attenuate(exposure0 + stops * (logHPerStop * compression));
#endif
    }
};
} // namespace looks
