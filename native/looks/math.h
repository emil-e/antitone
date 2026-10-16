#pragma once
#include <algorithm>
#include <array>
#include <cmath>
#include <stdexcept>

namespace looks {
using V = std::array<double, 3>;
using M = std::array<V, 3>;

inline V operator+(V a, V b) {
    for (int i = 0; i < 3; ++i)
        a[i] += b[i];
    return a;
}

inline V operator-(V a, V b) {
    for (int i = 0; i < 3; ++i)
        a[i] -= b[i];
    return a;
}

inline V operator*(V a, double b) {
    for (auto &x : a)
        x *= b;
    return a;
}

inline double dot(V a, V b) {
    return a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
}

inline V mul(M a, V b) {
    return {dot(a[0], b), dot(a[1], b), dot(a[2], b)};
}

inline M mul(M a, M b) {
    M c{};
    for (int i = 0; i < 3; ++i)
        for (int j = 0; j < 3; ++j)
            for (int k = 0; k < 3; ++k)
                c[i][j] += a[i][k] * b[k][j];
    return c;
}

inline M inverse(M a) {
    M b{{{1, 0, 0}, {0, 1, 0}, {0, 0, 1}}};
    for (int j = 0; j < 3; ++j) {
        int pivot = j;
        for (int i = j + 1; i < 3; ++i)
            if (std::abs(a[i][j]) > std::abs(a[pivot][j]))
                pivot = i;
        if (std::abs(a[pivot][j]) < 1e-15)
            throw std::runtime_error("singular matrix");
        std::swap(a[j], a[pivot]);
        std::swap(b[j], b[pivot]);
        double scale = a[j][j];
        for (int k = 0; k < 3; ++k) {
            a[j][k] /= scale;
            b[j][k] /= scale;
        }
        for (int i = 0; i < 3; ++i)
            if (i != j) {
                double s = a[i][j];
                for (int k = 0; k < 3; ++k) {
                    a[i][k] -= s * a[j][k];
                    b[i][k] -= s * b[j][k];
                }
            }
    }
    return b;
}

inline const M AP1_XYZ{{{.6624541811085053, .13400420645643313, .1561876870049078},
                        {.27222871678091454, .6740817658111483, .05368951740793705},
                        {-.005574649490394108, .004060733528982826, 1.0103391003129971}}};
inline const M BRADFORD{{{.8951, .2664, -.1614}, {-.7502, 1.7135, .0367}, {.0389, -.0685, 1.0296}}};

inline M adaptation(V from, V to) {
    V f = mul(BRADFORD, from), t = mul(BRADFORD, to);
    M d{};
    for (int i = 0; i < 3; ++i)
        d[i][i] = t[i] / f[i];
    return mul(mul(inverse(BRADFORD), d), BRADFORD);
}

inline double encode(double x) {
    return x <= .0078125 ? 10.5402377416545 * x + .0729055341958355 : (std::log2(x) + 9.72) / 17.52;
}

inline double decode(double x) {
    return x <= .155251141552511 ? (x - .0729055341958355) / 10.5402377416545
                                 : std::exp2(17.52 * x - 9.72);
}

inline double maxabs(V a) {
    for (double value : a)
        if (!std::isfinite(value))
            throw std::runtime_error("nonfinite vector in numerical admission");
    return std::max({std::abs(a[0]), std::abs(a[1]), std::abs(a[2])});
}

inline bool inCube(V v, double tolerance = 0) {
    return std::all_of(v.begin(), v.end(), [tolerance](double value) {
        return std::isfinite(value) && value >= -tolerance && value <= 1 + tolerance;
    });
}

// Only the gray operating point is used; no scalar output encoding is applied.
// Equations/preset follow OCIO 2.6.0 ACES2/Transform.cpp, BSD-3-Clause,
// Copyright Contributors to the OpenColorIO Project. Notice retained in
// assets/ocio/LICENSE-OpenColorIO.txt.
inline double paperGray() {
    constexpr double g = 1.15, toe = .04, hit = 128., gray = .18, displayGray = 10.013 / 100.;
    const double m1 = .5 * (1 + std::sqrt(1 + 4 * toe));
    const double u = std::pow((hit / m1) / (hit / m1 + 1), g), m = m1 / u;
    const double gip = .5 * (displayGray + std::sqrt(displayGray * (displayGray + 4 * toe)));
    const double power = std::pow(gip / m, 1 / g), gipp2 = -(m1 * power) / (power - 1),
                 w2 = gray / gipp2;
    const double sRelative = w2 * m1, u2 = std::pow((hit / m1) / (hit / m1 + w2), g), m2 = m1 / u2;
    const double f = m2 * std::pow(gray / (gray + sRelative), g);
    return f * f / (f + toe);
}
} // namespace looks
