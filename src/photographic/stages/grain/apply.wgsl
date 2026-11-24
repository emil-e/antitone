fn apply_grain(pixel: vec4f, field: vec3f, parameters: Grain) -> vec4f {
    var result = pixel;
    if raster_covered(parameters.raster, result) && parameters.values.y > 0.0 {
        let grain = mix(vec3f(field.y), field, 2.0 * parameters.values.z);
        let amount = parameters.values.y;
        let gain = exp(amount * grain - 0.5 * amount * amount * parameters.variance.xyz);
        for(var c = 0u; c < 3u; c++) {
            result[c] = photographic_finite(result[c] * gain[c]);
        }
    }
    return result;
}
