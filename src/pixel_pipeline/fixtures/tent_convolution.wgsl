struct Parameters {
    bounds: vec4i,
    values: vec4u,
}

fn convolve(point: vec2i, position: vec2f, parameters: Parameters) -> vec4f {
    let center = input_0(point);
    if center.w < 0.0 || (parameters.values.z != 0u && (u32(center.w) & 8u) == 0u) {
        return center;
    }
    let axis = select(vec2i(1, 0), vec2i(0, 1), parameters.values.y != 0u);
    var mean = vec3f(0.0);
    var total = 0.0;
    for(var i = -1; i <= 1; i++) {
        let at = clamp(
                point + i * i32(parameters.values.x) * axis,
                parameters.bounds.xy,
                parameters.bounds.zw - vec2i(1),
            );
        let sample = input_0(at);
        if sample.w < 0.0 {
            return sample;
        }
        if parameters.values.z != 0u && (u32(sample.w) & 8u) == 0u {
            continue;
        }
        let weight = select(0.25, 0.5, i == 0);
        let next = total + weight;
        let fraction = weight / next;
        mean = mean * (1.0 - fraction) + sample.rgb * fraction;
        total = next;
    }
    return vec4f(mean, center.w);
}
