// Originals, repaired positives and filtered views share Roll balance and exposure.
// Neutral working-view acquisition omits exposure; its caller restores that gain here.
fn photographic_post(input: array<f32, 3>, gain: f32) -> array<f32, 3> {
    var scaled: array<f32, 3>;
    for(var c = 0u; c < 3u; c++) {
        scaled[c] = photographic_finite(input[c] * gain);
    }
    return matrix_product(parameters.post, scaled);
}

fn colour_balance(
    layer: AdjustmentLayerParameters,
    rgb: array<f32, 3>,
    weight: f32,
) -> array<f32, 3> {
    let exposure = exp(weight * layer.log_exposure_gain);
    var exposed: array<f32, 3>;
    for(var c = 0u; c < 3u; c++) {
        exposed[c] = photographic_finite(rgb[c] * exposure);
    }
    var cones = matrix_product(adjustment_layer_context.ap1_to_bradford.columns, exposed);
    for(var c = 0u; c < 3u; c++) {
        cones[c] = photographic_finite(cones[c] * exp(weight * layer.wb_log_cone_offsets[c]));
    }
    return matrix_product(adjustment_layer_context.bradford_to_ap1.columns, cones);
}

fn colour_adjustment_layers(input: array<f32, 3>, source: vec2f) -> array<f32, 3> {
    let count = adjustment_layer_context.layer_count;
    if count == 0u {
        return input;
    }
    let origin = adjustment_layer_context.source_origin;
    let end = origin + adjustment_layer_context.source_extent;
    let inside = all(vec2f(origin) <= source) && all(source < vec2f(end));
    var rgb = input;
    for(var stage = 0u; stage < count; stage++) {
        let layer = adjustment_layer_stages[stage];
        if layer.mask_kind != GLOBAL_MASK && !inside {
            continue;
        }
        let weight = adjustment_layer_weight32(layer, source);
        if weight == 0.0 {
            continue;
        }
        if layer.has_wb != 0u {
            rgb = colour_balance(layer, rgb, weight);
        } else if layer.log_exposure_gain != 0.0 {
            let gain = exp(weight * layer.log_exposure_gain);
            for(var c = 0u; c < 3u; c++) {
                rgb[c] = photographic_finite(rgb[c] * gain);
            }
        }
        let da = weight * (layer.contrast - 1.0);
        let ds = weight * (layer.saturation - 1.0);
        let vibrance = weight * layer.vibrance;
        let tonal_ev = weight * tonal_exposure(rgb, layer.shadows_ev, layer.highlights_ev);
        if tonal_ev == 0.0 && da == 0.0 && ds == 0.0 && vibrance == 0.0 {
            continue;
        }
        rgb = colour_apply(rgb, tonal_ev, da, layer.encoded_pivot, layer.preserve, ds, vibrance);
    }
    return rgb;
}
