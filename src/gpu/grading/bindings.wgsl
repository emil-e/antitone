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
