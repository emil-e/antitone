fn matrix_product(columns: array<vec4f, 3>, rgb: array<f32, 3>) -> array<f32, 3> {
    var output: array<f32, 3>;
    for(var c = 0u; c < 3u; c++) {
        var value = 0.0;
        for(var j = 0u; j < 3u; j++) {
            value = fma(columns[j][c], rgb[j], value);
        }
        if (bitcast<u32>(value) & 0x7fffffffu) >= 0x7f800000u {
            // Retry only overflowing products on normalized operands, preserving cancellation
            // before restoring their magnitude. Ordinary photographic arithmetic is unchanged.
            // Keep reciprocal scales normal on GPUs that flush subnormal division results.
            let input_scale = min(
                    PHOTOGRAPHIC_LIMIT,
                    max(abs(rgb[0]), max(abs(rgb[1]), abs(rgb[2]))),
                );
            let matrix_scale = min(
                    PHOTOGRAPHIC_LIMIT,
                    max(abs(columns[0][c]), max(abs(columns[1][c]), abs(columns[2][c]))),
                );
            value = 0.0;
            if input_scale != 0.0 && matrix_scale != 0.0 {
                for(var j = 0u; j < 3u; j++) {
                    value = fma(columns[j][c] / matrix_scale, rgb[j] / input_scale, value);
                }
                if value != 0.0 {
                    value = sign(value)
                        * photographic_exp(log(abs(value)) + log(input_scale) + log(matrix_scale));
                }
            }
        }
        output[c] = photographic_finite(value);
    }
    return output;
}

fn camera_positive_to_ap1(rgb: array<f32, 3>) -> array<f32, 3> {
    return matrix_product(camera_matrix.columns, rgb);
}
