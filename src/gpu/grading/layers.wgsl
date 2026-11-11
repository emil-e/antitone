fn colour_adjustment_layer_delta(stage: u32, row: u32, column: u32, base: f32) -> f32 {
    return adjustment_layer_value(stage, row, column) - base;
}

fn colour_adjustment_layer(stage: u32, row: u32, column: u32) -> f32 {
    return adjustment_layer_value(stage, row, column);
}

// Originals, repaired positives and filtered views share Roll balance and exposure.
// Neutral working-view acquisition omits exposure; its caller restores that gain here.
fn photographic_post(input: array<f32, 3>, gain: f32) -> array<f32, 3> {
    var scaled: array<f32, 3>;
    for(var c = 0u; c < 3u; c++) {
        scaled[c] = photographic_finite(input[c] * gain);
    }
    return matrix_product(parameters.post, scaled);
}

fn colour_balance(stage: u32, rgb: array<f32, 3>, weight: f32) -> array<f32, 3> {
    let exposure = exp(weight * colour_adjustment_layer(stage, 2u, 0u));
    var exposed: array<f32, 3>;
    for(var c = 0u; c < 3u; c++) {
        exposed[c] = photographic_finite(rgb[c] * exposure);
    }
    var cones = matrix_product(adjustment_layer_context.cones.columns, exposed);
    for(var c = 0u; c < 3u; c++) {
        cones[c] = photographic_finite(
                cones[c] * exp(weight * colour_adjustment_layer(stage, 2u, c + 1u)),
            );
    }
    return matrix_product(adjustment_layer_context.inverse.columns, cones);
}

fn colour_adjustment_layers(input: array<f32, 3>, source: vec2f) -> array<f32, 3> {
    if adjustment_layer_context.count.x == 0u {
        return input;
    }
    let area = adjustment_layer_context.area;
    let inside =
        f32(area.x) <= source.x && f32(area.y) <= source.y && source.x < f32(area.x + area.z)
        && source.y < f32(area.y + area.w);
    var rgb = input;
    for(var stage = 0u; stage < adjustment_layer_context.count.x; stage++) {
        if adjustment_layer_stages[stage].flags.x != 0u && !inside {
            continue;
        }
        let weight = adjustment_layer_weight32(stage, source);
        if weight == 0.0 {
            continue;
        }
        let ev = colour_adjustment_layer(stage, 2u, 0u);
        if adjustment_layer_stages[stage].flags.z != 0u {
            rgb = colour_balance(stage, rgb, weight);
        } else if ev != 0.0 {
            let gain = exp(weight * ev);
            for(var c = 0u; c < 3u; c++) {
                rgb[c] = photographic_finite(rgb[c] * gain);
            }
        }
        let da = weight * colour_adjustment_layer_delta(stage, 3u, 0u, 1.0);
        let ds = weight * colour_adjustment_layer_delta(stage, 3u, 3u, 1.0);
        if da == 0.0 && ds == 0.0 {
            continue;
        }
        rgb = colour_apply(
                rgb,
                da,
                colour_adjustment_layer_delta(stage, 3u, 1u, COLOUR_OFFSET),
                colour_adjustment_layer(stage, 3u, 2u),
                ds,
                colour_adjustment_layer(stage, 4u, 0u),
            );
    }
    return rgb;
}
