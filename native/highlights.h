#pragma once
#include <algorithm>
#include <array>
#include <cmath>

namespace antitone {

// Camera-light Blend, with an independent capture balance rather than output WB.
// All working quantities use the least-amplified channel's white as one.
class HighlightBlend {
    std::array<float, 3> trigger{}, to_working{}, from_working{}, shoulder{};
    float ceiling = 0;
    bool available = false;

    static float chroma_energy(const std::array<float, 3> &value) {
        const float rg = value[0] - value[1];
        const float gb = value[1] - value[2];
        const float br = value[2] - value[0];
        return rg * rg + gb * gb + br * br;
    }

public:
    HighlightBlend(const std::array<float, 3> &white, const std::array<float, 3> &capture_balance,
                   bool integer_codes = false) {
        for (unsigned c = 0; c < 3; ++c)
            if (!std::isfinite(white[c]) || white[c] <= 0 || !std::isfinite(capture_balance[c]) ||
                capture_balance[c] <= 0)
                return;
        const float minimum =
            std::min({capture_balance[0], capture_balance[1], capture_balance[2]});
        const float maximum =
            std::max({capture_balance[0], capture_balance[1], capture_balance[2]});
        if (maximum / minimum > 256.f)
            return;
        std::array<float, 3> channel_white;
        for (unsigned c = 0; c < 3; ++c) {
            channel_white[c] = capture_balance[c] / minimum;
            trigger[c] = integer_codes ? std::floor(white[c]) : white[c];
            to_working[c] = channel_white[c] / white[c];
            from_working[c] = white[c] / channel_white[c];
            if (trigger[c] <= 0 || !std::isfinite(to_working[c]) || !std::isfinite(from_working[c]))
                return;
        }
        ceiling = channel_white[0] / 3.f + channel_white[1] / 3.f + channel_white[2] / 3.f;
        for (unsigned c = 0; c < 3; ++c)
            shoulder[c] = std::min(channel_white[c], ceiling);
        available = true;
    }

    bool supported() const {
        return available;
    }

    bool clipped(const std::array<float, 3> &pixel) const {
        return available &&
               (pixel[0] >= trigger[0] || pixel[1] >= trigger[1] || pixel[2] >= trigger[2]);
    }

    // The developer establishes an exact original-frame clipping anchor first.
    bool apply(std::array<float, 3> &pixel) const {
        if (!available)
            return false;
        std::array<float, 3> light, clipped_light;
        for (unsigned c = 0; c < 3; ++c) {
            light[c] = pixel[c] * to_working[c];
            if (!std::isfinite(light[c]) || light[c] < 0)
                return false;
            clipped_light[c] = std::min(light[c], 1.f);
        }
        // Below common white, both the chroma comparison and shoulders are identity.
        if (std::max({light[0], light[1], light[2]}) <= 1.f)
            return false;
        const float energy = chroma_energy(light);
        if (!std::isfinite(energy))
            return false;
        const float mean = light[0] / 3.f + light[1] / 3.f + light[2] / 3.f;
        if (energy == 0 && mean <= ceiling)
            return false;
        const float retained = energy > 0 ? std::sqrt(chroma_energy(clipped_light) / energy) : 0.f;
        std::array<float, 3> mixed;
        float capped_intensity = 0;
        float mixed_intensity = 0;
        for (unsigned c = 0; c < 3; ++c) {
            const float amount = std::clamp((light[c] - 0.5f) / (shoulder[c] - 0.5f), 0.f, 1.f);
            const float reduced = mean + retained * (light[c] - mean);
            mixed[c] = std::min(ceiling, light[c] + amount * amount * (reduced - light[c]));
            capped_intensity += std::min(light[c], shoulder[c]);
            mixed_intensity += mixed[c];
        }
        const float mixed_mean = mixed_intensity / 3.f;
        const float fade =
            ceiling > 1.f ? std::clamp(2.f * (ceiling - mixed_mean) / (ceiling - 1.f), 0.f, 1.f)
                          : 1.f;
        float saturation = mixed_intensity > 0 ? fade * capped_intensity / mixed_intensity : 0.f;
        // Preserve hue while keeping source light in [0, ceiling]. Downstream colour
        // matrices may be signed; negative-film inversion's camera light is not.
        for (float channel : mixed) {
            const float deviation = channel - mixed_mean;
            if (deviation < 0)
                saturation = std::min(saturation, mixed_mean / -deviation);
            else if (deviation > 0)
                saturation = std::min(saturation, (ceiling - mixed_mean) / deviation);
        }
        auto output = pixel;
        for (unsigned c = 0; c < 3; ++c) {
            const float result =
                std::clamp(mixed_mean + saturation * (mixed[c] - mixed_mean), 0.f, ceiling);
            output[c] = std::max(0.f, pixel[c] + (result - light[c]) * from_working[c]);
            if (!std::isfinite(output[c]))
                return false;
        }
        if (output == pixel)
            return false;
        pixel = output;
        return true;
    }
};

} // namespace antitone
