struct AdjustmentLayerParameters {
    values: array<vec4f, 5>,
    flags: vec4u,
}

struct AdjustmentLayerContext {
    cones: CameraMatrix,
    inverse: CameraMatrix,
    area: vec4u,
    count: vec4u,
}
@group(0) @binding(6)
var<storage, read> adjustment_layer_stages: array<AdjustmentLayerParameters>;
@group(0) @binding(7)
var<uniform> adjustment_layer_context: AdjustmentLayerContext;

fn adjustment_layer_value(stage: u32, row: u32, column: u32) -> f32 {
    return adjustment_layer_stages[stage].values[row][column];
}

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

fn adjustment_layer_coverage_point32(stage: u32, source: vec2f) -> f32 {
    if adjustment_layer_stages[stage].flags.x == 0u {
        return 1.0;
    }
    let delta = source
        - vec2f(adjustment_layer_value(stage, 0u, 0u), adjustment_layer_value(stage, 0u, 1u));
    let x_axis = vec2f(
            adjustment_layer_value(stage, 0u, 2u),
            adjustment_layer_value(stage, 0u, 3u),
        );
    let radius_x = adjustment_layer_value(stage, 4u, 1u);
    if adjustment_layer_stages[stage].flags.x == 1u {
        let normalized = delta / radius_x;
        return adjustment_layer_smoothstep32(adjustment_layer_projection32(normalized, x_axis));
    }
    let y_axis = vec2f(
            adjustment_layer_value(stage, 1u, 0u),
            adjustment_layer_value(stage, 1u, 1u),
        );
    let x = adjustment_layer_projection32(delta, x_axis) / radius_x;
    let y = adjustment_layer_projection32(delta, y_axis) / adjustment_layer_value(stage, 4u, 2u);
    if 1.0 <= x || x <= -1.0 || 1.0 <= y || y <= -1.0 {
        return 0.0;
    }
    let radius = sqrt(x * x + y * y);
    if 1.0 <= radius {
        return 0.0;
    }
    let feather = adjustment_layer_value(stage, 1u, 2u);
    if radius <= 1.0 - feather {
        return 1.0;
    }
    return adjustment_layer_smoothstep32((1.0 - radius) / feather);
}

fn adjustment_layer_weight32(stage: u32, source: vec2f) -> f32 {
    var coverage = adjustment_layer_coverage_point32(stage, source);
    if adjustment_layer_stages[stage].flags.y != 0u {
        coverage = 1.0 - coverage;
    }
    return coverage * adjustment_layer_value(stage, 1u, 3u);
}
