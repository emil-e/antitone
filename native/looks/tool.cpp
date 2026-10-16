#include "bake.h"
#include <iostream>
#include <limits>
#include <random>
using namespace looks;

struct Core {
    double rawGray = 0, gray = 0, offsets = 0, mapping = 0, roundtrip = 0, monoEqualY = 0;
    Core(const Reference &look) {
        Reference unit(1.);
        Domain domain;
        // NaN comparisons must not turn a failed residual into zero error.
        for (double invalid :
             {std::numeric_limits<double>::quiet_NaN(), std::numeric_limits<double>::infinity()}) {
            for (int component = 0; component < 3; ++component) {
                V value{};
                value[component] = invalid;
                bool rejected = false;
                try {
                    maxabs(value);
                } catch (const std::runtime_error &) {
                    rejected = true;
                }
                if (!rejected || inCube(value))
                    throw std::runtime_error("nonfinite residual/range admission failed open");
            }
        }
        V mid{.18, .18, .18};
        rawGray = maxabs(look.raw(mid) - V{paperGray(), paperGray(), paperGray()});
        gray = maxabs(look(mid) - mid);
        offsets = maxabs(look.source.exposure0 - unit.source.exposure0);
        std::mt19937 random(119);
        std::uniform_real_distribution<double> log(domain.lo, domain.hi);
        for (int i = 0; i < 10000; ++i) {
            V input{decode(log(random)), decode(log(random)), decode(log(random))},
                remapped = input;
#ifdef LOOK_NEUTRAL_DENSITY
            double y = dot(AP1_XYZ[1], input),
                   yy = decode(encode(.18) + .6 * (encode(y) - encode(.18)));
            remapped = V{yy, yy, yy};
            monoEqualY = std::max(monoEqualY, maxabs(look.raw(input) - look.raw(V{y, y, y})));
#else
            for (double &c : remapped)
                c = decode(encode(.18) + .6 * (encode(c) - encode(.18)));
#endif
            V output = look(input);
            mapping = std::max(mapping, maxabs(output - unit(remapped)));
            roundtrip = std::max(roundtrip,
                                 maxabs(look.map(look.raw(input)) - ocio(look.forwardDRT, output)));
            for (double c : output)
                if (!std::isfinite(c) || c < 0 || c > 1024)
                    throw std::runtime_error("synthetic scene outside native range");
        }
        for (double c :
             {decode(domain.lo - 1e-6), decode(domain.hi + 1e-6),
              std::numeric_limits<double>::quiet_NaN(), std::numeric_limits<double>::infinity()}) {
            bool rejected = false;
            try {
                domain.input(V{c, .18, .18});
            } catch (const std::runtime_error &) {
                rejected = true;
            }
            if (!rejected)
                throw std::runtime_error("invalid domain input accepted");
        }
        for (double c : {0., -.6, 1.1, std::numeric_limits<double>::quiet_NaN(),
                         std::numeric_limits<double>::infinity()}) {
            bool rejected = false;
            try {
                Material invalid(c);
            } catch (const std::runtime_error &) {
                rejected = true;
            }
            if (!rejected)
                throw std::runtime_error("invalid compression accepted");
        }
#ifdef LOOK_NEUTRAL_DENSITY
        V witness{.7, .05, .01}, wrong = witness;
        for (double &c : wrong)
            c = decode(encode(.18) + .6 * (encode(c) - encode(.18)));
        if (maxabs(look.raw(witness) - unit.raw(wrong)) < 1e-3)
            throw std::runtime_error(
                "monochrome ordering witness did not distinguish RGB-first mapping");
#endif
        if (rawGray > 1e-9 || gray > 2e-5 || offsets > 1e-12 || mapping > 1e-10 ||
            roundtrip > 2e-4 || monoEqualY > 1e-12)
            throw std::runtime_error("core admission failed");
    }
    void json(std::ostream &out) const {
        out << "\"raw_gray_error\":" << rawGray << ",\"gray_ap1_error\":" << gray
            << ",\"compression_offset_change\":" << offsets
            << ",\"explicit_remap_error\":" << mapping
            << ",\"synthetic_reference_roundtrip_max\":" << roundtrip
            << ",\"mono_equal_y_error\":" << monoEqualY;
    }
};
int main(int argc, char **argv) {
    try {
        if (std::string(OCIO::GetVersion()) != "2.6.0")
            throw std::runtime_error("requires pinned OCIO 2.6.0");
        if (argc < 2)
            throw std::runtime_error("usage: look-tool check | bake SIZE EMPTY_OUTPUT_DIR");
        std::string command = argv[1];
        if ((command == "check" && argc != 2) || (command == "bake" && argc != 4) ||
            (command != "check" && command != "bake"))
            throw std::runtime_error("invalid command/arguments");
        Reference look;
        Core core(look);
        std::cout << std::setprecision(12);
        if (command == "check") {
            std::cout << "{\"status\":\"passed\",\"look\":" << std::quoted(look_id) << ',';
            core.json(std::cout);
            std::cout << "}\n";
            return 0;
        }
        size_t used = 0;
        std::string text = argv[2];
        int size = std::stoi(text, &used);
        if (used != text.size() || (size != 65 && size != 97))
            throw std::runtime_error("supported sizes are 65 and 97");
        fs::path output = argv[3];
        // The Python owner atomically claims the destination. This internal
        // entry point only writes into the empty directory it was handed.
        if (!fs::is_directory(output) || !fs::is_empty(output))
            throw std::runtime_error("caller must provide an empty owned output directory");
        Cube cube(look, size);
        cube.write(output / "look.cube");
        auto file = load(output / "look.cube");
        double fileLocal = 0,
               bakedGray = maxabs(fileApply(file, cube, V{.18, .18, .18}) - V{.18, .18, .18});
        double referenceFileMax = 0;
        std::mt19937 rng(7108);
        std::uniform_real_distribution<double> log(cube.lo, cube.hi);
        for (int i = 0; i < 10000; ++i) {
            V input{decode(log(rng)), decode(log(rng)), decode(log(rng))};
            V actual = fileApply(file, cube, input);
            fileLocal = std::max(fileLocal, maxabs(actual - cube.apply(input)));
            referenceFileMax = std::max(referenceFileMax, maxabs(ocio(look.forwardDRT, actual) -
                                                                 look.map(look.raw(input))));
        }
        if (bakedGray > 2e-5 || fileLocal > 2e-4)
            throw std::runtime_error("actual-file gray/interpolation admission failed");
        std::ofstream report(output / "native-checks.json");
        report << std::setprecision(12)
               << "{\"status\":\"passed\",\"look\":" << std::quoted(look_id)
               << ",\"ocio\":\"2.6.0\",\"cube_size\":" << size << ",\"nodes\":" << cube.nodes.size()
               << ',';
        core.json(report);
        report << ",\"all_node_reference_roundtrip_max\":" << cube.roundtripMax
               << ",\"all_node_reference_failures\":" << cube.roundtripFailures
               << ",\"out_of_native_range_components\":" << cube.outsideRange
               << ",\"baked_gray_error\":" << bakedGray
               << ",\"file_local_tetra_error\":" << fileLocal
               << ",\"sampled_reference_lut_approximation_max\":" << referenceFileMax << "}\n";
        report.close();
        if (!report)
            throw std::runtime_error("cannot write check report");
        std::cout << "{\"status\":\"passed\",\"look\":" << std::quoted(look_id)
                  << ",\"nodes\":" << cube.nodes.size() << "}\n";
    } catch (const std::exception &e) {
        std::cerr << e.what() << '\n';
        return 1;
    }
}
