// Source-owned gradient observations. Strength and polarity are not material identity.
static float quantile(std::vector<float> values, double q) {
    size_t k = size_t(q * (values.size() - 1));
    std::nth_element(values.begin(), values.begin() + k, values.end());
    return values[k];
}
struct AlongGradient { std::vector<float> values; double first = 0, last = 0; };
static AlongGradient gradientAlong(const std::vector<float> &gradient, unsigned w, unsigned h, bool vertical, double position, double angle,
                                  double first = .1, double last = .9, unsigned count = 96, unsigned marginX = 1, unsigned marginY = 1) {
    const double across = vertical ? h : w, center = (across - 1) / 2.;
    const double slope = std::tan(angle * M_PI / 180.);
    AlongGradient result;
    for (unsigned sample = 0; sample < count; ++sample) {
        const double fraction = first + (last - first) * sample / (count - 1);
        const double along = (across - 1) * fraction;
        const double cross = position + slope * (along - center);
        const double px = vertical ? cross : along, py = vertical ? along : cross;
        // All four interpolation corners need measured gradients, including the widened derivative border.
        if (px < marginX || py < marginY || px >= w - marginX - 1 || py >= h - marginY - 1) continue;
        const unsigned x = std::floor(px), y = std::floor(py);
        const double dx = px - x, dy = py - y;
        const size_t i = size_t(y) * w + x;
        if (result.values.empty()) result.first = fraction;
        result.last = fraction;
        result.values.push_back((gradient[i] * (1 - dx) + gradient[i + 1] * dx) * (1 - dy)
            + (gradient[i + w] * (1 - dx) + gradient[i + w + 1] * dx) * dy);
    }
    return result;
}
static id surfaceAlong(const std::vector<uint16_t> &rgb, unsigned w, unsigned h, bool vertical, double position, double angle,
                       double first = .1, double last = .9, unsigned count = 96) {
    const double across = vertical ? h : w, center = (across - 1) / 2.;
    const double slope = std::tan(angle * M_PI / 180.);
    std::array<std::vector<float>, 3> values;
    double observedFirst = 0, observedLast = 0;
    for (unsigned sample = 0; sample < count; ++sample) {
        const double fraction = first + (last - first) * sample / (count - 1);
        const double along = (across - 1) * fraction;
        const double cross = position + slope * (along - center);
        const int x = std::lround(vertical ? cross : along), y = std::lround(vertical ? along : cross);
        if (x < 0 || y < 0 || x >= int(w) || y >= int(h)) continue;
        if (values[0].empty()) observedFirst = fraction;
        observedLast = fraction;
        for (unsigned c = 0; c < 3; ++c) values[c].push_back(rgb[(size_t(y) * w + x) * 3 + c] / 65535.f);
    }
    if (values[0].size() < count / 2) return [NSNull null];
    return @{@"sample_fraction": @(double(values[0].size()) / count),
        @"span_fraction": @[@(observedFirst), @(observedLast)], @"sample_count": @(values[0].size()),
        @"median_camera_rgb": @[@(quantile(values[0], .5)), @(quantile(values[1], .5)), @(quantile(values[2], .5))]};
}
static id normalTransition(const std::vector<uint16_t> &rgb, unsigned w, unsigned h, bool vertical, double position, double angle,
                           double first, double last, unsigned count, double scaleX = 1., double scaleY = 1.) {
    const double slope = std::tan(angle * M_PI / 180.), scale = std::sqrt(1 + slope * slope);
    const double nx = (vertical ? 1. : -slope) / scale * scaleX, ny = (vertical ? -slope : 1.) / scale * scaleY;
    const double across = vertical ? h : w;
    std::vector<std::array<double, 3>> deltas;
    std::array<double, 3> mean = {};
    auto sample = [&](double x, double y, unsigned c) {
        const unsigned ix = std::floor(x), iy = std::floor(y);
        const double dx = x - ix, dy = y - iy;
        const size_t i = (size_t(iy) * w + ix) * 3 + c;
        return ((rgb[i] * (1 - dx) + rgb[i + 3] * dx) * (1 - dy)
            + (rgb[i + w * 3] * (1 - dx) + rgb[i + w * 3 + 3] * dx) * dy) / 65535.;
    };
    for (unsigned k = 0; k < count; ++k) {
        const double along = (across - 1) * (first + (last - first) * k / (count - 1));
        const double cross = position + slope * (along - (across - 1) / 2.);
        const double x = vertical ? cross : along, y = vertical ? along : cross;
        if (x - std::abs(nx) < 0 || y - std::abs(ny) < 0 || x + std::abs(nx) >= w - 1 || y + std::abs(ny) >= h - 1) continue;
        std::array<double, 3> delta;
        for (unsigned c = 0; c < 3; ++c) { delta[c] = sample(x + nx, y + ny, c) - sample(x - nx, y - ny, c); mean[c] += delta[c]; }
        deltas.push_back(delta);
    }
    if (deltas.size() < count / 2) return [NSNull null];
    double magnitude = 0;
    for (double &v : mean) { v /= deltas.size(); magnitude += v * v; }
    magnitude = std::sqrt(magnitude);
    std::vector<float> projections;
    for (const auto &delta : deltas) {
        double dot = 0;
        for (unsigned c = 0; c < 3; ++c) dot += delta[c] * mean[c];
        projections.push_back(magnitude > 0 ? dot / magnitude : 0);
    }
    return @{@"mean_camera_delta": @[@(mean[0]), @(mean[1]), @(mean[2])],
        @"lower_quartile_projection": @(quantile(std::move(projections), .25)), @"sample_count": @(deltas.size())};
}
static bool opposedTransitions(id a, id b) {
    if (a == [NSNull null] || b == [NSNull null]) return false;
    if ([a[@"lower_quartile_projection"] doubleValue] <= 0 || [b[@"lower_quartile_projection"] doubleValue] <= 0) return false;
    double dot = 0;
    for (unsigned c = 0; c < 3; ++c) dot += [a[@"mean_camera_delta"][c] doubleValue] * [b[@"mean_camera_delta"][c] doubleValue];
    return dot < 0;
}
static NSArray *orientedLines(const std::vector<float> &gradient, const std::vector<uint16_t> &rgb, unsigned w, unsigned h, bool vertical, int first, int end,
                              double spanFirst = .1, double spanLast = .9, unsigned samples = 96, unsigned maximum = 5, bool partial = false,
                              const std::vector<double> &requestedAngles = {}, bool preserveOpposed = false, double scaleX = 1., double scaleY = 1.) {
    const double crossScale = vertical ? scaleX : scaleY;
    const unsigned marginX = std::lround(scaleX), marginY = std::lround(scaleY);
    struct Line { double score, angle, position, visible; };
    std::vector<Line> candidates;
    auto angles = requestedAngles;
    if (angles.empty()) for (int step = -24; step <= 24; ++step) angles.push_back(step * .25);
    // A small-angle continuity baseline, not an implementation of LSD/EDLines or a format prior.
    for (int position = first; position < end; ++position) {
        Line best = {};
        for (double angle : angles) {
            auto strengths = gradientAlong(gradient, w, h, vertical, position, angle, spanFirst, spanLast, samples, marginX, marginY);
            if (strengths.values.size() < samples / 2) continue;
            const double visible = double(strengths.values.size()) / samples;
            const double score = quantile(std::move(strengths.values), .25) * visible;
            if (score > best.score) best = {score, angle, double(position), visible};
        }
        if (best.score > 0) candidates.push_back(best);
    }
    std::sort(candidates.begin(), candidates.end(), [](auto a, auto b) { return a.score > b.score; });
    NSMutableArray *result = [NSMutableArray array];
    std::vector<double> accepted;
    NSMutableArray *transitions = [NSMutableArray array];
    for (const auto &line : candidates) {
        const double position = line.position;
        id transition = normalTransition(rgb, w, h, vertical, position, line.angle, spanFirst, spanLast, samples, scaleX, scaleY);
        bool blocked = false;
        for (size_t i = 0; i < accepted.size(); ++i) {
            const double distance = std::abs(position - accepted[i]);
            if (distance < 6 * crossScale && !(preserveOpposed && distance >= 2 * crossScale && opposedTransitions(transition, transitions[i]))) { blocked = true; break; }
        }
        if (blocked) continue;
        accepted.push_back(position); [transitions addObject:transition];
        double local = 0; unsigned shoulders = 0;
        for (double offset : {-6., 6.}) {
            auto values = gradientAlong(gradient, w, h, vertical, position + offset * crossScale, line.angle, spanFirst, spanLast, samples, marginX, marginY);
            if (values.values.size() >= samples / 2) { local += quantile(std::move(values.values), .25); ++shoulders; }
        }
        const double outward = (first < int((vertical ? w : h) / 2) ? -6. : 6.) * crossScale;
        auto observed = gradientAlong(gradient, w, h, vertical, position, line.angle, spanFirst, spanLast, samples, marginX, marginY);
        [result addObject:@{@"midpoint_pixel": @(position), @"angle_degrees": @(line.angle),
            @"lower_quartile_gradient_score": @(line.score), @"visible_sample_fraction": @(line.visible),
            @"angle_limit_reached": @(std::abs(line.angle) >= 6.),
            @"local_gradient_background": shoulders ? @(local / shoulders) : (id)[NSNull null],
            @"observed_shoulders": @(shoulders),
            @"outside_surface": surfaceAlong(rgb, w, h, vertical, position + outward, line.angle, spanFirst, spanLast, samples),
            @"inside_surface": surfaceAlong(rgb, w, h, vertical, position - outward, line.angle, spanFirst, spanLast, samples),
            @"span_fraction": @[@(observed.first), @(observed.last)], @"partial_span": @(partial), @"normal_transition": transition}];
        if (accepted.size() == maximum) break;
    }
    return result;
}
static std::vector<double> segmentPoseGuides(NSDictionary *lines) {
    struct Pose { unsigned side; double angle; };
    std::vector<Pose> poses;
    unsigned side = 0;
    for (NSString *name in @[@"left", @"right", @"top", @"bottom"]) {
        for (NSDictionary *line in lines[name]) {
            id background = line[@"local_gradient_background"];
            if (background == [NSNull null] || [line[@"angle_limit_reached"] boolValue]) continue;
            const double ratio = [line[@"lower_quartile_gradient_score"] doubleValue] / std::max(1. / 65535., [background doubleValue]);
            if (ratio >= 4.) poses.push_back({side, [line[@"angle_degrees"] doubleValue] * (side < 2 ? -1. : 1.)});
        }
        ++side;
    }
    std::vector<double> guides;
    for (size_t i = 0; i < poses.size(); ++i) for (size_t j = i + 1; j < poses.size(); ++j)
        if (poses[i].side != poses[j].side && std::abs(poses[i].angle - poses[j].angle) <= 1.5)
            guides.push_back((poses[i].angle + poses[j].angle) / 2.);
    std::sort(guides.begin(), guides.end());
    guides.erase(std::unique(guides.begin(), guides.end()), guides.end());
    return guides;
}
static NSArray *segmentLines(const std::vector<float> &gradient, const std::vector<uint16_t> &rgb, unsigned w, unsigned h, bool vertical, int first, int end,
                            const std::vector<double> &poses, bool preserveOpposed, double scaleX = 1., double scaleY = 1.) {
    NSMutableArray *result = [NSMutableArray array];
    if (poses.empty()) return result;
    auto angles = poses;
    if (vertical) for (double &angle : angles) angle = -angle;
    for (unsigned window = 0; window < 8; ++window) {
        const double start = window == 7 ? .70 : .05 + window * .10;
        [result addObjectsFromArray:orientedLines(gradient, rgb, w, h, vertical, first, end, start, start + .25, 48, 3, true, angles, preserveOpposed, scaleX, scaleY)];
    }
    return result;
}
