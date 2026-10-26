@group(0) @binding(1)
var<storage, read_write> positives: array<vec4f>;

fn pcg(value: u32) -> u32 {
    let state = value * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn grain_hash(ix: i32, iy: i32, seed: u32) -> f32 {
    let h = pcg(bitcast<u32>(ix) ^ pcg(bitcast<u32>(iy) ^ seed));
    // Near-Gaussian, variance 1/12, with finite support for the HDR gain bound.
    return (f32(countOneBits(h)) - 16.0) * 0.10206207261596575;
}

// Gaussian covariance factors fitted to source-frame-normalized scan spectra.
// Independent latent RGB fields are mixed into one continuous cross-spectrum.
const GRAIN_SIGMAS = array<f32, 2>(0.24, 0.68);
const GRAIN_FACTORS = array<mat3x3f, 2>(
        mat3x3f(
            vec3f(0.3730124895, 0.6619305266, 1.5330216192),
            vec3f(0.5050531591, 0.4772988068, -0.3289776090),
            vec3f(0.2495790334, -0.2357789247, 0.0410778103),
        ),
        mat3x3f(
            vec3f(-0.5272974482, -0.5838512039, 0.6499508815),
            vec3f(0.4116058302, 0.0504305084, 0.3792326835),
            vec3f(0.2041265846, -0.3754140180, -0.1716291221),
        ),
    );

fn grain_kernel_energy(phase: vec2f, variance: f32, radius: i32) -> f32 {
    var energy = vec2f(0.0);
    for(var i = -radius; i <= radius; i++) {
        let d = phase - vec2f(f32(i));
        energy += exp(-d * d / variance);
    }
    return energy.x * energy.y;
}

fn grain_field(source: vec2f, scale: f32, view_footprint: f32, seed: u32) -> vec3f {
    // Original-pixel integration remains authoritative when magnifying.
    let footprint = max(view_footprint, 1.0);
    var reference = 0.0;
    var peak_variance = 0.0;
    for(var i = 0u; i < 2u; i++) {
        let sigma = GRAIN_SIGMAS[i] * scale;
        let factor = GRAIN_FACTORS[i];
        let diagonal = factor[0] * factor[0] + factor[1] * factor[1] + factor[2] * factor[2];
        let power = sigma * sigma / (sigma * sigma + footprint * footprint / 12.0);
        reference += diagonal.y * sigma * sigma / (sigma * sigma + 1.0 / 12.0);
        peak_variance += max(max(diagonal.x, diagonal.y), diagonal.z) * power;
    }
    // Maximum Amount/Colour loses at most 6e-5 relative RMS below this budget.
    if peak_variance < 1e-8 * reference {
        return vec3f(0.0);
    }
    var value = vec3f(0.0);
    for(var i = 0u; i < 2u; i++) {
        let sigma = GRAIN_SIGMAS[i] * scale;
        // Energy normalization removes phase-dependent variance. This density keeps
        // residual covariance modulation below 0.3% without oversampling the source lattice.
        let spacing = 1.1 * sigma;
        let p = source / spacing;
        let base = vec2i(floor(p));
        let variance = 1.0 / (1.1 * 1.1) + footprint * footprint / (12.0 * spacing * spacing);
        let radius = i32(ceil(3.5 * sqrt(variance) + 1.0));
        let energy = grain_kernel_energy(p - vec2f(base), variance, radius);
        let stream = vec3u(
                pcg(seed ^ (3u * i + 1u) * 0x9e3779b9u),
                pcg(seed ^ (3u * i + 2u) * 0x9e3779b9u),
                pcg(seed ^ (3u * i + 3u) * 0x9e3779b9u),
            );
        var sum = vec3f(0.0);
        for(var y = -radius; y <= radius; y++) {
            for(var x = -radius; x <= radius; x++) {
                let d = p - vec2f(f32(base.x + x), f32(base.y + y));
                let weight = exp(-0.5 * dot(d, d) / variance);
                let noise = vec3f(
                        grain_hash(base.x + x, base.y + y, stream.x),
                        grain_hash(base.x + x, base.y + y, stream.y),
                        grain_hash(base.x + x, base.y + y, stream.z),
                    );
                sum += weight * noise;
            }
        }
        let power = sigma * sigma / (sigma * sigma + footprint * footprint / 12.0);
        value += sqrt(power) * (GRAIN_FACTORS[i] * sum) / sqrt(energy / 12.0);
    }
    return value / sqrt(reference);
}

// Native originals carry camera-floor flags; physical views additionally carry coverage flag 0x8.
fn film_covered(pixel: vec4f) -> bool {
    return view.dimensions.w != 0u || (u32(pixel.w) & 8u) != 0u;
}

// Image MTF remains sigma=L; grain's normalized spectral tails are checked against this
// fixed point response independently of the PSD basis used to evaluate grain.
fn source_mtf(index: u32, axis: vec2i, sigma: f32) -> vec4f {
    let dimensions = vec2i(view.dimensions.xy);
    let xy = vec2i(i32(index) % dimensions.x, i32(index) / dimensions.x);
    let radius = i32(ceil(4.0 * sigma));
    var mean = vec3f(0.0);
    var weight_sum = 0.0;
    for(var i = -radius; i <= radius; i++) {
        let p = clamp(xy + i * axis, vec2i(0), dimensions - vec2i(1));
        let sample = working_source[p.x + p.y * dimensions.x];
        if !film_covered(sample) {
            continue;
        }
        let weight = exp(-0.5 * f32(i * i) / max(sigma * sigma, 1e-12));
        if weight == 0.0 {
            continue;
        }
        let next_weight_sum = weight_sum + weight;
        let fraction = weight / next_weight_sum;
        // Convex accumulation keeps finite signed/HDR constants finite even for large kernels.
        mean = mean * (1.0 - fraction) + sample.rgb * fraction;
        weight_sum = next_weight_sum;
    }
    if weight_sum <= 0.0 {
        return working_source[index];
    }
    return vec4f(mean, working_source[index].w);
}

@group(0) @binding(12)
var<storage, read_write> grain_pixels: array<vec4f>;

@compute @workgroup_size(64)
fn film_field(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let index = film_index(id, groups);
    if index >= arrayLength(&grain_pixels) {
        return;
    }
    let footprint = max(length(view.axis_x.xy), length(view.axis_y.xy));
    grain_pixels[index] =
        vec4f(grain_field(view_point(index), view.film.z, footprint, view.dimensions.z), 0.0);
}

fn film_index(id: vec3u, groups: vec3u) -> u32 {
    return id.x + id.y * groups.x * 64u;
}

@compute @workgroup_size(64)
fn film_horizontal(
    @builtin(global_invocation_id) id: vec3u,
    @builtin(num_workgroups) groups: vec3u,
) {
    let index = film_index(id, groups);
    if index >= arrayLength(&working_source) {
        return;
    }
    let sigma = view.film.z / max(length(view.axis_x.xy), 1e-6);
    positives[index] = source_mtf(index, vec2i(1, 0), sigma);
}

@compute @workgroup_size(64)
fn film_vertical(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let index = film_index(id, groups);
    if index >= arrayLength(&working_source) {
        return;
    }
    let sigma = view.film.z / max(length(view.axis_y.xy), 1e-6);
    positives[index] = source_mtf(index, vec2i(0, 1), sigma);
}

@compute @workgroup_size(64)
fn film_grain(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let index = film_index(id, groups);
    if index >= arrayLength(&positives) {
        return;
    }
    var result = positives[index];
    if film_covered(result) && view.film.y > 0.0 {
        let field = grain_pixels[index].rgb;
        let grain = mix(vec3f(field.y), field, 2.0 * view.film_colour.x);
        let amount = view.film.y;
        result = vec4f(
                result.rgb * exp(amount * grain - 0.5 * amount * amount * view.film_colour.yzw),
                result.w,
            );
    }
    positives[index] = result;
}
