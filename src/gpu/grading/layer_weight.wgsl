fn evaluation_layer_weight(pixel: vec4f, position: vec2f) -> vec4f {
    let weight = adjustment_layer_weight32(0u, position);
    return vec4f(vec3f(weight), pixel.w);
}
