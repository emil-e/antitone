const GLOBAL_MASK: u32 = 0u;
const LINEAR_MASK: u32 = 1u;

fn adjustment_layer_projection32(delta: vec2f, axis: vec2f) -> f32 {
    return delta.x * axis.x + delta.y * axis.y;
}

fn adjustment_layer_smoothstep32(t: f32) -> f32 {
    if t <= 0.0 {
        return 0.0;
    }
    if 1.0 <= t {
        return 1.0;
    }
    let near_start = t <= 0.5;
    let u = select(1.0 - t, t, near_start);
    let value = u * u * (3.0 - 2.0 * u);
    return select(1.0 - value, value, near_start);
}

fn adjustment_layer_coverage_point32(layer: AdjustmentLayerParameters, source: vec2f) -> f32 {
    if layer.mask_kind == GLOBAL_MASK {
        return 1.0;
    }
    let delta = source - layer.mask_origin;
    if layer.mask_kind == LINEAR_MASK {
        let normalized = delta / layer.mask_radii.x;
        return adjustment_layer_smoothstep32(
                adjustment_layer_projection32(normalized, layer.mask_x_axis),
            );
    }
    let x = adjustment_layer_projection32(delta, layer.mask_x_axis) / layer.mask_radii.x;
    let y = adjustment_layer_projection32(delta, layer.mask_y_axis) / layer.mask_radii.y;
    if 1.0 <= x || x <= -1.0 || 1.0 <= y || y <= -1.0 {
        return 0.0;
    }
    let radius = sqrt(x * x + y * y);
    if 1.0 <= radius {
        return 0.0;
    }
    let feather = layer.mask_feather;
    if radius <= 1.0 - feather {
        return 1.0;
    }
    return adjustment_layer_smoothstep32((1.0 - radius) / feather);
}

fn adjustment_layer_weight32(layer: AdjustmentLayerParameters, source: vec2f) -> f32 {
    var coverage = adjustment_layer_coverage_point32(layer, source);
    if layer.invert_mask != 0u {
        coverage = 1.0 - coverage;
    }
    return coverage * layer.strength;
}
