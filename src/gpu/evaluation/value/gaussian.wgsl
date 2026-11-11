struct Parameters {
    bounds: vec4i,
    axis: vec4i,
    shape: vec4f,
}

fn pixel(p: vec2i, position: vec2f, parameters: Parameters) -> vec4f {
    let center = input_0(p);
    var mean = vec3f(0.0);
    var reference = vec3f(0.0);
    var centered = true;
    var weight_sum = 0.0;
    let radius = parameters.axis.z;
    let sigma = parameters.shape.x;
    for(var i = -radius; i <= radius; i++) {
        var q = p + i * parameters.axis.xy;
        if parameters.shape.y != 0.0 {
            q = clamp(q, parameters.bounds.xy, parameters.bounds.zw - vec2i(1));
        }
        let sample = input_0(q);
        if parameters.axis.w != 0 && (u32(sample.w) & u32(parameters.axis.w)) == 0u {
            continue;
        }
        let weight = exp(-0.5 * f32(i * i) / max(sigma * sigma, 1e-12));
        if weight == 0.0 {
            continue;
        }
        let next_weight_sum = weight_sum + weight;
        let fraction = weight / next_weight_sum;
        if weight_sum == 0.0 {
            reference = sample.rgb;
        } else {
            let difference = sample.rgb - reference;
            if centered && all(abs(difference) <= vec3f(3.4028234663852886e38)) {
                // Centering preserves constants and small structure on a nonzero plate.
                mean = mean * (1.0 - fraction) + difference * fraction;
            } else {
                if centered {
                    mean += reference;
                    centered = false;
                }
                // Opposite extreme signs need an absolute convex mean instead.
                mean = mean * (1.0 - fraction) + sample.rgb * fraction;
            }
        }
        weight_sum = next_weight_sum;
    }
    if weight_sum <= 0.0 {
        return center;
    }
    if centered {
        mean += reference;
    }
    return vec4f(mean, center.w);
}
