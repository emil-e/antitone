// Source-owned gradient observations. Strength and polarity are not material identity.
#pragma once
#include <algorithm>
#include <array>
#include <cmath>
#include <cstddef>
#include <cstdint>
#include <optional>
#include <utility>
#include <vector>

constexpr double kPi = 3.14159265358979323846;

static float quantile(std::vector<float> values, double q) {
    size_t k = size_t(q * (values.size() - 1));
    std::nth_element(values.begin(), values.begin() + k, values.end());
    return values[k];
}

struct AlongGradient {
    std::vector<float> values;
    double first = 0, last = 0;
};

static AlongGradient gradientAlong(const std::vector<float> &gradient, unsigned w, unsigned h,
                                   bool vertical, double position, double angle, double first = .1,
                                   double last = .9, unsigned count = 96, unsigned marginX = 1,
                                   unsigned marginY = 1) {
    const double across = vertical ? h : w, center = (across - 1) / 2.;
    const double slope = std::tan(angle * kPi / 180.);
    AlongGradient result;
    for (unsigned sample = 0; sample < count; ++sample) {
        const double fraction = first + (last - first) * sample / (count - 1);
        const double along = (across - 1) * fraction;
        const double cross = position + slope * (along - center);
        const double px = vertical ? cross : along, py = vertical ? along : cross;
        // All four interpolation corners need measured gradients, including the widened derivative border.
        if (px < marginX || py < marginY || px >= w - marginX - 1 || py >= h - marginY - 1)
            continue;
        const unsigned x = std::floor(px), y = std::floor(py);
        const double dx = px - x, dy = py - y;
        const size_t i = size_t(y) * w + x;
        if (result.values.empty())
            result.first = fraction;
        result.last = fraction;
        result.values.push_back((gradient[i] * (1 - dx) + gradient[i + 1] * dx) * (1 - dy) +
                                (gradient[i + w] * (1 - dx) + gradient[i + w + 1] * dx) * dy);
    }
    return result;
}

struct Surface {
    double sample_fraction;
    std::array<double, 2> span_fraction;
    size_t sample_count;
    std::array<double, 3> median_camera_rgb;
};

static std::optional<Surface> surfaceAlong(const std::vector<uint16_t> &rgb, unsigned w,
                                           unsigned h, bool vertical, double position,
                                           double angle, double first = .1, double last = .9,
                                           unsigned count = 96) {
    const double across = vertical ? h : w, center = (across - 1) / 2.;
    const double slope = std::tan(angle * kPi / 180.);
    std::array<std::vector<float>, 3> values;
    double observedFirst = 0, observedLast = 0;
    for (unsigned sample = 0; sample < count; ++sample) {
        const double fraction = first + (last - first) * sample / (count - 1);
        const double along = (across - 1) * fraction;
        const double cross = position + slope * (along - center);
        const int x = std::lround(vertical ? cross : along),
                  y = std::lround(vertical ? along : cross);
        if (x < 0 || y < 0 || x >= int(w) || y >= int(h))
            continue;
        if (values[0].empty())
            observedFirst = fraction;
        observedLast = fraction;
        for (unsigned c = 0; c < 3; ++c)
            values[c].push_back(rgb[(size_t(y) * w + x) * 3 + c] / 65535.f);
    }
    if (values[0].size() < count / 2)
        return std::nullopt;
    return Surface{
        double(values[0].size()) / count,
        {observedFirst, observedLast},
        values[0].size(),
        {quantile(values[0], .5), quantile(values[1], .5), quantile(values[2], .5)},
    };
}

struct Transition {
    std::array<double, 3> mean_camera_delta;
    double lower_quartile_projection;
    size_t sample_count;
};

static std::optional<Transition> normalTransition(const std::vector<uint16_t> &rgb, unsigned w,
                                                  unsigned h, bool vertical, double position,
                                                  double angle, double first, double last,
                                                  unsigned count, double scaleX = 1.,
                                                  double scaleY = 1.) {
    const double slope = std::tan(angle * kPi / 180.), scale = std::sqrt(1 + slope * slope);
    const double nx = (vertical ? 1. : -slope) / scale * scaleX,
                 ny = (vertical ? -slope : 1.) / scale * scaleY;
    const double across = vertical ? h : w;
    std::vector<std::array<double, 3>> deltas;
    std::array<double, 3> mean = {};
    auto sample = [&](double x, double y, unsigned c) {
        const unsigned ix = std::floor(x), iy = std::floor(y);
        const double dx = x - ix, dy = y - iy;
        const size_t i = (size_t(iy) * w + ix) * 3 + c;
        return ((rgb[i] * (1 - dx) + rgb[i + 3] * dx) * (1 - dy) +
                (rgb[i + w * 3] * (1 - dx) + rgb[i + w * 3 + 3] * dx) * dy) /
               65535.;
    };
    for (unsigned k = 0; k < count; ++k) {
        const double along = (across - 1) * (first + (last - first) * k / (count - 1));
        const double cross = position + slope * (along - (across - 1) / 2.);
        const double x = vertical ? cross : along, y = vertical ? along : cross;
        if (x - std::abs(nx) < 0 || y - std::abs(ny) < 0 || x + std::abs(nx) >= w - 1 ||
            y + std::abs(ny) >= h - 1)
            continue;
        std::array<double, 3> delta;
        for (unsigned c = 0; c < 3; ++c) {
            delta[c] = sample(x + nx, y + ny, c) - sample(x - nx, y - ny, c);
            mean[c] += delta[c];
        }
        deltas.push_back(delta);
    }
    if (deltas.size() < count / 2)
        return std::nullopt;
    double magnitude = 0;
    for (double &v : mean) {
        v /= deltas.size();
        magnitude += v * v;
    }
    magnitude = std::sqrt(magnitude);
    std::vector<float> projections;
    for (const auto &delta : deltas) {
        double dot = 0;
        for (unsigned c = 0; c < 3; ++c)
            dot += delta[c] * mean[c];
        projections.push_back(magnitude > 0 ? dot / magnitude : 0);
    }
    return Transition{
        {mean[0], mean[1], mean[2]},
        quantile(std::move(projections), .25),
        deltas.size(),
    };
}

static bool opposedTransitions(const std::optional<Transition> &a,
                               const std::optional<Transition> &b) {
    if (!a || !b || a->lower_quartile_projection <= 0 || b->lower_quartile_projection <= 0)
        return false;
    double dot = 0;
    for (unsigned c = 0; c < 3; ++c)
        dot += a->mean_camera_delta[c] * b->mean_camera_delta[c];
    return dot < 0;
}

struct Line {
    double midpoint_pixel;
    double angle_degrees;
    double lower_quartile_gradient_score;
    double visible_sample_fraction;
    bool angle_limit_reached;
    std::optional<double> local_gradient_background;
    unsigned observed_shoulders;
    std::optional<Surface> outside_surface;
    std::optional<Surface> inside_surface;
    std::array<double, 2> span_fraction;
    bool partial_span;
    std::optional<Transition> normal_transition;
};

static std::vector<Line> orientedLines(
    const std::vector<float> &gradient, const std::vector<uint16_t> &rgb, unsigned w, unsigned h,
    bool vertical, int first, int end, double spanFirst = .1, double spanLast = .9,
    unsigned samples = 96, unsigned maximum = 5, bool partial = false,
    const std::vector<double> &requestedAngles = {}, bool preserveOpposed = false,
    double scaleX = 1., double scaleY = 1.) {
    const double crossScale = vertical ? scaleX : scaleY;
    const unsigned marginX = std::lround(scaleX), marginY = std::lround(scaleY);

    struct Candidate {
        double score, angle, position, visible;
    };

    std::vector<Candidate> candidates;
    auto angles = requestedAngles;
    if (angles.empty())
        for (int step = -24; step <= 24; ++step)
            angles.push_back(step * .25);
    // A small-angle continuity baseline, not an implementation of LSD/EDLines or a format prior.
    for (int position = first; position < end; ++position) {
        Candidate best = {};
        for (double angle : angles) {
            auto strengths = gradientAlong(gradient, w, h, vertical, position, angle, spanFirst,
                                           spanLast, samples, marginX, marginY);
            if (strengths.values.size() < samples / 2)
                continue;
            const double visible = double(strengths.values.size()) / samples;
            const double score = quantile(std::move(strengths.values), .25) * visible;
            if (score > best.score)
                best = {score, angle, double(position), visible};
        }
        if (best.score > 0)
            candidates.push_back(best);
    }
    std::sort(candidates.begin(), candidates.end(),
              [](auto a, auto b) { return a.score > b.score; });
    std::vector<Line> result;
    std::vector<double> accepted;
    std::vector<std::optional<Transition>> transitions;
    for (const auto &line : candidates) {
        const double position = line.position;
        auto transition = normalTransition(rgb, w, h, vertical, position, line.angle, spanFirst,
                                           spanLast, samples, scaleX, scaleY);
        bool blocked = false;
        for (size_t i = 0; i < accepted.size(); ++i) {
            const double distance = std::abs(position - accepted[i]);
            if (distance < 6 * crossScale && !(preserveOpposed && distance >= 2 * crossScale &&
                                               opposedTransitions(transition, transitions[i]))) {
                blocked = true;
                break;
            }
        }
        if (blocked)
            continue;
        accepted.push_back(position);
        transitions.push_back(transition);
        double local = 0;
        unsigned shoulders = 0;
        for (double offset : {-6., 6.}) {
            auto values = gradientAlong(gradient, w, h, vertical, position + offset * crossScale,
                                        line.angle, spanFirst, spanLast, samples, marginX, marginY);
            if (values.values.size() >= samples / 2) {
                local += quantile(std::move(values.values), .25);
                ++shoulders;
            }
        }
        const double outward = (first < int((vertical ? w : h) / 2) ? -6. : 6.) * crossScale;
        auto observed = gradientAlong(gradient, w, h, vertical, position, line.angle, spanFirst,
                                      spanLast, samples, marginX, marginY);
        result.push_back(Line{
            position,
            line.angle,
            line.score,
            line.visible,
            std::abs(line.angle) >= 6.,
            shoulders ? std::optional<double>(local / shoulders) : std::nullopt,
            shoulders,
            surfaceAlong(rgb, w, h, vertical, position + outward, line.angle, spanFirst, spanLast,
                         samples),
            surfaceAlong(rgb, w, h, vertical, position - outward, line.angle, spanFirst, spanLast,
                         samples),
            {observed.first, observed.last},
            partial,
            std::move(transition),
        });
        if (accepted.size() == maximum)
            break;
    }
    return result;
}

static std::vector<double> segmentPoseGuides(const std::array<std::vector<Line>, 4> &lines) {
    struct Pose {
        unsigned side;
        double angle;
    };

    std::vector<Pose> poses;
    for (unsigned side = 0; side < 4; ++side) {
        for (const auto &line : lines[side]) {
            if (!line.local_gradient_background || line.angle_limit_reached)
                continue;
            const double ratio = line.lower_quartile_gradient_score /
                                 std::max(1. / 65535., *line.local_gradient_background);
            if (ratio >= 4.)
                poses.push_back(
                    {side, line.angle_degrees * (side < 2 ? -1. : 1.)});
        }
    }
    std::vector<double> guides;
    for (size_t i = 0; i < poses.size(); ++i)
        for (size_t j = i + 1; j < poses.size(); ++j)
            if (poses[i].side != poses[j].side && std::abs(poses[i].angle - poses[j].angle) <= 1.5)
                guides.push_back((poses[i].angle + poses[j].angle) / 2.);
    std::sort(guides.begin(), guides.end());
    guides.erase(std::unique(guides.begin(), guides.end()), guides.end());
    return guides;
}

static std::vector<Line> segmentLines(const std::vector<float> &gradient,
                                      const std::vector<uint16_t> &rgb, unsigned w, unsigned h,
                                      bool vertical, int first, int end,
                                      const std::vector<double> &poses, bool preserveOpposed,
                                      double scaleX = 1., double scaleY = 1.) {
    std::vector<Line> result;
    if (poses.empty())
        return result;
    auto angles = poses;
    if (vertical)
        for (double &angle : angles)
            angle = -angle;
    for (unsigned window = 0; window < 8; ++window) {
        const double start = window == 7 ? .70 : .05 + window * .10;
        auto lines = orientedLines(gradient, rgb, w, h, vertical, first, end, start, start + .25, 48,
                                   3, true, angles, preserveOpposed, scaleX, scaleY);
        result.insert(result.end(), lines.begin(), lines.end());
    }
    return result;
}
