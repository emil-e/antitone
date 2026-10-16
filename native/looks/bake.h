#pragma once
#include "reference.h"
#include <filesystem>
#include <fstream>
#include <iomanip>
#include <vector>

namespace looks {
namespace fs = std::filesystem;

struct Domain {
    double lo = -.5, hi = lo + (encode(.18) - lo) * (64. / 38.);

    V input(V linear) const {
        for (double &v : linear) {
            v = encode(v);
            if (!(v >= lo && v <= hi))
                throw std::runtime_error("input outside LUT domain");
        }
        return linear;
    }
};

struct Cube : Domain {
    int size;
    std::vector<V> nodes;
    size_t outsideRange = 0, roundtripFailures = 0;
    double roundtripMax = 0;

    explicit Cube(const Reference &look, int n) : size(n) {
        if (n != 65 && n != 97)
            throw std::runtime_error("supported cube sizes are 65 and 97");
        nodes.reserve(size_t(n) * n * n);
        for (int b = 0; b < n; ++b)
            for (int g = 0; g < n; ++g)
                for (int r = 0; r < n; ++r) {
                    V input{decode(lo + (hi - lo) * r / (n - 1)),
                            decode(lo + (hi - lo) * g / (n - 1)),
                            decode(lo + (hi - lo) * b / (n - 1))};
                    V mapped = look.map(look.raw(input)), out = ocio(look.inverseDRT, mapped);
                    double error = maxabs(mapped - ocio(look.forwardDRT, out));
                    roundtripMax = std::max(roundtripMax, error);
                    roundtripFailures += error > 2e-4;
                    for (double &c : out) {
                        outsideRange += c < 0 || c > 1024;
                        c = encode(c);
                        if (!std::isfinite(c))
                            throw std::runtime_error("nonfinite LUT node");
                    }
                    nodes.push_back(out);
                }
        if (roundtripFailures || outsideRange)
            throw std::runtime_error("all-node reference/range admission failed");
    }

    size_t index(std::array<int, 3> q) const {
        return (q[2] * size + q[1]) * size + q[0];
    }

    V apply(V linear) const {
        V log = input(linear), fraction{};
        std::array<int, 3> q{};
        for (int c = 0; c < 3; ++c) {
            double at = (log[c] - lo) / (hi - lo) * (size - 1);
            q[c] = std::min(size - 2, int(at));
            fraction[c] = at - q[c];
        }
        std::array<int, 3> order{0, 1, 2};
        std::sort(order.begin(), order.end(),
                  [&](int a, int b) { return fraction[a] > fraction[b]; });
        double a = fraction[order[0]], b = fraction[order[1]], c = fraction[order[2]];
        V out = nodes[index(q)] * (1 - a);
        ++q[order[0]];
        out = out + nodes[index(q)] * (a - b);
        ++q[order[1]];
        out = out + nodes[index(q)] * (b - c);
        ++q[order[2]];
        out = out + nodes[index(q)] * c;
        for (double &v : out)
            v = decode(v);
        return out;
    }

    void write(const fs::path &path) const {
        std::ofstream f(path);
        if (!f)
            throw std::runtime_error("cannot create LUT");
        f << std::setprecision(12);
        f << "# Antitone independent creative look: " << look_id
          << " / revision 1\n# ACEScct/AP1/D60 input and output; apply BEFORE ordinary "
             "selected-output ACES\n# Tetrahedral interpolation; explicit domain admission "
             "required; no fallback\n# Content selection is not release or physical-calibration "
             "clearance\nTITLE "
          << std::quoted(look_name) << "\nLUT_3D_SIZE " << size << "\nDOMAIN_MIN " << lo << ' '
          << lo << ' ' << lo << "\nDOMAIN_MAX " << hi << ' ' << hi << ' ' << hi << '\n';
        for (V v : nodes)
            f << v[0] << ' ' << v[1] << ' ' << v[2] << '\n';
        f.close();
        if (!f)
            throw std::runtime_error("LUT write failed");
    }
};

inline OCIO::ConstCPUProcessorRcPtr load(const fs::path &path) {
    auto file = OCIO::FileTransform::Create();
    std::string name = path.string();
    file->setSrc(name.c_str());
    file->setInterpolation(OCIO::INTERP_TETRAHEDRAL);
    return OCIO::Config::CreateRaw()->getProcessor(file)->getDefaultCPUProcessor();
}

inline V fileApply(OCIO::ConstCPUProcessorRcPtr file, const Domain &domain, V input) {
    V out = ocio(file, domain.input(input));
    for (double &v : out)
        v = decode(v);
    return out;
}
} // namespace looks
