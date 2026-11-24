struct Parameters {
    rectangle: vec4u,
}
fn pixel(
    p: vec2i,
    position: vec2f,
    previous: array<vec4f, 2>,
    parameters: Parameters,
) -> array<vec4f, 2> {
    let local = p - vec2i(parameters.rectangle.xy);
    if any(local < vec2i(0)) || any(vec2u(local) >= parameters.rectangle.zw) {
        return previous;
    }
    let contribution = input_1(p);
    let value = contribution[0];
    if value.w <= 0.0 {
        return previous;
    }
    let flags = merged_flags(previous[1].x, contribution[1].x);
    if previous[0].w == 0.0 {
        return array<vec4f, 2>(value, vec4f(flags, 0.0, 0.0, 0.0));
    }
    let mass = previous[0].w + value.w;
    let mean = weighted_mean(previous[0].rgb, value.rgb, value.w / mass);
    return array<vec4f, 2>(vec4f(mean, mass), vec4f(flags, 0.0, 0.0, 0.0));
}
