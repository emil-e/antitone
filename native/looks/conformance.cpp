#include "bake.h"
#include <iostream>

// Developer-only independent material/reference samples, before the inverse transform.
// Native-endian doubles are an intermediate consumed locally by generate_conformance.py.
int main() {
    try {
        if (std::string(looks::OCIO::GetVersion()) != "2.6.0")
            throw std::runtime_error("OCIO runtime version must be 2.6.0");
        const looks::Reference reference;
        const looks::Domain domain;
        for (int b = 0; b < 65; ++b)
            for (int g = 0; g < 65; ++g)
                for (int r = 0; r < 65; ++r) {
                    const looks::V input{
                        looks::decode(domain.lo + (domain.hi - domain.lo) * r / 64.),
                        looks::decode(domain.lo + (domain.hi - domain.lo) * g / 64.),
                        looks::decode(domain.lo + (domain.hi - domain.lo) * b / 64.)};
                    const looks::V mapped = reference.map(reference.raw(input));
                    for (double value : mapped)
                        std::cout.write(reinterpret_cast<const char *>(&value), sizeof(value));
                }
        std::cout.flush();
        if (!std::cout)
            throw std::runtime_error("native conformance write failed");
        return 0;
    } catch (const std::exception &error) {
        std::cerr << error.what() << '\n';
        return 1;
    }
}
