// Bounded camera-code geometry only; no RAW development, repair or processing eligibility.
#include <algorithm>
#include <array>
#include <cmath>
#include <cstddef>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <string>
#include <vector>
#include "logging.h"
#include "scan_region_json.h"
#include "scan_region_lines.h"

static Json surface_json(const std::optional<Surface> &surface) {
    if (!surface)
        return Json::null();
    return Json::object({
        {"sample_fraction", Json::number(surface->sample_fraction)},
        {"span_fraction",
         Json::array({Json::number(surface->span_fraction[0]),
                      Json::number(surface->span_fraction[1])})},
        {"sample_count", Json::number(double(surface->sample_count))},
        {"median_camera_rgb",
         Json::array({Json::number(surface->median_camera_rgb[0]),
                      Json::number(surface->median_camera_rgb[1]),
                      Json::number(surface->median_camera_rgb[2])})},
    });
}

static Json transition_json(const std::optional<Transition> &transition) {
    if (!transition)
        return Json::null();
    return Json::object({
        {"mean_camera_delta",
         Json::array({Json::number(transition->mean_camera_delta[0]),
                      Json::number(transition->mean_camera_delta[1]),
                      Json::number(transition->mean_camera_delta[2])})},
        {"lower_quartile_projection", Json::number(transition->lower_quartile_projection)},
        {"sample_count", Json::number(double(transition->sample_count))},
    });
}

static Json line_json(const Line &line) {
    return Json::object({
        {"midpoint_pixel", Json::number(line.midpoint_pixel)},
        {"angle_degrees", Json::number(line.angle_degrees)},
        {"lower_quartile_gradient_score", Json::number(line.lower_quartile_gradient_score)},
        {"visible_sample_fraction", Json::number(line.visible_sample_fraction)},
        {"angle_limit_reached", Json::boolean(line.angle_limit_reached)},
        {"local_gradient_background",
         line.local_gradient_background ? Json::number(*line.local_gradient_background)
                                        : Json::null()},
        {"observed_shoulders", Json::number(double(line.observed_shoulders))},
        {"outside_surface", surface_json(line.outside_surface)},
        {"inside_surface", surface_json(line.inside_surface)},
        {"span_fraction",
         Json::array({Json::number(line.span_fraction[0]),
                      Json::number(line.span_fraction[1])})},
        {"partial_span", Json::boolean(line.partial_span)},
        {"normal_transition", transition_json(line.normal_transition)},
    });
}

static Json lines_json(const std::vector<Line> &lines) {
    Json::Array result;
    result.reserve(lines.size());
    for (const auto &line : lines)
        result.push_back(line_json(line));
    return Json::array(std::move(result));
}

static Json sides_json(const std::array<std::vector<Line>, 4> &sides) {
    return Json::object({
        {"left", lines_json(sides[0])},
        {"right", lines_json(sides[1])},
        {"top", lines_json(sides[2])},
        {"bottom", lines_json(sides[3])},
    });
}

extern "C" char *antitone_scan_region_lines(const uint16_t *input, unsigned w, unsigned h) {
    antitone::NativeLog diagnostic("regions", "lines");
    try {
        if (!input || w < 16 || h < 16 || w > 768 || h > 768)
            return nullptr;
        std::vector<uint16_t> rgb(input, input + size_t(w) * h * 3);
        std::vector<float> gx(size_t(w) * h), gy(size_t(w) * h);
        for (unsigned y = 1; y + 1 < h; ++y)
            for (unsigned x = 1; x + 1 < w; ++x) {
                const size_t i = size_t(y) * w + x;
                double dx = 0, dy = 0;
                for (unsigned c = 0; c < 3; ++c) {
                    const double a =
                        (int(rgb[(i + 1) * 3 + c]) - int(rgb[(i - 1) * 3 + c])) / 65535.;
                    const double b =
                        (int(rgb[(i + w) * 3 + c]) - int(rgb[(i - w) * 3 + c])) / 65535.;
                    dx += a * a;
                    dy += b * b;
                }
                gx[i] = std::sqrt(dx);
                gy[i] = std::sqrt(dy);
            }
        std::array<std::vector<Line>, 4> lines = {
            orientedLines(gx, rgb, w, h, true, 1, w / 4, .1, .9, 96, 5, false, {}, true),
            orientedLines(gx, rgb, w, h, true, w * 3 / 4, w - 1, .1, .9, 96, 5, false, {}, true),
            orientedLines(gy, rgb, w, h, false, 1, h / 4, .1, .9, 96, 5, false, {}, true),
            orientedLines(gy, rgb, w, h, false, h * 3 / 4, h - 1, .1, .9, 96, 5, false, {}, true),
        };
        const auto poses = segmentPoseGuides(lines);
        std::array<std::vector<Line>, 4> segments = {
            segmentLines(gx, rgb, w, h, true, 1, w / 4, poses, true),
            segmentLines(gx, rgb, w, h, true, w * 3 / 4, w - 1, poses, true),
            segmentLines(gy, rgb, w, h, false, 1, h / 4, poses, true),
            segmentLines(gy, rgb, w, h, false, h * 3 / 4, h - 1, poses, true),
        };
        Json record = Json::object({
            {"lines", sides_json(lines)},
            {"segments", sides_json(segments)},
        });
        std::string json;
        record.write(json);
        auto *result = static_cast<char *>(std::malloc(json.size() + 1));
        if (!result)
            return nullptr;
        std::memcpy(result, json.data(), json.size());
        result[json.size()] = 0;
        diagnostic.complete();
        return result;
    } catch (...) {
        diagnostic.complete(1, 2);
        return nullptr;
    }
}

extern "C" void antitone_scan_region_lines_free(char *data) {
    std::free(data);
}
